//! Injected clock abstraction for deterministic timers, timeouts, and lifecycle cues.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use monoagens_protocol::Time;
use tokio::sync::Notify;

/// Boxed future returned by [`Clock::sleep_until`].
pub type SleepFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Injected time source used by the Gateway owner loop, timers, timeouts, and lifecycle cues.
pub trait Clock: Send + Sync {
    /// Returns the current timestamp in epoch milliseconds.
    fn now(&self) -> Time;

    /// Completes once [`Clock::now`] reaches or exceeds `deadline`.
    fn sleep_until(&self, deadline: Time) -> SleepFuture<'_>;
}

/// Wall-clock [`Clock`] backed by [`SystemTime`] and `tokio::time::sleep`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl SystemClock {
    /// Creates a new [`SystemClock`].
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Time {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        Time::from_millis(u64::try_from(ms).unwrap_or(u64::MAX))
    }

    fn sleep_until(&self, deadline: Time) -> SleepFuture<'_> {
        Box::pin(async move {
            let now = self.now();
            if now >= deadline {
                return;
            }
            let delta = deadline.saturating_sub_ms(now);
            tokio::time::sleep(Duration::from_millis(delta)).await;
        })
    }
}

/// Deterministic virtual [`Clock`] for offline tests without wall-clock sleeps.
#[derive(Debug, Clone)]
pub struct VirtualClock {
    now_ms: Arc<AtomicU64>,
    notify: Arc<Notify>,
}

impl VirtualClock {
    /// Creates a virtual clock initialized to `initial_time`.
    #[must_use]
    pub fn new(initial_time: Time) -> Self {
        Self {
            now_ms: Arc::new(AtomicU64::new(initial_time.as_millis())),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Advances the virtual clock by `delta_ms` milliseconds and wakes waiting timers.
    pub fn advance_by(&self, delta_ms: u64) -> Time {
        let prev = self.now_ms.fetch_add(delta_ms, Ordering::SeqCst);
        let next = prev.saturating_add(delta_ms);
        self.notify.notify_waiters();
        Time::from_millis(next)
    }

    /// Moves the virtual clock forward to at least `target` and wakes waiting timers.
    pub fn advance_to(&self, target: Time) -> Time {
        let target_ms = target.as_millis();
        let mut cur = self.now_ms.load(Ordering::SeqCst);
        while target_ms > cur {
            match self
                .now_ms
                .compare_exchange(cur, target_ms, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => {
                    self.notify.notify_waiters();
                    return target;
                }
                Err(actual) => cur = actual,
            }
        }
        Time::from_millis(cur)
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> Time {
        Time::from_millis(self.now_ms.load(Ordering::SeqCst))
    }

    fn sleep_until(&self, deadline: Time) -> SleepFuture<'_> {
        Box::pin(async move {
            let target = deadline.as_millis();
            loop {
                let notified = self.notify.notified();
                if self.now_ms.load(Ordering::SeqCst) >= target {
                    return;
                }
                notified.await;
            }
        })
    }
}

/// Typed timeout marker returned by [`run_with_clock_timeout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClockTimeout;

/// Races `fut` against `clock.sleep_until(now + timeout_ms)` deterministically.
pub async fn run_with_clock_timeout<F, T>(
    clock: &dyn Clock,
    timeout_ms: u64,
    fut: F,
) -> Result<T, ClockTimeout>
where
    F: Future<Output = T>,
{
    let deadline = clock.now().saturating_add_ms(timeout_ms);
    tokio::select! {
        biased;
        val = fut => Ok(val),
        _ = clock.sleep_until(deadline) => Err(ClockTimeout),
    }
}

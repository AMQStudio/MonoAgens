//! Honest `R-47` in-flight lifecycle cues (`working_cue` at `1 s`, `progress_note` at `10 s`).
//!
//! Fast paths emit zero cues and incur zero artificial delay (`d_art = 0 ms`).
//! Only genuine waits that cross `working_cue_ms` or `progress_note_ms` emit cues.

use std::sync::{Arc, Mutex};

use monoagens_protocol::{PlaceId, Time};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, mpsc};

use crate::clock::Clock;

/// Default threshold (`1_000 ms`) for emitting `working_cue` (`R-47`).
pub const DEFAULT_WORKING_CUE_MS: u64 = 1_000;

/// Default threshold (`10_000 ms`) for emitting `progress_note` (`R-47`).
pub const DEFAULT_PROGRESS_NOTE_MS: u64 = 10_000;

/// Honest in-flight lifecycle cue kind (`R-47`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleCueKind {
    WorkingCue,
    ProgressNote,
}

impl LifecycleCueKind {
    /// Canonical snake_case cue name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WorkingCue => "working_cue",
            Self::ProgressNote => "progress_note",
        }
    }
}

/// Honest lifecycle cue emitted for a genuine in-flight wait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleCueEvent {
    pub kind: LifecycleCueKind,
    pub wait_id: String,
    pub place: Option<PlaceId>,
    pub elapsed_ms: u64,
    pub emitted_at_ms: Time,
    pub detail: String,
}

/// Sink receiving honest lifecycle cues for observation by CLI/bodies/tests.
#[derive(Clone, Default)]
pub struct CueRecorder {
    events: Arc<Mutex<Vec<LifecycleCueEvent>>>,
    subscribers: Arc<Mutex<Vec<mpsc::UnboundedSender<LifecycleCueEvent>>>>,
}

impl std::fmt::Debug for CueRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CueRecorder")
            .field(
                "event_count",
                &self.events.lock().map(|e| e.len()).unwrap_or(0),
            )
            .finish()
    }
}

impl CueRecorder {
    /// Creates an empty [`CueRecorder`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribes to live [`LifecycleCueEvent`] notifications.
    #[must_use]
    pub fn subscribe(&self) -> mpsc::UnboundedReceiver<LifecycleCueEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.subscribers
            .lock()
            .expect("cue subscribers lock")
            .push(tx);
        rx
    }

    /// Emits a cue event to the recorder and any active subscribers.
    pub fn emit(&self, event: LifecycleCueEvent) {
        self.events
            .lock()
            .expect("cue events lock")
            .push(event.clone());
        let mut subs = self.subscribers.lock().expect("cue subscribers lock");
        subs.retain(|tx| tx.send(event.clone()).is_ok());
    }

    /// Returns all recorded cues in emission order.
    #[must_use]
    pub fn events(&self) -> Vec<LifecycleCueEvent> {
        self.events.lock().expect("cue events lock").clone()
    }
}

/// Cancellation handle for an in-flight honest wait monitor.
#[derive(Debug, Clone)]
pub struct WaitCueGuard {
    done: Arc<std::sync::atomic::AtomicBool>,
    notify: Arc<Notify>,
}

impl WaitCueGuard {
    /// Marks the wait as completed, cancelling any pending `working_cue`/`progress_note`.
    pub fn finish(&self) {
        self.done.store(true, std::sync::atomic::Ordering::SeqCst);
        self.notify.notify_waiters();
    }
}

impl Drop for WaitCueGuard {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Spawns an async cue monitor over `clock` that emits `working_cue` at `working_cue_ms`
/// and `progress_note` at `progress_note_ms` unless [`WaitCueGuard::finish`] is called first.
pub fn spawn_wait_cue_monitor(
    clock: Arc<dyn Clock>,
    recorder: CueRecorder,
    wait_id: impl Into<String>,
    place: Option<PlaceId>,
    working_cue_ms: u64,
    progress_note_ms: u64,
) -> WaitCueGuard {
    let wait_id = wait_id.into();
    let started_at = clock.now();
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let notify = Arc::new(Notify::new());

    let guard = WaitCueGuard {
        done: Arc::clone(&done),
        notify: Arc::clone(&notify),
    };

    tokio::spawn(async move {
        let working_deadline = started_at.saturating_add_ms(working_cue_ms);
        if !wait_until_or_cancelled(clock.as_ref(), working_deadline, &done, &notify).await {
            return;
        }
        let now = clock.now();
        recorder.emit(LifecycleCueEvent {
            kind: LifecycleCueKind::WorkingCue,
            wait_id: wait_id.clone(),
            place: place.clone(),
            elapsed_ms: now.saturating_sub_ms(started_at),
            emitted_at_ms: now,
            detail: "working on request".to_owned(),
        });

        let progress_deadline = started_at.saturating_add_ms(progress_note_ms);
        if !wait_until_or_cancelled(clock.as_ref(), progress_deadline, &done, &notify).await {
            return;
        }
        let now = clock.now();
        recorder.emit(LifecycleCueEvent {
            kind: LifecycleCueKind::ProgressNote,
            wait_id,
            place,
            elapsed_ms: now.saturating_sub_ms(started_at),
            emitted_at_ms: now,
            detail: "still working; waiting for completion".to_owned(),
        });
    });

    guard
}

async fn wait_until_or_cancelled(
    clock: &dyn Clock,
    deadline: Time,
    done: &std::sync::atomic::AtomicBool,
    notify: &Notify,
) -> bool {
    loop {
        let notified = notify.notified();
        if done.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        tokio::select! {
            biased;
            _ = notified => {
                if done.load(std::sync::atomic::Ordering::SeqCst) {
                    return false;
                }
            }
            _ = clock.sleep_until(deadline) => {
                return !done.load(std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
}

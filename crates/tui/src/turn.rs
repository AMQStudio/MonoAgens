//! Turn lifecycle bookkeeping: busy timing with an injectable clock, the post-turn grace window
//! for silent turns, and the queue of messages sent while a turn runs.
//!
//! Everything here is plain state driven by explicit calls (no tasks, no real sleeping) so it can be
//! tested with [`ManualClock`] and later sit behind a `GatewayClient` trait.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use crate::attachments::PendingAttachment;

/// How long after `TurnFinished` a reply may still arrive before the turn counts as silent.
pub const NO_REPLY_GRACE_MS: u64 = 1_500;

/// Notice shown under the user message when a turn ends without any reply.
pub const NO_REPLY_NOTICE: &str = "Mono chose not to reply to this message.";

/// Monotonic millisecond clock.
pub trait TurnClock: Send + Sync + std::fmt::Debug {
    /// Milliseconds since an arbitrary fixed origin.
    fn now_ms(&self) -> u64;
}

/// Real clock based on [`Instant`].
#[derive(Debug)]
pub struct SystemTurnClock {
    origin: Instant,
}

impl SystemTurnClock {
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemTurnClock {
    fn default() -> Self {
        Self::new()
    }
}

impl TurnClock for SystemTurnClock {
    fn now_ms(&self) -> u64 {
        self.origin.elapsed().as_millis() as u64
    }
}

/// Manually advanced clock for tests; clones share the same time.
#[derive(Debug, Clone, Default)]
pub struct ManualClock(Arc<AtomicU64>);

impl ManualClock {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Advances the shared time by `ms`.
    pub fn advance(&self, ms: u64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl TurnClock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A message typed while a turn was running.
#[derive(Debug, Clone)]
pub struct QueuedMessage {
    pub chat_id: String,
    pub place_id: String,
    /// Display text as it will appear in the timeline (attachment tokens included).
    pub text: String,
    pub attachments: Vec<PendingAttachment>,
}

/// Timing and queue state of the current turn.
#[derive(Debug, Clone)]
pub struct TurnState {
    clock: Arc<dyn TurnClock>,
    seq: u64,
    chat_id: Option<String>,
    started_ms: Option<u64>,
    finished_ms: Option<u64>,
    queue: VecDeque<QueuedMessage>,
    /// Chat of each recent turn (so a late reply can still be shown in its own chat).
    chats_by_seq: VecDeque<(u64, String)>,
}

/// How many past turns remember their chat.
const TURN_CHAT_HISTORY: usize = 64;

impl TurnState {
    #[must_use]
    pub fn new(clock: Arc<dyn TurnClock>) -> Self {
        Self {
            clock,
            seq: 0,
            chat_id: None,
            started_ms: None,
            finished_ms: None,
            queue: VecDeque::new(),
            chats_by_seq: VecDeque::new(),
        }
    }

    /// Replaces the clock (tests).
    pub fn set_clock(&mut self, clock: Arc<dyn TurnClock>) {
        self.clock = clock;
    }

    /// Current time on the turn clock (milliseconds).
    #[must_use]
    pub fn now_ms(&self) -> u64 {
        self.clock.now_ms()
    }

    /// Sequence number of the most recently started turn (0 before the first).
    #[must_use]
    pub const fn seq(&self) -> u64 {
        self.seq
    }

    /// Marks a new turn as started and returns its sequence number.
    pub fn begin(&mut self, chat_id: &str) -> u64 {
        self.seq += 1;
        self.chat_id = Some(chat_id.to_owned());
        self.started_ms = Some(self.clock.now_ms());
        self.finished_ms = None;
        if self.chats_by_seq.len() == TURN_CHAT_HISTORY {
            self.chats_by_seq.pop_front();
        }
        self.chats_by_seq.push_back((self.seq, chat_id.to_owned()));
        self.seq
    }

    /// Chat a (possibly finished) turn belongs to.
    #[must_use]
    pub fn chat_of(&self, seq: u64) -> Option<&str> {
        self.chats_by_seq
            .iter()
            .rev()
            .find(|(s, _)| *s == seq)
            .map(|(_, c)| c.as_str())
    }

    /// Clears running-turn timing.
    pub fn end(&mut self) {
        self.chat_id = None;
        self.started_ms = None;
        self.finished_ms = None;
    }

    /// Chat the running turn belongs to.
    #[must_use]
    pub fn chat_id(&self) -> Option<&str> {
        self.chat_id.as_deref()
    }

    /// Elapsed milliseconds of the running turn (0 when idle).
    #[must_use]
    pub fn elapsed_ms(&self) -> u64 {
        self.started_ms
            .map_or(0, |s| self.clock.now_ms().saturating_sub(s))
    }

    /// Records that the owner step of turn `seq` returned. Stale turn numbers are ignored.
    /// Returns `true` if the grace window was armed.
    pub fn mark_finished(&mut self, seq: u64) -> bool {
        if seq == self.seq && self.started_ms.is_some() {
            self.finished_ms = Some(self.clock.now_ms());
            true
        } else {
            false
        }
    }

    /// Starts the grace window over (a model call is still in flight, so the turn is not silent yet).
    pub fn rearm_grace(&mut self) {
        if self.finished_ms.is_some() {
            self.finished_ms = Some(self.clock.now_ms());
        }
    }

    /// True once the grace window after `TurnFinished` has fully elapsed.
    #[must_use]
    pub fn grace_expired(&self) -> bool {
        self.finished_ms
            .is_some_and(|f| self.clock.now_ms().saturating_sub(f) >= NO_REPLY_GRACE_MS)
    }

    /// Whether the given turn number is the running turn.
    #[must_use]
    pub fn is_current(&self, seq: u64) -> bool {
        seq == self.seq
    }

    /// Queued messages, oldest first.
    pub fn queued(&self) -> impl Iterator<Item = &QueuedMessage> {
        self.queue.iter()
    }

    /// Number of queued messages.
    #[must_use]
    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }

    pub fn push_queued(&mut self, msg: QueuedMessage) {
        self.queue.push_back(msg);
    }

    pub fn pop_front_queued(&mut self) -> Option<QueuedMessage> {
        self.queue.pop_front()
    }

    pub fn pop_back_queued(&mut self) -> Option<QueuedMessage> {
        self.queue.pop_back()
    }
}

impl Default for TurnState {
    fn default() -> Self {
        Self::new(Arc::new(SystemTurnClock::new()))
    }
}

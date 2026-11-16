//! Split-bubble `pid_1..pid_M` delivery, mid-stream barge-in cancellation, and `R-09` echo
//! and reply-to correlation (`delivered_units` accounting).

use core::fmt;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use monoagens_body_sdk::{
    ActionId, CapabilityId, HAct, Receipt, ReceiptOutcome, TerminationReason, Time,
};
use serde::{Deserialize, Serialize};

use crate::runtime::make_terminal_receipt;

const DEFAULT_ECHO_CAPACITY: usize = 4_096;

/// Errors returned while splitting chat messages for platform cap `L_cap`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitError {
    ZeroCapacity,
    CharExceedsCap { ch_bytes: usize, l_cap: usize },
}

impl fmt::Display for SplitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroCapacity => f.write_str("split-bubble L_cap must be >= 1"),
            Self::CharExceedsCap { ch_bytes, l_cap } => {
                write!(
                    f,
                    "single UTF-8 scalar ({ch_bytes} B) exceeds L_cap={l_cap}"
                )
            }
        }
    }
}

impl std::error::Error for SplitError {}

/// Splits `text` into chunks where every chunk satisfies `|c_m| <= l_cap` in both bytes and chars.
///
/// Prefers splitting on newlines or ASCII whitespace boundaries when available.
pub fn split_text_chunks(text: &str, l_cap: usize) -> Result<Vec<String>, SplitError> {
    if l_cap == 0 {
        return Err(SplitError::ZeroCapacity);
    }
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let rest = &text[start..];
        if rest.len() <= l_cap {
            chunks.push(rest.to_owned());
            break;
        }

        let mut hard_end = 0;
        let mut last_ws = None;
        for (idx, ch) in rest.char_indices() {
            let next_end = idx + ch.len_utf8();
            if next_end > l_cap {
                break;
            }
            hard_end = next_end;
            if ch == '\n' || ch == ' ' || ch == '\t' {
                last_ws = Some(next_end);
            }
        }

        if hard_end == 0 {
            let first_ch_len = rest.chars().next().map_or(0, char::len_utf8);
            return Err(SplitError::CharExceedsCap {
                ch_bytes: first_ch_len,
                l_cap,
            });
        }

        let split_at = match last_ws {
            Some(ws) if ws > 0 => ws,
            _ => hard_end,
        };
        let piece = rest[..split_at].trim_end_matches([' ', '\t']);
        if !piece.is_empty() {
            chunks.push(piece.to_owned());
        } else {
            chunks.push(rest[..hard_end].to_owned());
            start += hard_end;
            continue;
        }
        start += split_at;
        while start < text.len() && text.as_bytes()[start] == b' ' {
            start += 1;
        }
    }

    Ok(chunks)
}

/// Recorded outbound platform message ID for `R-09` self-echo and reply-to matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EchoRecord {
    pub pid: String,
    pub action_id: ActionId,
    pub chunk_index: usize,
    pub timestamp_ms: Time,
}

/// Bounded registry correlating emitted platform IDs (`pid_1..pid_M`) back to Core actions.
#[derive(Debug, Clone)]
pub struct EchoRegistry {
    capacity: usize,
    order: VecDeque<String>,
    by_pid: HashMap<String, EchoRecord>,
}

impl Default for EchoRegistry {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_ECHO_CAPACITY)
    }
}

impl EchoRegistry {
    /// Creates an [`EchoRegistry`] holding up to `capacity` recent platform message IDs.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            by_pid: HashMap::new(),
        }
    }

    /// Registers a delivered platform message ID `pid` for `action_id` and `chunk_index`.
    pub fn register_echo(
        &mut self,
        pid: impl Into<String>,
        action_id: ActionId,
        chunk_index: usize,
        timestamp_ms: Time,
    ) {
        let pid = pid.into();
        if !self.by_pid.contains_key(&pid) {
            if self.order.len() >= self.capacity
                && let Some(evicted) = self.order.pop_front()
            {
                self.by_pid.remove(&evicted);
            }
            self.order.push_back(pid.clone());
        }
        self.by_pid.insert(
            pid.clone(),
            EchoRecord {
                pid,
                action_id,
                chunk_index,
                timestamp_ms,
            },
        );
    }

    /// Returns `true` if `pid` corresponds to one of this body's own delivered bubbles.
    #[must_use]
    pub fn is_self_echo(&self, pid: &str) -> bool {
        self.by_pid.contains_key(pid)
    }

    /// Resolves a platform `reply_to_pid` back to its originating [`ActionId`] and chunk record.
    #[must_use]
    pub fn resolve_reply_to(&self, reply_to_pid: &str) -> Option<&EchoRecord> {
        self.by_pid.get(reply_to_pid)
    }

    /// Returns the number of tracked platform message IDs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_pid.len()
    }

    /// Returns `true` when no platform IDs are currently tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_pid.is_empty()
    }
}

const BARGE_NONE: u8 = 0;
const BARGE_PREEMPTED_EXTERNAL: u8 = 1;
const BARGE_CANCELED_BY_CORE: u8 = 2;

/// Thread-safe mid-stream preemption flag shared between input listeners and bubble delivery.
#[derive(Debug, Clone, Default)]
pub struct BargeInToken {
    triggered: Arc<AtomicBool>,
    kind: Arc<AtomicU8>,
}

impl BargeInToken {
    /// Creates an untriggered [`BargeInToken`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Triggers preemption due to external user activity (`preempted_external`).
    pub fn trigger_external(&self) {
        let _ = self.kind.compare_exchange(
            BARGE_NONE,
            BARGE_PREEMPTED_EXTERNAL,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        self.triggered.store(true, Ordering::SeqCst);
    }

    /// Triggers cancellation due to Core `action/cancel` (`canceled_by_core`).
    pub fn trigger_core_cancel(&self) {
        let _ = self.kind.compare_exchange(
            BARGE_NONE,
            BARGE_CANCELED_BY_CORE,
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        self.triggered.store(true, Ordering::SeqCst);
    }

    /// Returns `true` if barge-in or Core cancellation has been signalled.
    #[must_use]
    pub fn is_triggered(&self) -> bool {
        self.triggered.load(Ordering::SeqCst)
    }

    /// Returns the [`TerminationReason`] corresponding to the cancellation trigger.
    #[must_use]
    pub fn termination_reason(&self) -> TerminationReason {
        match self.kind.load(Ordering::SeqCst) {
            BARGE_CANCELED_BY_CORE => TerminationReason::CanceledByCore,
            BARGE_PREEMPTED_EXTERNAL => TerminationReason::PreemptedExternal,
            _ => TerminationReason::Normal,
        }
    }
}

/// Platform transport sink sending one bubble chunk and returning its platform message ID (`pid_m`).
pub trait BubbleSender {
    fn send_bubble(&mut self, chunk_index: usize, chunk_text: &str) -> Result<String, String>;
}

/// Result of split-bubble delivery including delivered `pid_1..pid_k` and single combined receipt.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitBubbleOutcome {
    pub total_chunks: usize,
    pub delivered_pids: Vec<String>,
    pub receipt: Receipt,
}

/// Delivers `text` split into chunks `<= l_cap`, halting unsent chunks immediately on `barge_in`,
/// registering every delivered `pid` in `echo_registry`, and returning ONE combined `Receipt`.
#[allow(clippy::too_many_arguments)]
pub fn deliver_split_bubbles(
    action_id: ActionId,
    capability_id: CapabilityId,
    text: &str,
    l_cap: usize,
    sender: &mut dyn BubbleSender,
    barge_in: &BargeInToken,
    echo_registry: &mut EchoRegistry,
    h_act: Option<HAct>,
    timestamp_ms: Time,
) -> Result<SplitBubbleOutcome, SplitError> {
    let chunks = split_text_chunks(text, l_cap)?;
    let total_chunks = chunks.len();
    let mut delivered_pids = Vec::with_capacity(total_chunks);

    for (idx, chunk) in chunks.iter().enumerate() {
        if barge_in.is_triggered() {
            let delivered = delivered_pids.len() as u64;
            let progress = if total_chunks == 0 {
                0.0
            } else {
                delivered as f32 / total_chunks as f32
            };
            let reason = barge_in.termination_reason();
            let reason_code = match reason {
                TerminationReason::CanceledByCore => "canceled_by_core",
                TerminationReason::PreemptedExternal => "preempted_external",
                _ => "preempted_external",
            };
            let receipt = make_terminal_receipt(
                action_id,
                capability_id,
                ReceiptOutcome::Failed,
                Some(reason_code.to_owned()),
                Some(format!(
                    "halted after {delivered}/{total_chunks} bubbles due to {reason_code}"
                )),
                Some(reason),
                Some(progress),
                Some(delivered),
                h_act,
                timestamp_ms,
            );
            return Ok(SplitBubbleOutcome {
                total_chunks,
                delivered_pids,
                receipt,
            });
        }

        match sender.send_bubble(idx, chunk) {
            Ok(pid) => {
                echo_registry.register_echo(pid.clone(), action_id.clone(), idx, timestamp_ms);
                delivered_pids.push(pid);
            }
            Err(err_detail) => {
                let delivered = delivered_pids.len() as u64;
                let progress = if total_chunks == 0 {
                    0.0
                } else {
                    delivered as f32 / total_chunks as f32
                };
                let receipt = make_terminal_receipt(
                    action_id,
                    capability_id,
                    ReceiptOutcome::Failed,
                    Some("bubble_send_failed".to_owned()),
                    Some(err_detail),
                    Some(TerminationReason::Normal),
                    Some(progress),
                    Some(delivered),
                    h_act,
                    timestamp_ms,
                );
                return Ok(SplitBubbleOutcome {
                    total_chunks,
                    delivered_pids,
                    receipt,
                });
            }
        }
    }

    let delivered = delivered_pids.len() as u64;
    let receipt = make_terminal_receipt(
        action_id,
        capability_id,
        ReceiptOutcome::Done,
        None,
        Some(if delivered_pids.is_empty() {
            "delivered 0 bubbles".to_owned()
        } else {
            format!("delivered pids: {}", delivered_pids.join(","))
        }),
        Some(TerminationReason::Normal),
        Some(1.0),
        Some(delivered),
        h_act,
        timestamp_ms,
    );

    Ok(SplitBubbleOutcome {
        total_chunks,
        delivered_pids,
        receipt,
    })
}

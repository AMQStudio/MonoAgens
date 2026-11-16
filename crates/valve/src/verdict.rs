//! Deterministic `door_verdict` and config-toggle audit log records.
//!
//! Records contain canonical rule names and [`ValveReasonTag`]s without exposing
//! internal Cedar policy ASTs or formula constants.

use monoagens_core::{DoorDecision, LogSeq};
use monoagens_protocol::{ActionId, EventId, Time};
use serde::{Deserialize, Serialize};

use crate::reason_tags::ValveReasonTag;

/// All four Gateway doors governed by `Valve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValveDoorKind {
    Inbound,
    Context,
    Action,
    MemoryWrite,
}

impl ValveDoorKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inbound => "inbound",
            Self::Context => "context",
            Self::Action => "action",
            Self::MemoryWrite => "memory_write",
        }
    }
}

/// Logged record of one `Valve` door decision or configuration toggle change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValveVerdictRecord {
    pub log_seq: LogSeq,
    pub t_now: Time,
    pub door: Option<ValveDoorKind>,
    pub rule: String,
    pub reason_tag: Option<ValveReasonTag>,
    pub outcome: String,
    pub target_event_id: Option<EventId>,
    pub target_action_id: Option<ActionId>,
    pub stripped_ids: Vec<EventId>,
    pub detail: String,
}

/// Append-only in-memory audit log of `Valve` verdicts and config changes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValveAuditLog {
    entries: Vec<ValveVerdictRecord>,
}

impl ValveAuditLog {
    /// Creates an empty audit log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a pre-built door verdict record.
    pub fn record(&mut self, record: ValveVerdictRecord) {
        self.entries.push(record);
    }

    /// Builds and appends a door verdict record from a [`DoorDecision`].
    pub fn record_door_decision(
        &mut self,
        log_seq: LogSeq,
        t_now: Time,
        door: ValveDoorKind,
        rule: impl Into<String>,
        reason_tag: Option<ValveReasonTag>,
        decision: &DoorDecision,
    ) -> &mut ValveVerdictRecord {
        let (outcome, stripped_ids) = match decision {
            DoorDecision::Allow => ("allow".to_owned(), Vec::new()),
            DoorDecision::Strip { removed_ids, .. } => ("strip".to_owned(), removed_ids.clone()),
            DoorDecision::Hold { .. } => ("hold".to_owned(), Vec::new()),
            DoorDecision::Refuse { .. } => ("refuse".to_owned(), Vec::new()),
        };
        self.entries.push(ValveVerdictRecord {
            log_seq,
            t_now,
            door: Some(door),
            rule: rule.into(),
            reason_tag,
            outcome,
            target_event_id: None,
            target_action_id: None,
            stripped_ids,
            detail: String::new(),
        });
        let last = self.entries.len() - 1;
        &mut self.entries[last]
    }

    /// Appends a configuration change record when a rule toggle or mode shifts.
    pub fn record_config_change(
        &mut self,
        log_seq: LogSeq,
        t_now: Time,
        rule: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.entries.push(ValveVerdictRecord {
            log_seq,
            t_now,
            door: None,
            rule: rule.into(),
            reason_tag: None,
            outcome: "config_changed".to_owned(),
            target_event_id: None,
            target_action_id: None,
            stripped_ids: Vec::new(),
            detail: detail.into(),
        });
    }

    /// Returns all recorded entries in chronological order.
    #[must_use]
    pub fn entries(&self) -> &[ValveVerdictRecord] {
        &self.entries
    }

    /// Clears recorded entries.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

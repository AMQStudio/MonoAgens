//! Recipient-local DST-safe quiet hours (`22:00..07:00`), proactive daily cap (`C_daily = 8`),
//! emergency urgency overrides (`U_quiet / U_cap = 0.90`), and `3x/day` digest batching (`R-46`).

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use monoagens_core::{EventDetail, EventKind, EventRecord, ExpectationLifecycleState};
use monoagens_protocol::{
    ActionId, ActorId, CapabilityId, EventId, OpaquePayload, PlaceId, ReceiptOutcome, Time,
};
use serde::{Deserialize, Serialize};

use crate::config::ValveConfig;
use crate::reason_tags::ValveReasonTag;

/// Default recipient-local quiet-hours start (`22:00`).
pub const QUIET_START_HOUR: u8 = 22;
/// Default recipient-local quiet-hours end (`07:00`).
pub const QUIET_END_HOUR: u8 = 7;
/// Default proactive notification daily cap per recipient (`8/day`).
pub const PROACTIVE_DAILY_CAP: u32 = 8;
/// Urgency cutoff bypassing quiet hours and daily proactive cap (`0.90`).
pub const EMERGENCY_URGENCY_CUTOFF: f64 = 0.90;
/// Daytime immediate proactive delivery urgency cutoff (`0.60`).
pub const IMMEDIATE_URGENCY_CUTOFF: f64 = 0.60;
/// Three recipient-local digest delivery hours outside `22:00..07:00` (`08:00`, `13:00`, `19:00`).
pub const DIGEST_HOURS_LOCAL: [u8; 3] = [8, 13, 19];

/// Errors evaluating recipient-local timezone or DST transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuietHoursError {
    InvalidTimestamp(String),
    InvalidTimezone { tz: String, detail: String },
}

impl fmt::Display for QuietHoursError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTimestamp(e) => write!(f, "invalid timestamp: {e}"),
            Self::InvalidTimezone { tz, detail } => write!(f, "invalid timezone '{tz}': {detail}"),
        }
    }
}

impl std::error::Error for QuietHoursError {}

/// Recipient-local date key `(year, month, day)` in the recipient's IANA timezone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LocalDateKey {
    pub year: i16,
    pub month: i8,
    pub day: i8,
}

/// Resolved recipient-local wall-clock view for DST-safe quiet-hours evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalTimeView {
    pub date: LocalDateKey,
    pub hour: u8,
    pub minute: u8,
    pub timezone: String,
    pub in_quiet_hours: bool,
}

/// Evaluates `t_now` in the recipient's IANA `tz_name` using `jiff` (never a fixed offset).
pub fn evaluate_recipient_local_time(
    t_now: Time,
    tz_name: &str,
    q_start_hour: u8,
    q_end_hour: u8,
) -> Result<LocalTimeView, QuietHoursError> {
    let tz = TimeZone::get(tz_name).map_err(|e| QuietHoursError::InvalidTimezone {
        tz: tz_name.to_owned(),
        detail: e.to_string(),
    })?;
    let ms = i64::try_from(t_now.as_millis())
        .map_err(|e| QuietHoursError::InvalidTimestamp(e.to_string()))?;
    let ts = Timestamp::from_millisecond(ms)
        .map_err(|e| QuietHoursError::InvalidTimestamp(e.to_string()))?;
    let zoned = ts.to_zoned(tz);
    let hour = u8::try_from(zoned.hour()).unwrap_or(0);
    let minute = u8::try_from(zoned.minute()).unwrap_or(0);
    let in_quiet_hours = is_hour_in_quiet_window(hour, q_start_hour, q_end_hour);

    Ok(LocalTimeView {
        date: LocalDateKey {
            year: zoned.year(),
            month: zoned.month(),
            day: zoned.day(),
        },
        hour,
        minute,
        timezone: tz_name.to_owned(),
        in_quiet_hours,
    })
}

/// Evaluates whether local `hour` (`0..=23`) falls within `[q_start, q_end)` (wrapping midnight).
#[must_use]
pub const fn is_hour_in_quiet_window(hour: u8, q_start_hour: u8, q_end_hour: u8) -> bool {
    if q_start_hour == q_end_hour {
        return false;
    }
    if q_start_hour > q_end_hour {
        // Overnight window e.g. 22:00..07:00
        hour >= q_start_hour || hour < q_end_hour
    } else {
        hour >= q_start_hour && hour < q_end_hour
    }
}

/// Computes the next recipient-local `3x/day` digest slot timestamp strictly after `t_now`.
pub fn next_digest_slot_ms(
    t_now: Time,
    tz_name: &str,
    digest_hours_local: [u8; 3],
) -> Result<Time, QuietHoursError> {
    let tz = TimeZone::get(tz_name).map_err(|e| QuietHoursError::InvalidTimezone {
        tz: tz_name.to_owned(),
        detail: e.to_string(),
    })?;
    let ms = i64::try_from(t_now.as_millis())
        .map_err(|e| QuietHoursError::InvalidTimestamp(e.to_string()))?;
    let ts = Timestamp::from_millisecond(ms)
        .map_err(|e| QuietHoursError::InvalidTimestamp(e.to_string()))?;
    let zoned = ts.to_zoned(tz.clone());
    let mut day = zoned.date();

    for day_offset in 0..=1 {
        if day_offset == 1 {
            day = day
                .tomorrow()
                .map_err(|e| QuietHoursError::InvalidTimestamp(e.to_string()))?;
        }
        for &slot_hour in &digest_hours_local {
            let slot_dt = day.at(i8::try_from(slot_hour).unwrap_or(8), 0, 0, 0);
            let slot_zoned = slot_dt
                .to_zoned(tz.clone())
                .map_err(|e| QuietHoursError::InvalidTimestamp(e.to_string()))?;
            let slot_ms = slot_zoned.timestamp().as_millisecond();
            if slot_ms > ms {
                return u64::try_from(slot_ms)
                    .map(Time::from_millis)
                    .map_err(|e| QuietHoursError::InvalidTimestamp(e.to_string()));
            }
        }
    }

    Err(QuietHoursError::InvalidTimestamp(
        "failed to resolve next digest slot".to_owned(),
    ))
}

/// Single proactive message deferred into a recipient's `3x/day` digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestItem {
    pub action_id: ActionId,
    pub recipient: ActorId,
    pub place: PlaceId,
    pub capability_id: CapabilityId,
    pub payload: OpaquePayload,
    pub queued_at_ms: Time,
    pub scheduled_at_ms: Time,
    pub reason: ValveReasonTag,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expectation_id: Option<EventId>,
}

/// Append-only event-log payload persisted inside `EventKind::DoorVerdict` for `DigestQueue` replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "digest_event", rename_all = "snake_case")]
pub enum DigestLogRecord {
    Queued { item: DigestItem },
    Cancelled { place: PlaceId, key: String },
    Drained { action_ids: Vec<ActionId> },
}

/// Batched digest delivered at one of the `3x/day` local digest slots.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestBatch {
    pub recipient: ActorId,
    pub scheduled_at_ms: Time,
    pub items: Vec<DigestItem>,
}

/// Deduplicated, event-sourced `3x/day` digest queue with replannable reconsider-before-send
/// cancellation (`arXiv:2610.05159`) ensuring stale or duplicate messages never fire.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestQueue {
    pending: Vec<DigestItem>,
    seen_or_dispatched: BTreeSet<ActionId>,
    #[serde(default)]
    cancelled_actions: BTreeSet<ActionId>,
    #[serde(default)]
    superseded_topics: BTreeMap<PlaceId, BTreeSet<String>>,
    #[serde(default)]
    superseded_expectations: BTreeSet<EventId>,
    #[serde(default)]
    superseded_dropped_total: u64,
}

impl DigestQueue {
    /// Creates an empty [`DigestQueue`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueues `item` once; returns `false` if `item.action_id` was already queued,
    /// dispatched, or cancelled/superseded.
    pub fn enqueue_once(&mut self, item: DigestItem) -> bool {
        if self.is_item_superseded(&item) {
            return false;
        }
        if !self.seen_or_dispatched.insert(item.action_id.clone()) {
            return false;
        }
        self.pending.push(item);
        true
    }

    fn is_item_superseded(&self, item: &DigestItem) -> bool {
        if self.cancelled_actions.contains(&item.action_id) {
            return true;
        }
        if let Some(eid) = item.expectation_id
            && self.superseded_expectations.contains(&eid)
        {
            return true;
        }
        if let Some(topic) = &item.topic
            && self
                .superseded_topics
                .get(&item.place)
                .is_some_and(|keys| keys.contains(topic))
        {
            return true;
        }
        self.superseded_topics
            .get(&item.place)
            .is_some_and(|keys| {
                keys.contains(item.action_id.as_str())
                    || item
                        .expectation_id
                        .is_some_and(|eid| keys.contains(&eid.to_string()))
            })
    }

    /// Marks queued items matching `(place, topic_or_expectation_id)` as superseded/cancelled
    /// so they are dropped before dispatch (`arXiv:2610.05159`). Returns the number cancelled.
    pub fn cancel_matching(&mut self, place: &PlaceId, topic_or_expectation_id: &str) -> usize {
        self.superseded_topics
            .entry(place.clone())
            .or_default()
            .insert(topic_or_expectation_id.to_owned());
        if let Ok(raw_id) = topic_or_expectation_id.parse::<u64>() {
            self.superseded_expectations.insert(EventId::new(raw_id));
        }

        let mut cancelled = 0usize;
        let mut kept = Vec::with_capacity(self.pending.len());
        for item in self.pending.drain(..) {
            let matches_place = &item.place == place;
            let matches_key = item.topic.as_deref() == Some(topic_or_expectation_id)
                || item
                    .expectation_id
                    .is_some_and(|eid| eid.to_string() == topic_or_expectation_id)
                || item.action_id.as_str() == topic_or_expectation_id;
            if matches_place && matches_key {
                self.cancelled_actions.insert(item.action_id.clone());
                self.seen_or_dispatched.insert(item.action_id);
                cancelled += 1;
            } else {
                kept.push(item);
            }
        }
        self.pending = kept;
        self.superseded_dropped_total = self
            .superseded_dropped_total
            .saturating_add(u64::try_from(cancelled).unwrap_or(u64::MAX));
        cancelled
    }

    /// Cancels any queued digest items tied to `expectation_id` in `place`.
    pub fn cancel_for_expectation(&mut self, place: &PlaceId, expectation_id: EventId) -> usize {
        self.superseded_expectations.insert(expectation_id);
        self.cancel_matching(place, &expectation_id.to_string())
    }

    /// Replays [`DigestQueue`] state deterministically from append-only [`EventRecord`]s across restarts.
    pub fn replay_from_events(&mut self, events: &[EventRecord]) {
        *self = Self::new();
        for ev in events {
            match ev.kind {
                EventKind::DoorVerdict => {
                    if let Ok(rec) = serde_json::from_slice::<DigestLogRecord>(ev.payload.as_bytes())
                    {
                        match rec {
                            DigestLogRecord::Queued { item } => {
                                let _ = self.enqueue_once(item);
                            }
                            DigestLogRecord::Cancelled { place, key } => {
                                let _ = self.cancel_matching(&place, &key);
                            }
                            DigestLogRecord::Drained { action_ids } => {
                                for aid in action_ids {
                                    self.seen_or_dispatched.insert(aid.clone());
                                    self.pending.retain(|item| item.action_id != aid);
                                }
                            }
                        }
                    }
                }
                EventKind::ExpectationChanged => match &ev.detail {
                    Some(EventDetail::ExpectationTransition {
                        expectation_id,
                        action_id,
                        to_state:
                            ExpectationLifecycleState::Kept
                            | ExpectationLifecycleState::KeptLate
                            | ExpectationLifecycleState::Cancelled
                            | ExpectationLifecycleState::Released
                            | ExpectationLifecycleState::Answered
                            | ExpectationLifecycleState::Reached
                            | ExpectationLifecycleState::Rejected
                            | ExpectationLifecycleState::Expired,
                        ..
                    }) => {
                        let _ = self.cancel_for_expectation(&ev.place, *expectation_id);
                        if let Some(aid) = action_id {
                            let _ = self.cancel_matching(&ev.place, aid.as_str());
                        }
                    }
                    Some(EventDetail::ExpectationRevised { expectation_id, .. }) => {
                        let _ = self.cancel_for_expectation(&ev.place, *expectation_id);
                    }
                    _ => {}
                },
                EventKind::Action => {
                    if let Some(EventDetail::Action { action_id, .. }) = &ev.detail {
                        self.seen_or_dispatched.insert(action_id.clone());
                        self.pending.retain(|item| &item.action_id != action_id);
                    }
                }
                EventKind::Receipt => {
                    if let Some(EventDetail::Receipt {
                        action_id, outcome, ..
                    }) = &ev.detail
                        && *outcome != ReceiptOutcome::Held
                    {
                        self.seen_or_dispatched.insert(action_id.clone());
                        self.pending.retain(|item| &item.action_id != action_id);
                    }
                }
                _ => {}
            }
        }
    }

    /// Returns total queued items suppressed by reconsider-before-send cancellation.
    #[must_use]
    pub const fn superseded_dropped_total(&self) -> u64 {
        self.superseded_dropped_total
    }

    /// Returns the number of currently queued digest items.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Returns `true` if empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Returns all currently pending digest items without draining them.
    #[must_use]
    pub fn pending_items(&self) -> &[DigestItem] {
        &self.pending
    }

    /// Drains all due digest items (`scheduled_at_ms <= t_now`), dropping any superseded items.
    pub fn drain_due(&mut self, t_now: Time) -> Vec<DigestBatch> {
        self.drain_due_reconsidered(t_now, |_| false)
    }

    /// Pre-drain reconsider-before-send hook: evaluates `is_superseded(&item)` on every due item
    /// before dispatch, dropping stale/cancelled entries instead of delivering them.
    pub fn drain_due_reconsidered<F>(&mut self, t_now: Time, mut is_superseded: F) -> Vec<DigestBatch>
    where
        F: FnMut(&DigestItem) -> bool,
    {
        let mut due_groups: BTreeMap<(ActorId, Time), Vec<DigestItem>> = BTreeMap::new();
        let pending = std::mem::take(&mut self.pending);
        let mut remaining = Vec::with_capacity(pending.len());
        let mut dropped_now = 0u64;

        for item in pending {
            if self.is_item_superseded(&item) || is_superseded(&item) {
                self.cancelled_actions.insert(item.action_id.clone());
                self.seen_or_dispatched.insert(item.action_id);
                dropped_now = dropped_now.saturating_add(1);
                continue;
            }
            if item.scheduled_at_ms <= t_now {
                due_groups
                    .entry((item.recipient.clone(), item.scheduled_at_ms))
                    .or_default()
                    .push(item);
            } else {
                remaining.push(item);
            }
        }
        self.pending = remaining;
        self.superseded_dropped_total = self.superseded_dropped_total.saturating_add(dropped_now);

        due_groups
            .into_iter()
            .map(|((recipient, scheduled_at_ms), items)| DigestBatch {
                recipient,
                scheduled_at_ms,
                items,
            })
            .collect()
    }
}

/// Per-recipient local-day counter tracking proactive notifications sent against `C_daily`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProactiveCapTracker {
    counts: BTreeMap<(ActorId, LocalDateKey), u32>,
}

impl ProactiveCapTracker {
    /// Returns proactive count sent on `date` to `recipient`.
    #[must_use]
    pub fn count_for(&self, recipient: &ActorId, date: LocalDateKey) -> u32 {
        self.counts
            .get(&(recipient.clone(), date))
            .copied()
            .unwrap_or(0)
    }

    /// Increments proactive count on `date` for `recipient`.
    pub fn record_sent(&mut self, recipient: &ActorId, date: LocalDateKey) {
        let entry = self.counts.entry((recipient.clone(), date)).or_insert(0);
        *entry = entry.saturating_add(1);
    }
}

/// Outcome of proactive notification gate evaluation (`quiet_hours`, `C_daily`, emergency cutoff).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProactiveEvaluation {
    AllowImmediate {
        emergency_override: bool,
        local_date: LocalDateKey,
    },
    HoldDigest {
        reason: ValveReasonTag,
        scheduled_at_ms: Time,
    },
    RefuseInvalidTz,
}

/// Evaluates recipient-local quiet hours, `C_daily` proactive cap, emergency cutoff (`>= 0.90`),
/// and `3x/day` digest scheduling.
pub fn evaluate_proactive_notification(
    recipient: &ActorId,
    tz_name: &str,
    t_now: Time,
    urgency: Option<f64>,
    tracker: &ProactiveCapTracker,
    snapshot_sent_today: u32,
    config: &ValveConfig,
) -> ProactiveEvaluation {
    let Ok(local) = evaluate_recipient_local_time(
        t_now,
        tz_name,
        config.quiet_start_hour,
        config.quiet_end_hour,
    ) else {
        return ProactiveEvaluation::RefuseInvalidTz;
    };

    let emergency_override =
        urgency.is_some_and(|u| u.is_finite() && u >= config.emergency_urgency_cutoff);
    if emergency_override {
        return ProactiveEvaluation::AllowImmediate {
            emergency_override: true,
            local_date: local.date,
        };
    }

    let sent_today = tracker
        .count_for(recipient, local.date)
        .max(snapshot_sent_today);
    if config.proactive_cap_enabled && sent_today >= config.proactive_daily_cap {
        let scheduled_at_ms =
            next_digest_slot_ms(t_now, tz_name, config.digest_hours_local).unwrap_or(t_now);
        return ProactiveEvaluation::HoldDigest {
            reason: ValveReasonTag::DailyCapReached,
            scheduled_at_ms,
        };
    }

    if config.quiet_hours_enabled && local.in_quiet_hours {
        let scheduled_at_ms =
            next_digest_slot_ms(t_now, tz_name, config.digest_hours_local).unwrap_or(t_now);
        return ProactiveEvaluation::HoldDigest {
            reason: ValveReasonTag::QuietHoursDigest,
            scheduled_at_ms,
        };
    }

    if config.quiet_hours_enabled
        && let Some(u) = urgency
        && u.is_finite()
        && u < config.immediate_urgency_cutoff
    {
        let scheduled_at_ms =
            next_digest_slot_ms(t_now, tz_name, config.digest_hours_local).unwrap_or(t_now);
        return ProactiveEvaluation::HoldDigest {
            reason: ValveReasonTag::QuietHoursDigest,
            scheduled_at_ms,
        };
    }

    ProactiveEvaluation::AllowImmediate {
        emergency_override: false,
        local_date: local.date,
    }
}

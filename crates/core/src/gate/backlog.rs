//! Per-place backlog chunking (`R-14`, `A-04`):
//!
//! Triggers ONLY when an event's ingest lag exceeds `patience_place` OR when Core was genuinely
//! `stopped/down` — NEVER during background async sleep (`"or asleep"` deleted permanently).
//! Combines events per place using event-level age-discounted `max` (`(1 + L_hours)^(-d)`)
//! and appraises at most `backlog_max_chunks` (4) chunks per wake in salience order, keeping
//! all underlying events in the log.

use std::collections::BTreeMap;

use monoagens_protocol::{EventId, PlaceId, Time};
use serde::{Deserialize, Serialize};

use crate::activation::MS_PER_HOUR;
use crate::gate::decide::CommitmentInputs;
use crate::gate::salience::compute_salience;
use crate::gate::signals::GateSignals;
use crate::prior::Prior;
use crate::registry::Switches;

/// Default maximum place chunks handled in one wake (`R-14`, Cowan 2001).
pub const MAX_CHUNKS_PER_WAKE: usize = 4;

/// Single backlogged event's evaluated signals and commitment flags before chunking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BacklogEventInput {
    pub event_id: EventId,
    pub place: PlaceId,
    pub timestamp_ms: Time,
    pub signals: GateSignals,
    pub commitments: CommitmentInputs,
}

/// Aggregated per-place backlog chunk produced by [`chunk_place_backlog`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceBacklogChunk {
    pub place: PlaceId,
    /// Representative event id (most salient or latest event in the chunk) used for the seeded draw.
    pub representative_event_id: EventId,
    /// All event ids folded into this chunk (all remain preserved in the log).
    pub event_ids: Vec<EventId>,
    /// Age-discounted event-level max signals across the chunk.
    pub signals: GateSignals,
    /// Combined commitment flags (`any` event in the chunk engaging a commitment fires the breaker).
    pub commitments: CommitmentInputs,
    /// Clamped salience `S` computed from the aggregated chunk signals.
    pub chunk_salience: f64,
}

/// Output of [`schedule_backlog_chunks`]: up to `backlog_max_chunks` (4) handled now,
/// with any remaining place chunks deferred to subsequent wakes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BacklogScheduleResult {
    pub handle_now: Vec<PlaceBacklogChunk>,
    pub deferred: Vec<PlaceBacklogChunk>,
}

/// Classifies whether an event belongs to a backlog (`A-04`, `R-14`).
///
/// Returns `true` ONLY when `now_ms - event_timestamp_ms > patience_place_ms` OR
/// `core_was_stopped_down == true`. Background sleep consolidation NEVER triggers backlog
/// chunking (`Delta theta_sleep = 0`, live events appraise immediately).
#[must_use]
pub fn is_backlog_event(
    event_timestamp_ms: Time,
    now_ms: Time,
    patience_place_ms: u64,
    core_was_stopped_down: bool,
) -> bool {
    if core_was_stopped_down {
        return true;
    }
    now_ms.saturating_sub_ms(event_timestamp_ms) > patience_place_ms
}

/// Combines all backlogged events for a single place into one [`PlaceBacklogChunk`] using
/// event-level age-discounted `max`: `s_k = max_i (s_k(e_i) * (1 + L_i)^(-d))`.
///
/// Returns `None` if `events` is empty.
#[must_use]
pub fn chunk_place_backlog(
    place: PlaceId,
    events: &[BacklogEventInput],
    now_ms: Time,
    prior: &Prior,
    switches: &Switches,
) -> Option<PlaceBacklogChunk> {
    if events.is_empty() {
        return None;
    }

    let d = prior.petrov_d();
    let mut agg = GateSignals::default();
    let mut commitments = CommitmentInputs::default();
    let mut event_ids = Vec::with_capacity(events.len());
    let mut best_score = f64::NEG_INFINITY;
    let mut representative_event_id = events[0].event_id;

    for ev in events {
        event_ids.push(ev.event_id);
        let l_hours = (now_ms.saturating_sub_ms(ev.timestamp_ms) as f64) / MS_PER_HOUR;
        let discount = (1.0 + l_hours).powf(-d);

        agg.own_name = agg
            .own_name
            .max((ev.signals.own_name * discount).clamp(0.0, 1.0));
        agg.dialogue_act_gap = agg
            .dialogue_act_gap
            .max((ev.signals.dialogue_act_gap * discount).clamp(0.0, 1.0));
        agg.asr_clarification_boost = agg
            .asr_clarification_boost
            .max((ev.signals.asr_clarification_boost * discount).clamp(0.0, 1.0));
        agg.info_gap = agg
            .info_gap
            .max((ev.signals.info_gap * discount).clamp(0.0, 1.0));
        agg.own_info_gap = agg
            .own_info_gap
            .max((ev.signals.own_info_gap * discount).clamp(0.0, 1.0));
        agg.feeling_of_knowing = agg
            .feeling_of_knowing
            .max((ev.signals.feeling_of_knowing * discount).clamp(0.0, 1.0));
        agg.relevance = agg
            .relevance
            .max((ev.signals.relevance * discount).clamp(0.0, 1.0));
        agg.commitment = agg
            .commitment
            .max((ev.signals.commitment * discount).clamp(0.0, 1.0));
        agg.deg_soul = agg
            .deg_soul
            .max((ev.signals.deg_soul * discount).clamp(0.0, 1.0));
        agg.max_deg_other = agg
            .max_deg_other
            .max((ev.signals.max_deg_other * discount).clamp(0.0, 1.0));
        agg.answered = agg
            .answered
            .max((ev.signals.answered * discount).clamp(0.0, 1.0));
        agg.closeness_sender = agg.closeness_sender.max(ev.signals.closeness_sender);
        agg.social_cost = agg
            .social_cost
            .max((ev.signals.social_cost * discount).clamp(0.0, 1.0));

        commitments.turn_allocated |= ev.commitments.turn_allocated;
        commitments.fires_or_answers_expectation |= ev.commitments.fires_or_answers_expectation;
        commitments.continues_active_chain |= ev.commitments.continues_active_chain;

        let ev_salience = compute_salience(&ev.signals, prior, switches).raw_salience * discount;
        if ev_salience > best_score {
            best_score = ev_salience;
            representative_event_id = ev.event_id;
        }
    }

    let chunk_salience = compute_salience(&agg, prior, switches).raw_salience;

    Some(PlaceBacklogChunk {
        place,
        representative_event_id,
        event_ids,
        signals: agg,
        commitments,
        chunk_salience,
    })
}

/// Groups `events` per place, chunks each place (unless `!switches.backlog_chunking` Row-22 neutral
/// `per_event` mode, where each event stays its own unit), sorts by commitment + salience descending,
/// and selects at most `prior.backlog_max_chunks` (4) chunks for this wake.
#[must_use]
pub fn schedule_backlog_chunks(
    events: &[BacklogEventInput],
    now_ms: Time,
    prior: &Prior,
    switches: &Switches,
) -> BacklogScheduleResult {
    if events.is_empty() {
        return BacklogScheduleResult::default();
    }

    let mut chunks: Vec<PlaceBacklogChunk> = if switches.backlog_chunking {
        let mut by_place: BTreeMap<PlaceId, Vec<BacklogEventInput>> = BTreeMap::new();
        for ev in events {
            by_place
                .entry(ev.place.clone())
                .or_default()
                .push(ev.clone());
        }
        by_place
            .into_iter()
            .filter_map(|(place, list)| chunk_place_backlog(place, &list, now_ms, prior, switches))
            .collect()
    } else {
        // Row-22 neutral (`per_event`): single loop processes each event individually.
        events
            .iter()
            .filter_map(|ev| {
                chunk_place_backlog(
                    ev.place.clone(),
                    std::slice::from_ref(ev),
                    now_ms,
                    prior,
                    switches,
                )
            })
            .collect()
    };

    if !switches.backlog_chunking {
        return BacklogScheduleResult {
            handle_now: chunks,
            deferred: Vec::new(),
        };
    }

    chunks.sort_by(|a, b| {
        let a_comm = switches.commitment_breaker && a.commitments.any();
        let b_comm = switches.commitment_breaker && b.commitments.any();
        b_comm
            .cmp(&a_comm)
            .then_with(|| b.chunk_salience.total_cmp(&a.chunk_salience))
            .then_with(|| a.place.cmp(&b.place))
    });

    let cap = prior.backlog_max_chunks.min(chunks.len());
    let deferred = chunks.split_off(cap);
    BacklogScheduleResult {
        handle_now: chunks,
        deferred,
    }
}

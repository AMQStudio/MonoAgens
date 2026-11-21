//! Bounded per-place backlog chunking (`R-14`):
//! triggers only when an event's ingest lag exceeds `config.backlog_gap_ms` OR when Core was
//! `stopped/down`. Evaluates at most `config.max_backlog_chunks_per_wake` chunks per wake,
//! preserving all underlying events in the log.

use std::collections::BTreeMap;

use monoagens_protocol::{EventId, PlaceId, Time};
use serde::{Deserialize, Serialize};

use crate::config::CoreConfig;
use crate::gate::decide::CommitmentInputs;

/// Default maximum place chunks handled in one wake.
pub const MAX_CHUNKS_PER_WAKE: usize = 4;

/// Single backlogged event metadata before per-place chunking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacklogEventInput {
    pub event_id: EventId,
    pub place: PlaceId,
    pub timestamp_ms: Time,
    pub commitments: CommitmentInputs,
}

/// Aggregated per-place backlog chunk produced by [`chunk_place_backlog`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceBacklogChunk {
    pub place: PlaceId,
    /// Representative (latest or commitment-bearing) event id in this chunk.
    pub representative_event_id: EventId,
    /// All event ids folded into this chunk.
    pub event_ids: Vec<EventId>,
    /// Latest event timestamp in the chunk.
    pub latest_timestamp_ms: Time,
    /// Combined commitment flags (`any` event engaging a commitment fires the breaker).
    pub commitments: CommitmentInputs,
}

/// Output of [`schedule_backlog_chunks`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BacklogScheduleResult {
    pub handle_now: Vec<PlaceBacklogChunk>,
    pub deferred: Vec<PlaceBacklogChunk>,
}

/// Classifies whether an event belongs to a backlog (`R-14`).
#[must_use]
pub fn is_backlog_event(
    event_timestamp_ms: Time,
    now_ms: Time,
    backlog_gap_ms: u64,
    core_was_stopped_down: bool,
) -> bool {
    if core_was_stopped_down {
        return true;
    }
    now_ms.saturating_sub_ms(event_timestamp_ms) > backlog_gap_ms
}

/// Combines all backlogged events for a single place into one [`PlaceBacklogChunk`].
#[must_use]
pub fn chunk_place_backlog(
    place: PlaceId,
    events: &[BacklogEventInput],
) -> Option<PlaceBacklogChunk> {
    if events.is_empty() {
        return None;
    }

    let mut commitments = CommitmentInputs::default();
    let mut event_ids = Vec::with_capacity(events.len());
    let mut representative = &events[0];

    for ev in events {
        event_ids.push(ev.event_id);
        commitments.turn_allocated |= ev.commitments.turn_allocated;
        commitments.fires_or_answers_expectation |= ev.commitments.fires_or_answers_expectation;
        commitments.continues_active_chain |= ev.commitments.continues_active_chain;

        let ev_better = (ev.commitments.any(), ev.timestamp_ms, ev.event_id)
            > (
                representative.commitments.any(),
                representative.timestamp_ms,
                representative.event_id,
            );
        if ev_better {
            representative = ev;
        }
    }

    Some(PlaceBacklogChunk {
        place,
        representative_event_id: representative.event_id,
        event_ids,
        latest_timestamp_ms: representative.timestamp_ms,
        commitments,
    })
}

/// Groups `events` per place, prioritizes commitments + latest timestamps, and caps `handle_now`
/// to `config.max_backlog_chunks_per_wake`.
#[must_use]
pub fn schedule_backlog_chunks(
    events: &[BacklogEventInput],
    config: &CoreConfig,
) -> BacklogScheduleResult {
    if events.is_empty() {
        return BacklogScheduleResult::default();
    }

    let mut by_place: BTreeMap<PlaceId, Vec<BacklogEventInput>> = BTreeMap::new();
    for ev in events {
        by_place
            .entry(ev.place.clone())
            .or_default()
            .push(ev.clone());
    }

    let mut chunks: Vec<PlaceBacklogChunk> = by_place
        .into_iter()
        .filter_map(|(place, list)| chunk_place_backlog(place, &list))
        .collect();

    chunks.sort_by(|a, b| {
        b.commitments
            .any()
            .cmp(&a.commitments.any())
            .then_with(|| b.latest_timestamp_ms.cmp(&a.latest_timestamp_ms))
            .then_with(|| a.place.cmp(&b.place))
    });

    let cap = config.max_backlog_chunks_per_wake.max(1).min(chunks.len());
    let deferred = chunks.split_off(cap);
    BacklogScheduleResult {
        handle_now: chunks,
        deferred,
    }
}

//! `{open, provisional, sealed}` episode lifecycle per place, seamless reopening with
//! `superseded_by` linkage (`A-05`), monotonic `min(trust)` taint propagation, and overlap-gated
//! episode merging (`R-24`).

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{ActorId, EventId, PlaceId, TaggedEmbedding, Taint, Time};
use serde::{Deserialize, Serialize};

use crate::consolidation::boundaries::{
    BoundaryEvaluation, BoundaryInputs, PlaceBoundaryState, participant_jaccard,
};
use crate::prior::Prior;
use crate::registry::Switches;

/// Three-state episode lifecycle (`A-05`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeState {
    Open,
    Provisional,
    Sealed,
}

impl EpisodeState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Provisional => "provisional",
            Self::Sealed => "sealed",
        }
    }

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "open" => Some(Self::Open),
            "provisional" => Some(Self::Provisional),
            "sealed" => Some(Self::Sealed),
            _ => None,
        }
    }
}

/// Tracked episodic segment for one place (`A-05`, `R-24`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeRecord {
    pub episode_id: EventId,
    pub place: PlaceId,
    pub state: EpisodeState,
    pub event_ids: Vec<EventId>,
    pub participants: BTreeSet<ActorId>,
    pub start_ms: Time,
    pub end_ms: Time,
    pub centroid: Option<TaggedEmbedding>,
    pub vector_count: usize,
    pub taint: Taint,
    pub gist_id: Option<EventId>,
    pub dirty: bool,
    pub consolidated: bool,
}

/// Outcome of admitting an event into [`EpisodeBook`].
#[derive(Debug, Clone, PartialEq)]
pub struct EpisodeEventOutcome {
    pub episode_id: EventId,
    /// If a `Provisional` episode reopened smoothly, returns `(stale_gist_id, reopening_event_id)`.
    pub reopened_stale_gist: Option<(EventId, EventId)>,
    /// If a prior `Open`/`Provisional` episode became `Sealed` on boundary, returns its ID.
    pub sealed_episode_id: Option<EventId>,
    pub boundary_eval: BoundaryEvaluation,
}

/// Per-place episode manager and unconsolidated backlog tracker (`A-05`, `A-06`, `R-24`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct EpisodeBook {
    pub episodes: BTreeMap<EventId, EpisodeRecord>,
    pub active_by_place: BTreeMap<PlaceId, EventId>,
    pub boundary_states: BTreeMap<PlaceId, PlaceBoundaryState>,
    pub unconsolidated_events: BTreeSet<EventId>,
    pub consolidated_events: BTreeSet<EventId>,
}

/// Parameters for [`EpisodeBook::admit_event`].
#[derive(Debug, Clone)]
pub struct EpisodeAdmitParams<'a> {
    pub event_id: EventId,
    pub place: &'a PlaceId,
    pub timestamp_ms: Time,
    pub participants: BTreeSet<ActorId>,
    pub taint: Taint,
    pub embeddings: &'a [TaggedEmbedding],
    pub missed_expectation: bool,
}

impl EpisodeBook {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Admits a recorded event into its place's episode stream (`A-05`, `A-06`).
    pub fn admit_event(
        &mut self,
        params: EpisodeAdmitParams<'_>,
        prior: &Prior,
        switches: &Switches,
    ) -> EpisodeEventOutcome {
        let EpisodeAdmitParams {
            event_id,
            place,
            timestamp_ms,
            participants,
            taint,
            embeddings,
            missed_expectation,
        } = params;
        if event_id.as_u64() != 0 && !self.consolidated_events.contains(&event_id) {
            self.unconsolidated_events.insert(event_id);
        }

        let current_ep_id = self.active_by_place.get(place).copied();
        let boundary_state = self.boundary_states.entry(place.clone()).or_default();

        if let Some(ep_id) = current_ep_id
            && let Some(ep) = self.episodes.get(&ep_id)
        {
            let inputs = BoundaryInputs {
                timestamp_ms,
                event_embeddings: embeddings,
                episode_centroid: ep.centroid.as_ref(),
                episode_len: ep.event_ids.len(),
                event_participants: &participants,
                episode_participants: &ep.participants,
                resolved_response_embeddings: None,
                missed_expectation,
            };

            let eval = boundary_state.evaluate_and_record(&inputs, prior, switches);
            if eval.is_boundary {
                // Seal existing episode and start a new Open episode for `event_id`.
                if let Some(prev) = self.episodes.get_mut(&ep_id) {
                    prev.state = EpisodeState::Sealed;
                }
                let new_ep = create_episode(
                    event_id,
                    place.clone(),
                    timestamp_ms,
                    participants,
                    taint,
                    embeddings,
                );
                self.episodes.insert(event_id, new_ep);
                self.active_by_place.insert(place.clone(), event_id);
                return EpisodeEventOutcome {
                    episode_id: event_id,
                    reopened_stale_gist: None,
                    sealed_episode_id: Some(ep_id),
                    boundary_eval: eval,
                };
            }

            // Smooth continuation: reopen `Provisional` -> `Open` or append to `Open`.
            let ep_mut = self.episodes.get_mut(&ep_id).expect("checked");
            let mut reopened_stale_gist = None;
            if ep_mut.state == EpisodeState::Provisional {
                ep_mut.state = EpisodeState::Open;
                ep_mut.dirty = true;
                ep_mut.consolidated = false;
                if let Some(stale_gist) = ep_mut.gist_id {
                    reopened_stale_gist = Some((stale_gist, event_id));
                }
            }
            append_to_episode(
                ep_mut,
                event_id,
                timestamp_ms,
                participants,
                taint,
                embeddings,
            );
            return EpisodeEventOutcome {
                episode_id: ep_id,
                reopened_stale_gist,
                sealed_episode_id: None,
                boundary_eval: eval,
            };
        }

        // First event for this place: record into causal window and start an Open episode.
        let empty_set = BTreeSet::new();
        let inputs = BoundaryInputs {
            timestamp_ms,
            event_embeddings: embeddings,
            episode_centroid: None,
            episode_len: 0,
            event_participants: &participants,
            episode_participants: &empty_set,
            resolved_response_embeddings: None,
            missed_expectation,
        };
        let eval = boundary_state.evaluate_and_record(&inputs, prior, switches);
        let new_ep = create_episode(
            event_id,
            place.clone(),
            timestamp_ms,
            participants,
            taint,
            embeddings,
        );
        self.episodes.insert(event_id, new_ep);
        self.active_by_place.insert(place.clone(), event_id);
        EpisodeEventOutcome {
            episode_id: event_id,
            reopened_stale_gist: None,
            sealed_episode_id: None,
            boundary_eval: eval,
        }
    }

    /// Soft-closes qualifying `Open` tails at `t_snapshot` into `Provisional` (`A-05`).
    ///
    /// Returns the IDs of episodes transitioned to `Provisional`.
    pub fn soft_close_idle_tails(&mut self, t_snapshot: Time, prior: &Prior) -> Vec<EventId> {
        if !prior.provisional_close {
            return Vec::new();
        }
        let mut transitioned = Vec::new();
        let k_stale_ms = (prior.k_stale_sleep * (prior.t_idle_sleep_ms as f64)).round() as u64;

        let active_ids: Vec<(PlaceId, EventId)> = self
            .active_by_place
            .iter()
            .map(|(p, &eid)| (p.clone(), eid))
            .collect();

        for (place, ep_id) in active_ids {
            let p95_place = self
                .boundary_states
                .get(&place)
                .and_then(PlaceBoundaryState::p95_gap_ms)
                .unwrap_or(0);
            let tau_stale_ms = p95_place.max(k_stale_ms);

            if let Some(ep) = self.episodes.get_mut(&ep_id)
                && ep.state == EpisodeState::Open
                && ep.end_ms <= t_snapshot
            {
                let dt_place = t_snapshot.saturating_sub_ms(ep.end_ms);
                let len_ok = ep.event_ids.len() >= prior.n_ep_min;
                let stale_ok = dt_place > tau_stale_ms;
                if len_ok || stale_ok {
                    ep.state = EpisodeState::Provisional;
                    ep.consolidated = false;
                    transitioned.push(ep_id);
                }
            }
        }
        transitioned
    }

    /// Returns unconsolidated `Sealed` or `Provisional` episodes up to `t_snapshot` (never `Open`).
    #[must_use]
    pub fn unconsolidated_episodes_at(&self, t_snapshot: Time) -> Vec<EventId> {
        self.episodes
            .values()
            .filter(|ep| {
                !ep.consolidated
                    && ep.end_ms <= t_snapshot
                    && matches!(ep.state, EpisodeState::Sealed | EpisodeState::Provisional)
            })
            .map(|ep| ep.episode_id)
            .collect()
    }

    /// Marks an episode as consolidated with its resulting `gist_id` (clearing `dirty` and
    /// draining its events from `unconsolidated_events` so no spin loop occurs).
    pub fn mark_episode_consolidated(&mut self, episode_id: EventId, gist_id: EventId) {
        if let Some(ep) = self.episodes.get_mut(&episode_id) {
            ep.gist_id = Some(gist_id);
            ep.dirty = false;
            ep.consolidated = true;
            for &eid in &ep.event_ids {
                self.unconsolidated_events.remove(&eid);
                self.consolidated_events.insert(eid);
            }
        }
    }

    /// Attempts to merge two episodes strictly when `place` matches, `Jaccard(participants) >= J`,
    /// and time intervals overlap or fall within `merge_gap_ms` (`R-24`).
    ///
    /// Semantic vector similarity alone NEVER merges episodes.
    #[must_use]
    pub fn can_merge(a: &EpisodeRecord, b: &EpisodeRecord, prior: &Prior) -> bool {
        if a.episode_id == b.episode_id || a.place != b.place {
            return false;
        }
        if a.state == EpisodeState::Open || b.state == EpisodeState::Open {
            return false;
        }
        let jac = participant_jaccard(&a.participants, &b.participants);
        if jac < prior.merge_jaccard_min {
            return false;
        }
        let overlap = a.start_ms <= b.end_ms && b.start_ms <= a.end_ms;
        if overlap {
            return true;
        }
        let gap_ms = if a.end_ms < b.start_ms {
            b.start_ms.saturating_sub_ms(a.end_ms)
        } else {
            a.start_ms.saturating_sub_ms(b.end_ms)
        };
        gap_ms <= prior.merge_gap_ms
    }

    /// Merges adjacent/overlapping qualifying episodes per place (`R-24`), returning pairs
    /// `(merged_into_id, absorbed_id)` and updating `min(trust)` taint & provenance.
    pub fn merge_eligible_episodes(&mut self, prior: &Prior) -> Vec<(EventId, EventId)> {
        let mut merges = Vec::new();
        let mut by_place: BTreeMap<PlaceId, Vec<EventId>> = BTreeMap::new();
        for ep in self.episodes.values() {
            if ep.state != EpisodeState::Open {
                by_place
                    .entry(ep.place.clone())
                    .or_default()
                    .push(ep.episode_id);
            }
        }

        for (_place, mut ids) in by_place {
            ids.sort_by_key(|id| {
                self.episodes
                    .get(id)
                    .map_or(Time::from_millis(0), |e| e.start_ms)
            });
            let mut idx = 0usize;
            while idx < ids.len() {
                let id_a = ids[idx];
                let mut merged_any = false;
                let mut j = idx + 1;
                while j < ids.len() {
                    let id_b = ids[j];
                    let can = match (self.episodes.get(&id_a), self.episodes.get(&id_b)) {
                        (Some(a), Some(b)) => Self::can_merge(a, b, prior),
                        _ => false,
                    };
                    if can {
                        let b = self.episodes.remove(&id_b).expect("checked");
                        if let Some(a) = self.episodes.get_mut(&id_a) {
                            for eid in b.event_ids {
                                if !a.event_ids.contains(&eid) {
                                    a.event_ids.push(eid);
                                }
                            }
                            a.participants.extend(b.participants);
                            a.start_ms = a.start_ms.min(b.start_ms);
                            a.end_ms = a.end_ms.max(b.end_ms);
                            a.taint = a.taint.combine(b.taint);
                            a.dirty = true;
                            a.consolidated = false;
                            if self.active_by_place.get(&a.place).copied() == Some(id_b) {
                                self.active_by_place.insert(a.place.clone(), id_a);
                            }
                        }
                        merges.push((id_a, id_b));
                        ids.remove(j);
                        merged_any = true;
                    } else {
                        j += 1;
                    }
                }
                if !merged_any {
                    idx += 1;
                }
            }
        }
        merges
    }

    /// Removes erased events/actors from episodes on `hard_forget` (`R-34`).
    pub fn forget(&mut self, erased_events: &[EventId], erased_actors: &[ActorId]) {
        let ev_set: BTreeSet<EventId> = erased_events.iter().copied().collect();
        let actor_set: BTreeSet<ActorId> = erased_actors.iter().cloned().collect();

        for eid in &ev_set {
            self.unconsolidated_events.remove(eid);
            self.consolidated_events.remove(eid);
        }

        let mut drop_eps = Vec::new();
        for (&ep_id, ep) in &mut self.episodes {
            ep.event_ids.retain(|id| !ev_set.contains(id));
            for actor in &actor_set {
                ep.participants.remove(actor);
            }
            if let Some(gid) = ep.gist_id
                && ev_set.contains(&gid)
            {
                ep.gist_id = None;
            }
            if ep.event_ids.is_empty() || ev_set.contains(&ep_id) {
                drop_eps.push(ep_id);
            }
        }
        for ep_id in drop_eps {
            self.episodes.remove(&ep_id);
        }
        self.active_by_place
            .retain(|_, ep_id| self.episodes.contains_key(ep_id));
        for bs in self.boundary_states.values_mut() {
            for actor in &actor_set {
                bs.last_participants.remove(actor);
            }
        }
    }
}

fn create_episode(
    episode_id: EventId,
    place: PlaceId,
    timestamp_ms: Time,
    participants: BTreeSet<ActorId>,
    taint: Taint,
    embeddings: &[TaggedEmbedding],
) -> EpisodeRecord {
    let mut ep = EpisodeRecord {
        episode_id,
        place,
        state: EpisodeState::Open,
        event_ids: Vec::new(),
        participants,
        start_ms: timestamp_ms,
        end_ms: timestamp_ms,
        centroid: None,
        vector_count: 0,
        taint,
        gist_id: None,
        dirty: false,
        consolidated: false,
    };
    append_to_episode(
        &mut ep,
        episode_id,
        timestamp_ms,
        BTreeSet::new(),
        taint,
        embeddings,
    );
    ep
}

fn append_to_episode(
    ep: &mut EpisodeRecord,
    event_id: EventId,
    timestamp_ms: Time,
    participants: BTreeSet<ActorId>,
    taint: Taint,
    embeddings: &[TaggedEmbedding],
) {
    if event_id.as_u64() != 0 && !ep.event_ids.contains(&event_id) {
        ep.event_ids.push(event_id);
    }
    ep.participants.extend(participants);
    ep.start_ms = ep.start_ms.min(timestamp_ms);
    ep.end_ms = ep.end_ms.max(timestamp_ms);
    ep.taint = ep.taint.combine(taint);

    if let Some(emb) = embeddings.first()
        && !emb.values.is_empty()
    {
        match &mut ep.centroid {
            None => {
                ep.centroid = Some(TaggedEmbedding {
                    tag: emb.tag.clone(),
                    values: normalize_vec(&emb.values),
                });
                ep.vector_count = 1;
            }
            Some(c) if c.tag == emb.tag && c.values.len() == emb.values.len() => {
                let n = ep.vector_count as f64;
                let mut sum: Vec<f64> = c
                    .values
                    .iter()
                    .zip(emb.values.iter())
                    .map(|(&old, &new)| f64::from(old) * n + f64::from(new))
                    .collect();
                let norm = sum.iter().map(|x| x * x).sum::<f64>().sqrt();
                if norm > 1e-12 {
                    for x in &mut sum {
                        *x /= norm;
                    }
                    c.values = sum.into_iter().map(|x| x as f32).collect();
                }
                ep.vector_count += 1;
            }
            Some(_) => {}
        }
    }
}

fn normalize_vec(v: &[f32]) -> Vec<f32> {
    let norm = v
        .iter()
        .map(|&x| f64::from(x) * f64::from(x))
        .sum::<f64>()
        .sqrt();
    if norm <= 1e-12 || !norm.is_finite() {
        return v.to_vec();
    }
    v.iter().map(|&x| (f64::from(x) / norm) as f32).collect()
}

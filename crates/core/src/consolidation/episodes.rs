//! `{open, provisional, sealed}` episode lifecycle per place, seamless reopening with
//! `superseded_by` linkage, monotonic `min(trust)` taint propagation, and overlap-gated
//! episode merging.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{ActorId, Audience, EventId, PlaceId, TaggedEmbedding, Taint, Time};
use serde::{Deserialize, Serialize};

use crate::config::CoreConfig;
use crate::gate::cosine_similarity;

const DEFAULT_EPISODE_GAP_MS: u64 = 1_800_000;
const DEFAULT_N_EP_MIN: usize = 3;
const DEFAULT_MERGE_JACCARD_MIN: f64 = 0.5;
const DEFAULT_MERGE_GAP_MS: u64 = 3_600_000;

/// Maximum concurrent active thread slots maintained per `(PlaceId, CanonicalAudienceHash)` scope.
pub const MAX_ACTIVE_THREADS_PER_SCOPE: usize = 6;

const THREAD_ASSIGN_THRESHOLD: f64 = 0.35;
const THREAD_W_SEM: f64 = 0.55;
const THREAD_W_WHO: f64 = 0.25;
const THREAD_W_WHEN: f64 = 0.20;
const THREAD_TAU_SECS: f64 = 600.0;
const THREAD_REPLY_FASTPATH_BONUS: f64 = 1.0;

/// Computes the canonical BLAKE3 digest over sorted unique actor IDs in `audience` (`\n`-delimited).
#[must_use]
pub fn canonical_audience_hash(audience: &Audience) -> [u8; 32] {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for member in &audience.members {
        names.insert(member.as_str());
    }
    hash_sorted_actor_names(&names)
}

fn canonical_participants_hash(participants: &BTreeSet<ActorId>) -> [u8; 32] {
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for member in participants {
        names.insert(member.as_str());
    }
    hash_sorted_actor_names(&names)
}

fn hash_sorted_actor_names(names: &BTreeSet<&str>) -> [u8; 32] {
    let mut buf = String::new();
    for (idx, name) in names.iter().enumerate() {
        if idx > 0 {
            buf.push('\n');
        }
        buf.push_str(name);
    }
    crate::blake3::hash(buf.as_bytes())
}

/// Three-state episode lifecycle.
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

/// Computes Jaccard overlap `|A ∩ B| / |A ∪ B|` across participant sets (`0.0` if both empty).
#[must_use]
pub fn participant_jaccard(a: &BTreeSet<ActorId>, b: &BTreeSet<ActorId>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f64;
    let union = a.union(b).count() as f64;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// Tracked episodic segment for one place.
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpisodeEventOutcome {
    pub episode_id: EventId,
    pub reopened_stale_gist: Option<(EventId, EventId)>,
    pub sealed_episode_id: Option<EventId>,
    pub is_boundary: bool,
}

/// Per-place episode manager and unconsolidated backlog tracker with streaming 3W multi-slot
/// disentanglement (`M <= MAX_ACTIVE_THREADS_PER_SCOPE` per `(PlaceId, CanonicalAudienceHash)`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EpisodeBook {
    pub episodes: BTreeMap<EventId, EpisodeRecord>,
    pub active_by_place: BTreeMap<PlaceId, EventId>,
    pub unconsolidated_events: BTreeSet<EventId>,
    pub consolidated_events: BTreeSet<EventId>,
    /// Active thread slots keyed strictly by `(PlaceId, CanonicalAudienceHash)` (`M <= 6`).
    #[serde(default)]
    pub active_slots: BTreeMap<(PlaceId, [u8; 32]), Vec<EventId>>,
    /// Deterministic mapping `event_id -> thread_id` (root `episode_id`).
    #[serde(default)]
    pub event_threads: BTreeMap<EventId, EventId>,
    /// Canonical audience hash recorded per episode for audience-locked rollups & isolation.
    #[serde(default)]
    pub episode_audiences: BTreeMap<EventId, [u8; 32]>,
}

impl PartialEq for EpisodeBook {
    fn eq(&self, other: &Self) -> bool {
        self.episodes == other.episodes
            && self.active_by_place == other.active_by_place
            && self.unconsolidated_events == other.unconsolidated_events
            && self.consolidated_events == other.consolidated_events
    }
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

    /// Returns the assigned thread/episode ID for `event_id`, if tracked.
    #[must_use]
    pub fn thread_for_event(&self, event_id: EventId) -> Option<EventId> {
        if let Some(&tid) = self.event_threads.get(&event_id)
            && self.episodes.contains_key(&tid)
        {
            return Some(tid);
        }
        self.episodes
            .values()
            .find(|ep| ep.episode_id == event_id || ep.event_ids.contains(&event_id))
            .map(|ep| ep.episode_id)
    }

    /// Returns the canonical audience hash for `episode_id` (falling back to its participants).
    #[must_use]
    pub fn audience_hash_for_episode(&self, episode_id: EventId) -> Option<[u8; 32]> {
        if let Some(&h) = self.episode_audiences.get(&episode_id) {
            return Some(h);
        }
        self.episodes
            .get(&episode_id)
            .map(|ep| canonical_participants_hash(&ep.participants))
    }

    /// Returns currently active (non-`Sealed`) thread records strictly within `(place, audience)`.
    #[must_use]
    pub fn active_threads_for_scope(
        &self,
        place: &PlaceId,
        audience: &Audience,
    ) -> Vec<&EpisodeRecord> {
        let aud_hash = canonical_audience_hash(audience);
        let Some(slot_ids) = self.active_slots.get(&(place.clone(), aud_hash)) else {
            return Vec::new();
        };
        slot_ids
            .iter()
            .filter_map(|id| self.episodes.get(id))
            .filter(|ep| ep.state != EpisodeState::Sealed)
            .collect()
    }

    /// Admits a recorded event into its place's episode stream using deterministic 3W multi-slot
    /// disentanglement, using `params.participants` as fallback audience scope.
    pub fn admit_event(
        &mut self,
        params: EpisodeAdmitParams<'_>,
        config: &CoreConfig,
    ) -> EpisodeEventOutcome {
        self.admit_event_with_thread(params, None, None, config)
    }

    /// Admits a recorded event with optional explicit `audience` scope and `reply_to` fast-path
    /// into up to [`MAX_ACTIVE_THREADS_PER_SCOPE`] active slots per `(PlaceId, CanonicalAudienceHash)`.
    pub fn admit_event_with_thread(
        &mut self,
        params: EpisodeAdmitParams<'_>,
        audience: Option<&Audience>,
        reply_to: Option<EventId>,
        config: &CoreConfig,
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

        let aud_hash = match audience {
            Some(aud) => canonical_audience_hash(aud),
            None => canonical_participants_hash(&participants),
        };
        let scope_key = (place.clone(), aud_hash);

        let mut slot_ids = self.active_slots.remove(&scope_key).unwrap_or_default();
        slot_ids.retain(|id| {
            self.episodes
                .get(id)
                .is_some_and(|ep| ep.state != EpisodeState::Sealed)
        });

        // Backward-compatible hydration from legacy snapshots that only stored `active_by_place`.
        if slot_ids.is_empty()
            && let Some(&legacy_id) = self.active_by_place.get(place)
            && self
                .episodes
                .get(&legacy_id)
                .is_some_and(|ep| ep.state != EpisodeState::Sealed)
            && self.audience_hash_for_episode(legacy_id) == Some(aud_hash)
        {
            slot_ids.push(legacy_id);
        }

        let gap_threshold = DEFAULT_EPISODE_GAP_MS.max(config.sleep_idle_ms);
        let mut gap_expired: Vec<EventId> = Vec::new();
        let mut best_match: Option<(f64, EventId)> = None;
        let mut best_any: Option<(f64, EventId)> = None;

        for &ep_id in &slot_ids {
            let Some(ep) = self.episodes.get(&ep_id) else {
                continue;
            };
            let gap_ms = timestamp_ms.saturating_sub_ms(ep.end_ms);
            let is_gap_expired = ep.event_ids.len() >= DEFAULT_N_EP_MIN && gap_ms > gap_threshold;
            let has_reply_link = reply_to.is_some_and(|rid| ep.event_ids.contains(&rid));

            let sem_cos = max_centroid_cosine(embeddings, ep.centroid.as_ref());
            let actor_overlap = participant_jaccard(&participants, &ep.participants);
            let dt_s = (gap_ms as f64) / 1_000.0;
            let when_score = (-dt_s / THREAD_TAU_SECS).exp();
            let reply_bonus = if has_reply_link {
                THREAD_REPLY_FASTPATH_BONUS
            } else {
                0.0
            };
            let score = THREAD_W_SEM * sem_cos
                + THREAD_W_WHO * actor_overlap
                + THREAD_W_WHEN * when_score
                + reply_bonus;

            if better_slot_candidate(score, ep_id, best_any) {
                best_any = Some((score, ep_id));
            }
            if is_gap_expired && !has_reply_link {
                gap_expired.push(ep_id);
                continue;
            }
            if better_slot_candidate(score, ep_id, best_match) {
                best_match = Some((score, ep_id));
            }
        }

        if !missed_expectation
            && let Some((best_score, best_ep_id)) = best_match
            && best_score >= THREAD_ASSIGN_THRESHOLD
        {
            let ep_mut = self.episodes.get_mut(&best_ep_id).expect("checked");
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
            if event_id.as_u64() != 0 {
                self.event_threads.insert(event_id, best_ep_id);
            }
            self.active_slots.insert(scope_key, slot_ids);
            self.active_by_place.insert(place.clone(), best_ep_id);
            return EpisodeEventOutcome {
                episode_id: best_ep_id,
                reopened_stale_gist,
                sealed_episode_id: None,
                is_boundary: false,
            };
        }

        // Open a new thread slot.
        let had_existing_slots = !slot_ids.is_empty();
        let mut sealed_episode_id: Option<EventId> = None;

        // Seal any gap-expired slots first (deterministic smallest EventId order).
        gap_expired.sort();
        for expired_id in gap_expired {
            if let Some(ep) = self.episodes.get_mut(&expired_id) {
                ep.state = EpisodeState::Sealed;
            }
            slot_ids.retain(|&id| id != expired_id);
            if sealed_episode_id.is_none() {
                sealed_episode_id = Some(expired_id);
            }
        }

        if missed_expectation
            && sealed_episode_id.is_none()
            && let Some((_, target_id)) = best_any
        {
            if let Some(ep) = self.episodes.get_mut(&target_id) {
                ep.state = EpisodeState::Sealed;
            }
            slot_ids.retain(|&id| id != target_id);
            sealed_episode_id = Some(target_id);
        }

        while slot_ids.len() >= MAX_ACTIVE_THREADS_PER_SCOPE {
            if let Some(oldest_id) = slot_ids
                .iter()
                .copied()
                .min_by_key(|id| {
                    let end_ms = self
                        .episodes
                        .get(id)
                        .map_or(Time::from_millis(0), |e| e.end_ms);
                    (end_ms, *id)
                })
            {
                if let Some(ep) = self.episodes.get_mut(&oldest_id) {
                    ep.state = EpisodeState::Sealed;
                }
                slot_ids.retain(|&id| id != oldest_id);
                if sealed_episode_id.is_none() {
                    sealed_episode_id = Some(oldest_id);
                }
            } else {
                break;
            }
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
        self.episode_audiences.insert(event_id, aud_hash);
        if event_id.as_u64() != 0 {
            self.event_threads.insert(event_id, event_id);
        }
        slot_ids.push(event_id);
        self.active_slots.insert(scope_key, slot_ids);
        self.active_by_place.insert(place.clone(), event_id);

        let is_boundary = missed_expectation || sealed_episode_id.is_some() || had_existing_slots;
        EpisodeEventOutcome {
            episode_id: event_id,
            reopened_stale_gist: None,
            sealed_episode_id,
            is_boundary,
        }
    }

    /// Returns sealed, consolidated episode IDs eligible for tier-2 temporal rollup strictly
    /// within `(place, canonical_audience_hash(audience))` (never crossing audience boundaries).
    #[must_use]
    pub fn tier2_temporal_rollup_candidates(
        &self,
        place: &PlaceId,
        audience: &Audience,
    ) -> Vec<EventId> {
        let aud_hash = canonical_audience_hash(audience);
        let mut cands: Vec<(Time, EventId)> = self
            .episodes
            .values()
            .filter(|ep| {
                &ep.place == place
                    && ep.state == EpisodeState::Sealed
                    && ep.consolidated
                    && self.audience_hash_for_episode(ep.episode_id) == Some(aud_hash)
            })
            .map(|ep| (ep.start_ms, ep.episode_id))
            .collect();
        cands.sort_by_key(|&(start, id)| (start, id));
        cands.into_iter().map(|(_, id)| id).collect()
    }

    /// Soft-closes qualifying `Open` tails at `t_snapshot` into `Provisional` across all active slots.
    pub fn soft_close_idle_tails(&mut self, t_snapshot: Time, config: &CoreConfig) -> Vec<EventId> {
        let mut transitioned = Vec::new();
        let mut active_ids: BTreeSet<EventId> =
            self.active_by_place.values().copied().collect();
        for slots in self.active_slots.values() {
            for &eid in slots {
                active_ids.insert(eid);
            }
        }

        for ep_id in active_ids {
            if let Some(ep) = self.episodes.get_mut(&ep_id)
                && ep.state == EpisodeState::Open
                && ep.end_ms <= t_snapshot
            {
                let dt_place = t_snapshot.saturating_sub_ms(ep.end_ms);
                if ep.event_ids.len() >= DEFAULT_N_EP_MIN || dt_place > config.sleep_idle_ms {
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

    /// Marks an episode as consolidated with its resulting `gist_id`.
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

    /// Attempts to merge two episodes when `place` matches, `Jaccard(participants) >= min`,
    /// and time intervals overlap or fall within `merge_gap_ms`.
    #[must_use]
    pub fn can_merge(a: &EpisodeRecord, b: &EpisodeRecord) -> bool {
        if a.episode_id == b.episode_id || a.place != b.place {
            return false;
        }
        if a.state == EpisodeState::Open || b.state == EpisodeState::Open {
            return false;
        }
        if participant_jaccard(&a.participants, &b.participants) < DEFAULT_MERGE_JACCARD_MIN {
            return false;
        }
        if a.start_ms <= b.end_ms && b.start_ms <= a.end_ms {
            return true;
        }
        let gap_ms = if a.end_ms < b.start_ms {
            b.start_ms.saturating_sub_ms(a.end_ms)
        } else {
            a.start_ms.saturating_sub_ms(b.end_ms)
        };
        gap_ms <= DEFAULT_MERGE_GAP_MS
    }

    /// Merges adjacent/overlapping qualifying episodes per place.
    pub fn merge_eligible_episodes(&mut self) -> Vec<(EventId, EventId)> {
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
                    let same_audience = match (
                        self.episode_audiences.get(&id_a),
                        self.episode_audiences.get(&id_b),
                    ) {
                        (Some(ha), Some(hb)) => ha == hb,
                        _ => true,
                    };
                    let can = same_audience
                        && match (self.episodes.get(&id_a), self.episodes.get(&id_b)) {
                            (Some(a), Some(b)) => Self::can_merge(a, b),
                            _ => false,
                        };
                    if can {
                        let b = self.episodes.remove(&id_b).expect("checked");
                        self.episode_audiences.remove(&id_b);
                        if let Some(a) = self.episodes.get_mut(&id_a) {
                            for eid in b.event_ids {
                                if !a.event_ids.contains(&eid) {
                                    a.event_ids.push(eid);
                                }
                                self.event_threads.insert(eid, id_a);
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
                        for slots in self.active_slots.values_mut() {
                            for slot in slots.iter_mut() {
                                if *slot == id_b {
                                    *slot = id_a;
                                }
                            }
                            let mut seen = BTreeSet::new();
                            slots.retain(|id| seen.insert(*id));
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

    /// Removes erased events/actors from episodes on `hard_forget`, clearing any stale
    /// centroids on surviving partial episodes and scrubbing `active_slots`, `event_threads`,
    /// and `episode_audiences` so zero residual traces remain (`C-8`).
    pub fn forget(&mut self, erased_events: &[EventId], erased_actors: &[ActorId]) {
        let ev_set: BTreeSet<EventId> = erased_events.iter().copied().collect();
        let actor_set: BTreeSet<ActorId> = erased_actors.iter().cloned().collect();

        for eid in &ev_set {
            self.unconsolidated_events.remove(eid);
            self.consolidated_events.remove(eid);
            self.event_threads.remove(eid);
        }

        let mut drop_eps = Vec::new();
        for (&ep_id, ep) in &mut self.episodes {
            let before_len = ep.event_ids.len();
            ep.event_ids.retain(|id| !ev_set.contains(id));
            let mut actor_removed = false;
            for actor in &actor_set {
                if ep.participants.remove(actor) {
                    actor_removed = true;
                }
            }
            if let Some(gid) = ep.gist_id
                && ev_set.contains(&gid)
            {
                ep.gist_id = None;
            }
            if ep.event_ids.is_empty() || ev_set.contains(&ep_id) {
                drop_eps.push(ep_id);
            } else if ep.event_ids.len() < before_len || actor_removed {
                // Clear centroid and vector_count immediately so zero residual floats of
                // erased events survive in the partial episode (`C-8`).
                ep.centroid = None;
                ep.vector_count = 0;
                ep.dirty = true;
                ep.consolidated = false;
            }
        }
        for ep_id in drop_eps {
            self.episodes.remove(&ep_id);
            self.episode_audiences.remove(&ep_id);
        }
        self.active_by_place
            .retain(|_, ep_id| self.episodes.contains_key(ep_id));
        self.event_threads
            .retain(|eid, tid| !ev_set.contains(eid) && self.episodes.contains_key(tid));
        self.episode_audiences
            .retain(|ep_id, _| self.episodes.contains_key(ep_id));
        for slots in self.active_slots.values_mut() {
            slots.retain(|ep_id| self.episodes.contains_key(ep_id));
        }
        self.active_slots.retain(|_, slots| !slots.is_empty());
    }

    /// Recomputes `centroid` and `vector_count` strictly from surviving non-erased `memory_entries`
    /// for any surviving episodes whose centroid was cleared by [`Self::forget`].
    pub fn recompute_centroids_from_entries(
        &mut self,
        memory_entries: &BTreeMap<EventId, crate::recall::MemoryEntry>,
    ) {
        for ep in self.episodes.values_mut() {
            if ep.centroid.is_some() || ep.event_ids.is_empty() {
                continue;
            }
            let mut count = 0usize;
            let mut tag_ref: Option<monoagens_protocol::EmbeddingTag> = None;
            let mut accum: Vec<f64> = Vec::new();
            for eid in &ep.event_ids {
                let Some(entry) = memory_entries.get(eid) else {
                    continue;
                };
                if entry.erased {
                    continue;
                }
                let Some(emb) = entry.embeddings.first() else {
                    continue;
                };
                if emb.values.is_empty() {
                    continue;
                }
                match &tag_ref {
                    None => {
                        tag_ref = Some(emb.tag.clone());
                        accum = emb.values.iter().map(|&v| f64::from(v)).collect();
                        count = 1;
                    }
                    Some(tag) if tag == &emb.tag && accum.len() == emb.values.len() => {
                        for (acc, &v) in accum.iter_mut().zip(emb.values.iter()) {
                            *acc += f64::from(v);
                        }
                        count += 1;
                    }
                    Some(_) => {}
                }
            }
            if let Some(tag) = tag_ref
                && count > 0
            {
                let norm = accum.iter().map(|x| x * x).sum::<f64>().sqrt();
                if norm > 1e-12 && norm.is_finite() {
                    let values = accum.into_iter().map(|x| (x / norm) as f32).collect();
                    ep.centroid = Some(TaggedEmbedding { tag, values });
                    ep.vector_count = count;
                }
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

fn max_centroid_cosine(embeddings: &[TaggedEmbedding], centroid: Option<&TaggedEmbedding>) -> f64 {
    let Some(c) = centroid else {
        return 0.0;
    };
    let mut best = 0.0_f64;
    for emb in embeddings {
        if emb.tag == c.tag
            && let Some(sim) = cosine_similarity(&emb.values, &c.values)
            && sim.is_finite()
        {
            best = best.max(sim.clamp(0.0, 1.0));
        }
    }
    best
}

fn better_slot_candidate(score: f64, ep_id: EventId, current_best: Option<(f64, EventId)>) -> bool {
    match current_best {
        None => true,
        Some((best_score, best_id)) => match score.total_cmp(&best_score) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Equal => ep_id < best_id,
            std::cmp::Ordering::Less => false,
        },
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

#[cfg(test)]
mod tests {
    use monoagens_protocol::{Audience, EmbeddingTag, OriginActor};

    use super::*;
    use crate::activation::ActivationTrace;
    use crate::recall::{ImportanceInputs, MemoryBasis, MemoryEntry};

    fn test_tag() -> EmbeddingTag {
        EmbeddingTag {
            embed_model_ver: "egemma2-256".to_owned(),
            dim_d: 4,
            prompt_regime_id: "t_d".to_owned(),
        }
    }

    #[test]
    fn partial_hard_forget_clears_and_recomputes_episode_centroid_without_residual_trace() {
        let tag = test_tag();
        let cfg = CoreConfig::default();
        let mut book = EpisodeBook::new();
        let place = PlaceId::new("room:a");
        let shared_aud = Audience::new([
            ActorId::new("alice"),
            ActorId::new("bob"),
            ActorId::new("soul"),
        ]);

        // Event 10: [1, 0, 0, 0] (survives), Event 11: [0, 1, 0, 0] replies to 10 (erased later)
        let e10 = TaggedEmbedding {
            tag: tag.clone(),
            values: vec![1.0, 0.0, 0.0, 0.0],
        };
        let e11 = TaggedEmbedding {
            tag: tag.clone(),
            values: vec![0.0, 1.0, 0.0, 0.0],
        };
        let out1 = book.admit_event_with_thread(
            EpisodeAdmitParams {
                event_id: EventId::new(10),
                place: &place,
                timestamp_ms: Time::from_millis(1_000),
                participants: BTreeSet::from([ActorId::new("alice")]),
                taint: Taint::Trusted,
                embeddings: std::slice::from_ref(&e10),
                missed_expectation: false,
            },
            Some(&shared_aud),
            None,
            &cfg,
        );
        let out2 = book.admit_event_with_thread(
            EpisodeAdmitParams {
                event_id: EventId::new(11),
                place: &place,
                timestamp_ms: Time::from_millis(2_000),
                participants: BTreeSet::from([ActorId::new("bob")]),
                taint: Taint::Trusted,
                embeddings: std::slice::from_ref(&e11),
                missed_expectation: false,
            },
            Some(&shared_aud),
            Some(EventId::new(10)),
            &cfg,
        );
        assert_eq!(out2.episode_id, out1.episode_id);
        assert_eq!(book.thread_for_event(EventId::new(11)), Some(out1.episode_id));

        let ep = book.episodes.get(&out1.episode_id).expect("episode");
        let c_before = ep.centroid.as_ref().expect("centroid");
        assert!(c_before.values[0] > 0.5, "centroid initially mixes event 10");

        // Hard-forget event 11 -> surviving partial episode (ep_id=10) must immediately clear centroid (C-8)
        // and scrub event_threads for event 11.
        book.forget(&[EventId::new(11)], &[]);
        assert_eq!(book.thread_for_event(EventId::new(11)), None);
        let ep_after = book.episodes.get(&out1.episode_id).expect("surviving episode");
        assert_eq!(ep_after.event_ids, vec![EventId::new(10)]);
        assert!(
            ep_after.centroid.is_none(),
            "partial forget must clear stale centroid immediately"
        );
        assert_eq!(ep_after.vector_count, 0);

        // Recomputing from surviving memory_entries must yield pure e10 ([1, 0, 0, 0]), zero residual e11.
        let mut entries = BTreeMap::new();
        entries.insert(
            EventId::new(10),
            MemoryEntry {
                event_id: EventId::new(10),
                source_ids: vec![EventId::new(10)],
                is_gist: false,
                timestamp_ms: Time::from_millis(1_000),
                place: place.clone(),
                actor: ActorId::new("alice"),
                origin_actor: OriginActor::new(ActorId::new("alice")),
                basis: MemoryBasis::Observed,
                taint: Taint::Trusted,
                audience: Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
                embeddings: vec![e10],
                attachments: Vec::new(),
                activation: ActivationTrace::default(),
                importance: ImportanceInputs::default(),
                concepts: Vec::new(),
                superseded_by: None,
                erased: false,
            },
        );
        book.recompute_centroids_from_entries(&entries);
        let c_recomputed = book.episodes[&out1.episode_id]
            .centroid
            .as_ref()
            .expect("recomputed centroid");
        assert!((c_recomputed.values[0] - 1.0).abs() < 1e-6);
        assert_eq!(c_recomputed.values[1], 0.0);
    }

    #[test]
    fn streaming_3w_disentangles_interleaved_threads_and_caps_at_six_slots() {
        let tag = test_tag();
        let cfg = CoreConfig::default();
        let mut book = EpisodeBook::new();
        let place = PlaceId::new("room:group");
        let group_aud = Audience::new([
            ActorId::new("alice"),
            ActorId::new("bob"),
            ActorId::new("carol"),
            ActorId::new("dave"),
            ActorId::new("soul"),
        ]);

        let emb_topic_a = TaggedEmbedding {
            tag: tag.clone(),
            values: vec![1.0, 0.0, 0.0, 0.0],
        };
        let emb_topic_b = TaggedEmbedding {
            tag: tag.clone(),
            values: vec![0.0, 1.0, 0.0, 0.0],
        };

        // Interleaved Topic A (alice) and Topic B (bob) within same room/audience stay separated!
        let a1 = book.admit_event_with_thread(
            EpisodeAdmitParams {
                event_id: EventId::new(100),
                place: &place,
                timestamp_ms: Time::from_millis(1_000),
                participants: BTreeSet::from([ActorId::new("alice")]),
                taint: Taint::Trusted,
                embeddings: std::slice::from_ref(&emb_topic_a),
                missed_expectation: false,
            },
            Some(&group_aud),
            None,
            &cfg,
        );
        let b1 = book.admit_event_with_thread(
            EpisodeAdmitParams {
                event_id: EventId::new(101),
                place: &place,
                timestamp_ms: Time::from_millis(2_000),
                participants: BTreeSet::from([ActorId::new("bob")]),
                taint: Taint::Trusted,
                embeddings: std::slice::from_ref(&emb_topic_b),
                missed_expectation: false,
            },
            Some(&group_aud),
            None,
            &cfg,
        );
        assert_ne!(a1.episode_id, b1.episode_id);

        // Follow-up on Topic A (carol with emb_topic_a) joins Topic A.
        let a2 = book.admit_event_with_thread(
            EpisodeAdmitParams {
                event_id: EventId::new(102),
                place: &place,
                timestamp_ms: Time::from_millis(3_000),
                participants: BTreeSet::from([ActorId::new("carol")]),
                taint: Taint::Trusted,
                embeddings: std::slice::from_ref(&emb_topic_a),
                missed_expectation: false,
            },
            Some(&group_aud),
            None,
            &cfg,
        );
        assert_eq!(a2.episode_id, a1.episode_id);

        // Reply-to fast-path attaches orthogonal message directly to Topic B!
        let b2 = book.admit_event_with_thread(
            EpisodeAdmitParams {
                event_id: EventId::new(103),
                place: &place,
                timestamp_ms: Time::from_millis(4_000),
                participants: BTreeSet::from([ActorId::new("dave")]),
                taint: Taint::Trusted,
                embeddings: std::slice::from_ref(&emb_topic_a),
                missed_expectation: false,
            },
            Some(&group_aud),
            Some(EventId::new(101)),
            &cfg,
        );
        assert_eq!(b2.episode_id, b1.episode_id);
        assert_eq!(book.thread_for_event(EventId::new(102)), Some(EventId::new(100)));
        assert_eq!(book.thread_for_event(EventId::new(103)), Some(EventId::new(101)));

        // Open 5 more orthogonal/distinct slots -> cap at M=6 must seal oldest end_ms slot (a1, end_ms=3000).
        let mut sealed_first = None;
        for idx in 0..5u64 {
            let mut vals = vec![0.0; 4];
            vals[(idx as usize) % 4] = if idx < 2 { -1.0 } else { 0.0 };
            let emb = TaggedEmbedding {
                tag: tag.clone(),
                values: vals,
            };
            let out = book.admit_event_with_thread(
                EpisodeAdmitParams {
                    event_id: EventId::new(200 + idx),
                    place: &place,
                    timestamp_ms: Time::from_millis(10_000 + idx * 1_000),
                    participants: BTreeSet::from([ActorId::new(format!("user_{idx}"))]),
                    taint: Taint::Trusted,
                    embeddings: std::slice::from_ref(&emb),
                    missed_expectation: false,
                },
                Some(&group_aud),
                None,
                &cfg,
            );
            if out.sealed_episode_id.is_some() && sealed_first.is_none() {
                sealed_first = out.sealed_episode_id;
            }
        }
        assert_eq!(sealed_first, Some(a1.episode_id));
        assert_eq!(book.episodes[&a1.episode_id].state, EpisodeState::Sealed);
        assert!(book.active_threads_for_scope(&place, &group_aud).len() <= MAX_ACTIVE_THREADS_PER_SCOPE);
    }

    #[test]
    fn audience_isolation_and_tier2_rollup_never_cross_audiences_and_forget_scrubs_slots() {
        let tag = test_tag();
        let cfg = CoreConfig::default();
        let mut book = EpisodeBook::new();
        let place = PlaceId::new("room:shared");
        let aud_pub = Audience::new([ActorId::new("alice"), ActorId::new("bob"), ActorId::new("soul")]);
        let aud_priv = Audience::new([ActorId::new("alice"), ActorId::new("soul")]);
        let emb = TaggedEmbedding {
            tag,
            values: vec![1.0, 0.0, 0.0, 0.0],
        };

        let pub_ep = book.admit_event_with_thread(
            EpisodeAdmitParams {
                event_id: EventId::new(10),
                place: &place,
                timestamp_ms: Time::from_millis(1_000),
                participants: BTreeSet::from([ActorId::new("alice")]),
                taint: Taint::Trusted,
                embeddings: std::slice::from_ref(&emb),
                missed_expectation: false,
            },
            Some(&aud_pub),
            None,
            &cfg,
        );
        let priv_ep = book.admit_event_with_thread(
            EpisodeAdmitParams {
                event_id: EventId::new(20),
                place: &place,
                timestamp_ms: Time::from_millis(1_500),
                participants: BTreeSet::from([ActorId::new("alice")]),
                taint: Taint::Trusted,
                embeddings: std::slice::from_ref(&emb),
                missed_expectation: false,
            },
            Some(&aud_priv),
            None,
            &cfg,
        );
        assert_ne!(
            pub_ep.episode_id, priv_ep.episode_id,
            "identical embeddings/place must never merge across distinct audiences"
        );

        // Seal & consolidate both -> tier2_temporal_rollup_candidates strictly audience-locked!
        book.episodes.get_mut(&pub_ep.episode_id).unwrap().state = EpisodeState::Sealed;
        book.mark_episode_consolidated(pub_ep.episode_id, EventId::new(1000));
        book.episodes.get_mut(&priv_ep.episode_id).unwrap().state = EpisodeState::Sealed;
        book.mark_episode_consolidated(priv_ep.episode_id, EventId::new(2000));

        assert_eq!(
            book.tier2_temporal_rollup_candidates(&place, &aud_pub),
            vec![pub_ep.episode_id]
        );
        assert_eq!(
            book.tier2_temporal_rollup_candidates(&place, &aud_priv),
            vec![priv_ep.episode_id]
        );

        // Hard-forget private episode scrubs active_slots, event_threads, and episode_audiences cleanly.
        book.forget(&[priv_ep.episode_id], &[]);
        assert!(!book.episodes.contains_key(&priv_ep.episode_id));
        assert!(!book.episode_audiences.contains_key(&priv_ep.episode_id));
        assert!(book.thread_for_event(priv_ep.episode_id).is_none());
        assert!(book.tier2_temporal_rollup_candidates(&place, &aud_priv).is_empty());
        assert_eq!(
            book.tier2_temporal_rollup_candidates(&place, &aud_pub),
            vec![pub_ep.episode_id]
        );
    }
}

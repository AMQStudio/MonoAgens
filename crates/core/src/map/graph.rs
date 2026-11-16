//! Successor Representation (SR) and co-occurrence topological cognitive map (`R-32`).
//!
//! Operates strictly over opaque `{actor, place, concept}` vertex identifiers without
//! reading raw payload bytes. Spreading activation uses bounded deterministic power
//! iterations (`hop_budget`) — never unbounded random walks — and bridges related events
//! even when stored under incompatible `embed_model_ver` partitions.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{ActorId, EventId, PlaceId, Time};
use serde::{Deserialize, Serialize};

use crate::activation::MS_PER_HOUR;
use crate::prior::Prior;

/// Typed opaque vertex in the topological cognitive map (`R-32`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MapVertex {
    Actor(ActorId),
    Place(PlaceId),
    Concept(String),
}

impl MapVertex {
    /// Constructs an actor vertex.
    #[must_use]
    pub fn actor(id: ActorId) -> Self {
        Self::Actor(id)
    }

    /// Constructs a place vertex.
    #[must_use]
    pub fn place(id: PlaceId) -> Self {
        Self::Place(id)
    }

    /// Constructs an opaque concept/domain/capability vertex.
    #[must_use]
    pub fn concept(id: impl Into<String>) -> Self {
        Self::Concept(id.into())
    }
}

/// Weighted undirected co-occurrence edge `(weight, last_updated_ms)`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct CoOccurrenceEdge {
    pub weight: f64,
    pub last_updated_ms: Time,
}

/// Topological cognitive map storing co-occurrence adjacency `W` and TD Successor
/// Representation `M` over opaque `{actor, place, concept}` identifiers (`R-32`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CognitiveMap {
    /// Symmetric co-occurrence adjacency `W[u][v]`.
    pub co_occurrence: BTreeMap<MapVertex, BTreeMap<MapVertex, CoOccurrenceEdge>>,
    /// Asymmetric TD Successor Representation `M[u][v]` (truncated to top-`K_sr` per row).
    pub sr_matrix: BTreeMap<MapVertex, BTreeMap<MapVertex, f64>>,
    /// Most recent anchor vertex and timestamp per place for intra-episode TD-SR updates.
    pub last_place_anchor: BTreeMap<PlaceId, (MapVertex, Time)>,
    /// Inverted index from map vertices to associated event/memory IDs.
    pub vertex_events: BTreeMap<MapVertex, BTreeSet<EventId>>,
    /// Forward index from event/memory IDs to their bound vertices `V(m)`.
    pub event_vertices: BTreeMap<EventId, Vec<MapVertex>>,
}

impl CognitiveMap {
    /// Creates an empty [`CognitiveMap`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records co-occurring opaque vertices `vertices` for `event_id` in `place` at `now_ms`,
    /// updating symmetric co-occurrence weights `W` and intra-episode TD-SR `M`.
    pub fn record_event_vertices(
        &mut self,
        event_id: EventId,
        vertices: &[MapVertex],
        place: &PlaceId,
        now_ms: Time,
        prior: &Prior,
    ) {
        let mut unique: Vec<MapVertex> = Vec::with_capacity(vertices.len());
        for v in vertices {
            if !unique.contains(v) {
                unique.push(v.clone());
            }
        }
        if unique.is_empty() {
            return;
        }

        if event_id.as_u64() != 0 {
            for v in &unique {
                self.vertex_events
                    .entry(v.clone())
                    .or_default()
                    .insert(event_id);
            }
            self.event_vertices.insert(event_id, unique.clone());
        }

        // 1. Update continuous-time co-occurrence edges W[u, v].
        let lambda = prior.map_lambda_decay.max(0.0);
        let eta_co = prior.map_eta_co.max(0.0);
        let w_max = prior.map_w_max.max(1.0);
        if eta_co > 0.0 && unique.len() >= 2 {
            for i in 0..unique.len() {
                for j in (i + 1)..unique.len() {
                    let u = &unique[i];
                    let v = &unique[j];
                    Self::bump_co_edge(
                        &mut self.co_occurrence,
                        u,
                        v,
                        now_ms,
                        lambda,
                        eta_co,
                        w_max,
                    );
                    Self::bump_co_edge(
                        &mut self.co_occurrence,
                        v,
                        u,
                        now_ms,
                        lambda,
                        eta_co,
                        w_max,
                    );
                }
            }
        }

        // 2. Update TD Successor Representation M[u_prev, v] along intra-episode transitions.
        let anchor_vertex = unique
            .iter()
            .find(|v| matches!(v, MapVertex::Concept(_)))
            .or_else(|| unique.iter().find(|v| matches!(v, MapVertex::Actor(_))))
            .unwrap_or(&unique[0])
            .clone();

        if let Some((prev_v, prev_ts)) = self.last_place_anchor.get(place).cloned() {
            let dt_s = now_ms.saturating_sub_ms(prev_ts) as f64 / 1000.0;
            if prev_v != anchor_vertex && dt_s <= prior.map_ep_gap_s && prior.map_alpha_sr > 0.0 {
                let step_ratio = (dt_s / prior.map_tau_step_s.max(1.0)).max(1.0);
                let disc = prior.map_gamma_sr.clamp(0.0, 0.99).powf(step_ratio);
                let alpha = prior.map_alpha_sr.clamp(0.0, 1.0);

                let succ_row = self
                    .sr_matrix
                    .get(&anchor_vertex)
                    .cloned()
                    .unwrap_or_default();
                let mut candidates: BTreeSet<MapVertex> = BTreeSet::new();
                candidates.insert(prev_v.clone());
                candidates.insert(anchor_vertex.clone());
                if let Some(row) = self.sr_matrix.get(&prev_v) {
                    for k in row.keys() {
                        candidates.insert(k.clone());
                    }
                }
                for k in succ_row.keys() {
                    candidates.insert(k.clone());
                }

                let row = self.sr_matrix.entry(prev_v.clone()).or_default();
                for target_v in candidates {
                    let ind = if target_v == prev_v { 1.0 } else { 0.0 };
                    let succ_m = succ_row.get(&target_v).copied().unwrap_or(0.0);
                    let cur_m = row.get(&target_v).copied().unwrap_or(0.0);
                    let delta = ind + disc * succ_m - cur_m;
                    let updated = (cur_m + alpha * delta).max(0.0);
                    if updated > 1e-9 {
                        row.insert(target_v, updated);
                    } else {
                        row.remove(&target_v);
                    }
                }
                truncate_top_k(row, prior.map_k_sr);
            }
        }

        self.last_place_anchor
            .insert(place.clone(), (anchor_vertex, now_ms));
    }

    fn bump_co_edge(
        co: &mut BTreeMap<MapVertex, BTreeMap<MapVertex, CoOccurrenceEdge>>,
        u: &MapVertex,
        v: &MapVertex,
        now_ms: Time,
        lambda: f64,
        eta_co: f64,
        w_max: f64,
    ) {
        let entry = co
            .entry(u.clone())
            .or_default()
            .entry(v.clone())
            .or_default();
        let decayed = if entry.weight > 0.0 {
            let dt_hours = now_ms.saturating_sub_ms(entry.last_updated_ms) as f64 / MS_PER_HOUR;
            entry.weight * (-lambda * dt_hours).exp()
        } else {
            0.0
        };
        entry.weight = (decayed + eta_co).min(w_max);
        entry.last_updated_ms = now_ms;
    }

    /// Directly links two opaque vertices with an undirected co-occurrence edge (useful for
    /// structural associations and unit testing).
    pub fn add_edge(&mut self, u: MapVertex, v: MapVertex, weight: f64, now_ms: Time) {
        if u == v || !weight.is_finite() || weight <= 0.0 {
            return;
        }
        let edge = CoOccurrenceEdge {
            weight,
            last_updated_ms: now_ms,
        };
        self.co_occurrence
            .entry(u.clone())
            .or_default()
            .insert(v.clone(), edge);
        self.co_occurrence.entry(v).or_default().insert(u, edge);
    }

    /// Associates `event_id` with `vertices` for recall lookup without reading payload text.
    pub fn bind_event(&mut self, event_id: EventId, vertices: &[MapVertex]) {
        let mut unique = Vec::with_capacity(vertices.len());
        for v in vertices {
            if !unique.contains(v) {
                unique.push(v.clone());
                self.vertex_events
                    .entry(v.clone())
                    .or_default()
                    .insert(event_id);
            }
        }
        if !unique.is_empty() {
            self.event_vertices.insert(event_id, unique);
        }
    }

    /// Runs deterministic, bounded multi-hop spreading activation (Personalized PageRank)
    /// from `seeds` for at most `hop_budget` steps (never an unbounded random walk).
    #[must_use]
    pub fn spread_activation(
        &self,
        seeds: &[MapVertex],
        hop_budget: usize,
        prior: &Prior,
    ) -> BTreeMap<MapVertex, f64> {
        let mut unique_seeds: Vec<MapVertex> = Vec::new();
        for s in seeds {
            if !unique_seeds.contains(s) {
                unique_seeds.push(s.clone());
            }
        }
        if unique_seeds.is_empty() {
            return BTreeMap::new();
        }

        let seed_mass = 1.0 / (unique_seeds.len() as f64);
        let mut s0: BTreeMap<MapVertex, f64> = BTreeMap::new();
        for s in unique_seeds {
            s0.insert(s, seed_mass);
        }

        if hop_budget == 0 {
            return s0;
        }

        let beta = prior.map_beta_sr.clamp(0.0, 1.0);
        let delta = prior.map_delta_ppr.clamp(0.0, 0.95);
        let k_sr = prior.map_k_sr.max(1);
        let mut r = s0.clone();

        for _ in 0..hop_budget {
            let mut next: BTreeMap<MapVertex, f64> = BTreeMap::new();
            for (v, &sv) in &s0 {
                *next.entry(v.clone()).or_insert(0.0) += (1.0 - delta) * sv;
            }

            let mut active_nodes: Vec<(MapVertex, f64)> =
                r.iter().map(|(k, &v)| (k.clone(), v)).collect();
            active_nodes.sort_by(|a, b| a.0.cmp(&b.0));

            for (u, ru) in active_nodes {
                if ru <= 0.0 {
                    continue;
                }
                let neighbors = self.normalized_out_edges(&u, beta, k_sr);
                if neighbors.is_empty() {
                    continue;
                }
                for (v, p_uv) in neighbors {
                    *next.entry(v).or_insert(0.0) += delta * ru * p_uv;
                }
            }

            r = next;
        }

        r
    }

    fn normalized_out_edges(
        &self,
        u: &MapVertex,
        beta_sr: f64,
        deg_max: usize,
    ) -> Vec<(MapVertex, f64)> {
        let mut combined: BTreeMap<MapVertex, f64> = BTreeMap::new();

        if let Some(co_row) = self.co_occurrence.get(u) {
            for (v, edge) in co_row {
                if v != u && edge.weight > 0.0 {
                    *combined.entry(v.clone()).or_insert(0.0) += (1.0 - beta_sr) * edge.weight;
                }
            }
        }
        if let Some(sr_row) = self.sr_matrix.get(u) {
            for (v, &w) in sr_row {
                if v != u && w > 0.0 {
                    *combined.entry(v.clone()).or_insert(0.0) += beta_sr * w;
                }
            }
        }

        let mut ranked: Vec<(MapVertex, f64)> =
            combined.into_iter().filter(|(_, w)| *w > 0.0).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        ranked.truncate(deg_max.max(1));

        let sum: f64 = ranked.iter().map(|(_, w)| *w).sum();
        if sum <= 0.0 {
            return Vec::new();
        }
        for (_, w) in &mut ranked {
            *w /= sum;
        }
        ranked
    }

    /// Computes bounded `R_map(q, event_id) in [0, 1]` from precomputed spreading activation scores.
    #[must_use]
    pub fn score_event(
        &self,
        event_id: EventId,
        activation_scores: &BTreeMap<MapVertex, f64>,
    ) -> f64 {
        let Some(verts) = self.event_vertices.get(&event_id) else {
            return 0.0;
        };
        let sum: f64 = verts.iter().filter_map(|v| activation_scores.get(v)).sum();
        if sum.is_finite() {
            sum.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// Collects candidate `(EventId, r_map)` pairs reachable from activated vertices,
    /// sorted deterministically by `(score DESC, event_id ASC)`.
    ///
    /// Callers MUST project returned IDs through `VisibleScope` BEFORE FOK, ranking, or use-count updates.
    #[must_use]
    pub fn candidate_events_for_activation(
        &self,
        activation_scores: &BTreeMap<MapVertex, f64>,
        limit: usize,
    ) -> Vec<(EventId, f64)> {
        if limit == 0 || activation_scores.is_empty() {
            return Vec::new();
        }
        let mut by_event: BTreeMap<EventId, f64> = BTreeMap::new();
        for (vertex, &score) in activation_scores {
            if score <= 0.0 {
                continue;
            }
            if let Some(ev_ids) = self.vertex_events.get(vertex) {
                for &ev_id in ev_ids {
                    let entry = by_event.entry(ev_id).or_insert(0.0);
                    *entry = (*entry + score).clamp(0.0, 1.0);
                }
            }
        }
        let mut out: Vec<(EventId, f64)> = by_event.into_iter().collect();
        out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out.truncate(limit);
        out
    }

    /// Erases all vertex associations and edges for `event_id` (`hard_forget`).
    pub fn forget_event(&mut self, event_id: EventId) {
        if let Some(verts) = self.event_vertices.remove(&event_id) {
            for v in verts {
                if let Some(set) = self.vertex_events.get_mut(&v) {
                    set.remove(&event_id);
                    if set.is_empty() {
                        self.vertex_events.remove(&v);
                    }
                }
            }
        }
    }

    /// Erases an actor vertex and all edges referencing it (`hard_forget`).
    pub fn forget_actor(&mut self, actor: &ActorId) {
        let target = MapVertex::Actor(actor.clone());
        self.co_occurrence.remove(&target);
        for row in self.co_occurrence.values_mut() {
            row.remove(&target);
        }
        self.sr_matrix.remove(&target);
        for row in self.sr_matrix.values_mut() {
            row.remove(&target);
        }
        self.vertex_events.remove(&target);
        for verts in self.event_vertices.values_mut() {
            verts.retain(|v| v != &target);
        }
        self.last_place_anchor.retain(|_, (v, _)| v != &target);
    }
}

fn truncate_top_k(row: &mut BTreeMap<MapVertex, f64>, k: usize) {
    if row.len() <= k.max(1) {
        return;
    }
    let mut pairs: Vec<(MapVertex, f64)> = row.iter().map(|(v, &w)| (v.clone(), w)).collect();
    pairs.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    pairs.truncate(k.max(1));
    row.clear();
    for (v, w) in pairs {
        row.insert(v, w);
    }
}

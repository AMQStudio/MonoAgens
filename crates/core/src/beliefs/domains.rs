//! Warm-started spherical k-means domain clustering and Hungarian cross-snapshot alignment (`A-12`).

use std::collections::BTreeMap;

use monoagens_protocol::{ActorId, DomainId, EmbeddingTag, EventId, TaggedEmbedding};
use serde::{Deserialize, Serialize};

use crate::gate::cosine_similarity;
use crate::prior::Prior;
use crate::rng::SeededRng;

/// Default neutral domain identifier used during cold start (`N_vec < m_min`) or `K = 1`.
pub const DEFAULT_DOMAIN_ID: &str = "d_0";

/// Lifecycle transition of a domain centroid across sleep snapshots (`A-12`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainTransitionKind {
    Continued,
    Split,
    Birth,
    Merged,
    Retired,
}

impl DomainTransitionKind {
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            Self::Continued => 1,
            Self::Split => 2,
            Self::Birth => 3,
            Self::Merged => 4,
            Self::Retired => 5,
        }
    }

    #[must_use]
    pub const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Continued),
            2 => Some(Self::Split),
            3 => Some(Self::Birth),
            4 => Some(Self::Merged),
            5 => Some(Self::Retired),
            _ => None,
        }
    }
}

/// Recorded transition of a domain across snapshots (`EventKind::DomainsUpdated`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainTransitionRecord {
    pub kind: DomainTransitionKind,
    pub old_domain: Option<DomainId>,
    pub new_domain: Option<DomainId>,
    pub similarity: f64,
}

/// Recorded `(actor, domain)` Beta credence transfer/re-basing across domain split/merge/drift (`A-12`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredenceRebaseRecord {
    pub actor: ActorId,
    pub from_domain: DomainId,
    pub to_domain: DomainId,
    pub new_alpha: f64,
    pub new_beta: f64,
    pub alpha_0: f64,
    pub beta_0: f64,
}

/// Active or retired domain centroid tracked across sleep runs (`A-12`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainCentroid {
    pub domain_id: DomainId,
    pub centroid: TaggedEmbedding,
    pub active: bool,
    pub member_count: usize,
}

/// Output of a sleep-time domain clustering and Hungarian alignment pass.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DomainClusterUpdate {
    pub changed: bool,
    pub centroids: Vec<DomainCentroid>,
    pub transitions: Vec<DomainTransitionRecord>,
    pub assignments: BTreeMap<EventId, DomainId>,
}

/// Deterministic warm-started spherical k-means + Hungarian domain aligner (`A-12`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DomainClusterBook {
    pub clusters: BTreeMap<DomainId, DomainCentroid>,
    pub next_domain_seq: u64,
}

impl DomainClusterBook {
    #[must_use]
    pub fn new() -> Self {
        Self {
            clusters: BTreeMap::new(),
            next_domain_seq: 1,
        }
    }

    /// Assigns an embedding slice to the highest-cosine active domain centroid with matching tag.
    #[must_use]
    pub fn assign_domain(&self, embeddings: &[TaggedEmbedding]) -> DomainId {
        let mut best: Option<(f64, &DomainId)> = None;
        for emb in embeddings {
            for (did, cluster) in &self.clusters {
                if !cluster.active || cluster.centroid.tag != emb.tag {
                    continue;
                }
                if let Some(sim) = cosine_similarity(&emb.values, &cluster.centroid.values) {
                    let take = match best {
                        None => true,
                        Some((best_sim, best_id)) => {
                            sim > best_sim + 1e-12
                                || ((sim - best_sim).abs() <= 1e-12 && did < best_id)
                        }
                    };
                    if take {
                        best = Some((sim, did));
                    }
                }
            }
        }
        best.map_or_else(|| DomainId::new(DEFAULT_DOMAIN_ID), |(_, d)| d.clone())
    }

    /// Runs cold-start-guarded, warm-started spherical k-means and Hungarian matching (`A-12`).
    pub fn update_clusters(
        &mut self,
        active_vectors: &[(EventId, TaggedEmbedding)],
        seed: u64,
        sleep_seq: u64,
        prior: &Prior,
    ) -> DomainClusterUpdate {
        if active_vectors.is_empty() {
            return DomainClusterUpdate::default();
        }

        // Partition by dominant tag (first sorted event's tag) so incompatible embedders never mix.
        let mut sorted: Vec<(EventId, TaggedEmbedding)> = active_vectors.to_vec();
        sorted.sort_by_key(|(eid, _)| *eid);
        let active_tag: EmbeddingTag = sorted[0].1.tag.clone();
        let filtered: Vec<(EventId, Vec<f32>)> = sorted
            .into_iter()
            .filter(|(_, e)| e.tag == active_tag && !e.values.is_empty())
            .map(|(eid, e)| (eid, normalize_f32(&e.values)))
            .filter(|(_, v)| !v.is_empty())
            .collect();

        if filtered.is_empty() {
            return DomainClusterUpdate::default();
        }

        let n_vec = filtered.len();
        let k_max = prior.domain_clusters_k.max(1);
        let m_min = prior.domain_m_min.max(1);

        // Cold start (`N_vec < m_min`) or neutral single-domain (`K = 1`): use `d_0`.
        if k_max == 1 || n_vec < m_min {
            let centroid_vec = mean_normalized(filtered.iter().map(|(_, v)| v.as_slice()));
            let d0 = DomainId::new(DEFAULT_DOMAIN_ID);
            let centroid = DomainCentroid {
                domain_id: d0.clone(),
                centroid: TaggedEmbedding {
                    tag: active_tag,
                    values: centroid_vec,
                },
                active: true,
                member_count: n_vec,
            };
            let had_prev = self.clusters.contains_key(&d0);
            let sim = self
                .clusters
                .get(&d0)
                .and_then(|old| cosine_similarity(&old.centroid.values, &centroid.centroid.values))
                .unwrap_or(1.0);
            let mut assignments = BTreeMap::new();
            for (eid, _) in &filtered {
                assignments.insert(*eid, d0.clone());
            }
            let transition = DomainTransitionRecord {
                kind: if had_prev {
                    DomainTransitionKind::Continued
                } else {
                    DomainTransitionKind::Birth
                },
                old_domain: if had_prev { Some(d0.clone()) } else { None },
                new_domain: Some(d0.clone()),
                similarity: sim,
            };
            self.clusters.insert(d0, centroid.clone());
            return DomainClusterUpdate {
                changed: true,
                centroids: vec![centroid],
                transitions: vec![transition],
                assignments,
            };
        }

        let k_eff = (n_vec / m_min).clamp(1, k_max);
        let mut old_active: Vec<DomainCentroid> = self
            .clusters
            .values()
            .filter(|c| c.active && c.centroid.tag == active_tag)
            .cloned()
            .collect();
        old_active.sort_by(|a, b| a.domain_id.cmp(&b.domain_id));

        let dim = filtered[0].1.len();
        let mut rng = SeededRng::for_event(seed, EventId::new(sleep_seq.max(1)));

        // Initialize centroids: warm-start from `old_active`, plus k-means++ for remaining slots.
        let mut centroids: Vec<Vec<f32>> = Vec::with_capacity(k_eff);
        for old in old_active.iter().take(k_eff) {
            if old.centroid.values.len() == dim {
                centroids.push(normalize_f32(&old.centroid.values));
            }
        }
        if centroids.is_empty() {
            let first_idx = (rng.next_u64() as usize) % filtered.len();
            centroids.push(filtered[first_idx].1.clone());
        }
        while centroids.len() < k_eff {
            let mut weights = Vec::with_capacity(filtered.len());
            let mut sum_w = 0.0_f64;
            for (_, v) in &filtered {
                let max_cos = centroids
                    .iter()
                    .filter_map(|c| cosine_similarity(v, c))
                    .fold(-1.0_f64, f64::max);
                let dist_sq = (2.0 * (1.0 - max_cos)).max(0.0);
                weights.push(dist_sq);
                sum_w += dist_sq;
            }
            if sum_w <= 1e-12 {
                let idx = (rng.next_u64() as usize) % filtered.len();
                centroids.push(filtered[idx].1.clone());
            } else {
                let target = rng.next_unit_f64() * sum_w;
                let mut acc = 0.0;
                let mut chosen = 0usize;
                for (idx, w) in weights.iter().enumerate() {
                    acc += *w;
                    if acc >= target {
                        chosen = idx;
                        break;
                    }
                }
                centroids.push(filtered[chosen].1.clone());
            }
        }

        // Evolutionary spherical k-means iterations.
        let max_iter = prior.domain_max_iter.max(1);
        let mut point_cluster = vec![0usize; filtered.len()];
        for _ in 0..max_iter {
            let mut changed = false;
            for (i, (_, v)) in filtered.iter().enumerate() {
                let mut best_j = 0usize;
                let mut best_sim = f64::NEG_INFINITY;
                for (j, c) in centroids.iter().enumerate() {
                    let sim = cosine_similarity(v, c).unwrap_or(-1.0);
                    if sim > best_sim + 1e-12 {
                        best_sim = sim;
                        best_j = j;
                    }
                }
                if point_cluster[i] != best_j {
                    point_cluster[i] = best_j;
                    changed = true;
                }
            }

            let mut next_centroids = vec![vec![0.0_f64; dim]; centroids.len()];
            let mut counts = vec![0usize; centroids.len()];
            for (i, (_, v)) in filtered.iter().enumerate() {
                let c_idx = point_cluster[i];
                counts[c_idx] += 1;
                for (d, val) in v.iter().enumerate() {
                    next_centroids[c_idx][d] += f64::from(*val);
                }
            }
            for (j, sum_vec) in next_centroids.iter_mut().enumerate() {
                if counts[j] == 0 {
                    continue;
                }
                if j < old_active.len() && prior.domain_w_hist > 0.0 {
                    let old_v = normalize_f32(&old_active[j].centroid.values);
                    for (d, val) in old_v.iter().enumerate() {
                        sum_vec[d] += prior.domain_w_hist * f64::from(*val);
                    }
                }
                let norm = sum_vec.iter().map(|x| x * x).sum::<f64>().sqrt();
                if norm > 1e-12 {
                    for (d, x) in sum_vec.iter().enumerate() {
                        centroids[j][d] = (*x / norm) as f32;
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // Drop empty clusters and remap non-empty cluster indices cleanly.
        let mut non_empty_indices = Vec::new();
        let mut counts = vec![0usize; centroids.len()];
        for &c_idx in &point_cluster {
            counts[c_idx] += 1;
        }
        for (j, &cnt) in counts.iter().enumerate() {
            if cnt > 0 {
                non_empty_indices.push(j);
            }
        }
        let new_centroids: Vec<(Vec<f32>, usize)> = non_empty_indices
            .iter()
            .map(|&j| (centroids[j].clone(), counts[j]))
            .collect();
        let mut compact_map = vec![0usize; centroids.len()];
        for (compact_j, &orig_j) in non_empty_indices.iter().enumerate() {
            compact_map[orig_j] = compact_j;
        }

        // Hungarian matching between `old_active` and `new_centroids`.
        let rows = old_active.len();
        let cols = new_centroids.len();
        let mut assigned_new_ids: Vec<Option<DomainId>> = vec![None; cols];
        let mut old_matched = vec![false; rows];
        let mut transitions = Vec::new();

        if rows > 0 && cols > 0 {
            let mut cost = vec![vec![2.0_f64; cols]; rows];
            let mut sims = vec![vec![0.0_f64; cols]; rows];
            for i in 0..rows {
                for j in 0..cols {
                    let sim =
                        cosine_similarity(&old_active[i].centroid.values, &new_centroids[j].0)
                            .unwrap_or(0.0);
                    sims[i][j] = sim;
                    cost[i][j] = (1.0 - sim).max(0.0);
                }
            }
            let col_to_row = hungarian_min_cost(&cost);
            for (j, maybe_i) in col_to_row.into_iter().enumerate() {
                if let Some(i) = maybe_i {
                    let sim = sims[i][j];
                    if sim >= prior.domain_tau_match {
                        let kept_id = old_active[i].domain_id.clone();
                        assigned_new_ids[j] = Some(kept_id.clone());
                        old_matched[i] = true;
                        transitions.push(DomainTransitionRecord {
                            kind: DomainTransitionKind::Continued,
                            old_domain: Some(kept_id.clone()),
                            new_domain: Some(kept_id),
                            similarity: sim,
                        });
                    }
                }
            }
        }

        // Unmatched new centroids -> Split or Birth with fresh monotonic IDs.
        for j in 0..cols {
            if assigned_new_ids[j].is_some() {
                continue;
            }
            let new_id = self.allocate_fresh_domain_id();
            assigned_new_ids[j] = Some(new_id.clone());
            let mut best_old: Option<(f64, &DomainId)> = None;
            for old in &old_active {
                let sim =
                    cosine_similarity(&old.centroid.values, &new_centroids[j].0).unwrap_or(0.0);
                if best_old.is_none_or(|(b, _)| sim > b) {
                    best_old = Some((sim, &old.domain_id));
                }
            }
            if let Some((best_sim, parent_id)) = best_old
                && best_sim >= prior.domain_tau_rel
            {
                transitions.push(DomainTransitionRecord {
                    kind: DomainTransitionKind::Split,
                    old_domain: Some(parent_id.clone()),
                    new_domain: Some(new_id),
                    similarity: best_sim,
                });
            } else {
                transitions.push(DomainTransitionRecord {
                    kind: DomainTransitionKind::Birth,
                    old_domain: None,
                    new_domain: Some(new_id),
                    similarity: best_old.map_or(0.0, |(s, _)| s),
                });
            }
        }

        // Unmatched old clusters -> Merge or Retire.
        for (i, old) in old_active.iter().enumerate() {
            if old_matched[i] {
                continue;
            }
            let mut best_surv: Option<(f64, &DomainId)> = None;
            for j in 0..cols {
                if let Some(surv_id) = &assigned_new_ids[j] {
                    let sim =
                        cosine_similarity(&old.centroid.values, &new_centroids[j].0).unwrap_or(0.0);
                    if best_surv.is_none_or(|(b, _)| sim > b) {
                        best_surv = Some((sim, surv_id));
                    }
                }
            }
            if let Some((best_sim, surv_id)) = best_surv
                && best_sim >= prior.domain_tau_merge
            {
                transitions.push(DomainTransitionRecord {
                    kind: DomainTransitionKind::Merged,
                    old_domain: Some(old.domain_id.clone()),
                    new_domain: Some(surv_id.clone()),
                    similarity: best_sim,
                });
            } else {
                transitions.push(DomainTransitionRecord {
                    kind: DomainTransitionKind::Retired,
                    old_domain: Some(old.domain_id.clone()),
                    new_domain: None,
                    similarity: best_surv.map_or(0.0, |(s, _)| s),
                });
            }
            if let Some(c) = self.clusters.get_mut(&old.domain_id) {
                c.active = false;
            }
        }

        let mut updated_centroids = Vec::with_capacity(cols);
        for (j, (vec, count)) in new_centroids.into_iter().enumerate() {
            let did = assigned_new_ids[j].clone().expect("assigned");
            let entry = DomainCentroid {
                domain_id: did.clone(),
                centroid: TaggedEmbedding {
                    tag: active_tag.clone(),
                    values: vec,
                },
                active: true,
                member_count: count,
            };
            self.clusters.insert(did, entry.clone());
            updated_centroids.push(entry);
        }

        let mut assignments = BTreeMap::new();
        for (i, (eid, _)) in filtered.iter().enumerate() {
            let compact_j = compact_map[point_cluster[i]];
            if let Some(did) = &assigned_new_ids[compact_j] {
                assignments.insert(*eid, did.clone());
            }
        }

        DomainClusterUpdate {
            changed: true,
            centroids: updated_centroids,
            transitions,
            assignments,
        }
    }

    fn allocate_fresh_domain_id(&mut self) -> DomainId {
        loop {
            let candidate = DomainId::new(format!("d_{}", self.next_domain_seq));
            self.next_domain_seq = self.next_domain_seq.saturating_add(1);
            if !self.clusters.contains_key(&candidate) {
                return candidate;
            }
        }
    }

    /// Applies an event-sourced `DomainsUpdated` snapshot deterministically for replay.
    pub fn apply_domains_updated(
        &mut self,
        centroids: &[(DomainId, TaggedEmbedding)],
        transitions: &[DomainTransitionRecord],
    ) {
        for tr in transitions {
            if matches!(
                tr.kind,
                DomainTransitionKind::Merged | DomainTransitionKind::Retired
            ) && let Some(old_id) = &tr.old_domain
                && let Some(c) = self.clusters.get_mut(old_id)
            {
                c.active = false;
            }
        }
        for (did, emb) in centroids {
            if let Some(suffix) = did.as_str().strip_prefix("d_")
                && let Ok(n) = suffix.parse::<u64>()
                && n >= self.next_domain_seq
            {
                self.next_domain_seq = n.saturating_add(1);
            }
            self.clusters.insert(
                did.clone(),
                DomainCentroid {
                    domain_id: did.clone(),
                    centroid: emb.clone(),
                    active: true,
                    member_count: 1,
                },
            );
        }
    }
}

/// Exact Kuhn-Munkres (Hungarian) `O(n^3)` minimum-cost bipartite matching.
///
/// Given a `rows x cols` cost matrix, returns a vector of length `cols` where
/// `result[j] = Some(i)` indicates column `j` is matched to row `i`.
#[must_use]
pub fn hungarian_min_cost(cost: &[Vec<f64>]) -> Vec<Option<usize>> {
    let rows = cost.len();
    let cols = cost.first().map_or(0, Vec::len);
    if rows == 0 || cols == 0 {
        return vec![None; cols];
    }
    let n = rows.max(cols);
    let mut u = vec![0.0_f64; n + 1];
    let mut v = vec![0.0_f64; n + 1];
    let mut p = vec![0_usize; n + 1];
    let mut way = vec![0_usize; n + 1];

    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0_usize;
        let mut minv = vec![f64::INFINITY; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let mut delta = f64::INFINITY;
            let mut j1 = 0_usize;
            for j in 1..=n {
                if !used[j] {
                    let c = if i0 <= rows && j <= cols {
                        let raw = cost[i0 - 1][j - 1];
                        if raw.is_finite() { raw } else { 10.0 }
                    } else {
                        10.0
                    };
                    let cur = c - u[i0] - v[j];
                    if cur < minv[j] {
                        minv[j] = cur;
                        way[j] = j0;
                    }
                    if minv[j] < delta {
                        delta = minv[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    minv[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }

    let mut out = vec![None; cols];
    for j in 1..=cols {
        if p[j] >= 1 && p[j] <= rows {
            out[j - 1] = Some(p[j] - 1);
        }
    }
    out
}

fn normalize_f32(v: &[f32]) -> Vec<f32> {
    let norm = v
        .iter()
        .map(|&x| f64::from(x) * f64::from(x))
        .sum::<f64>()
        .sqrt();
    if norm <= 1e-12 || !norm.is_finite() {
        return Vec::new();
    }
    v.iter().map(|&x| (f64::from(x) / norm) as f32).collect()
}

fn mean_normalized<'a>(vecs: impl Iterator<Item = &'a [f32]>) -> Vec<f32> {
    let mut sum: Vec<f64> = Vec::new();
    for v in vecs {
        if sum.is_empty() {
            sum.resize(v.len(), 0.0);
        }
        if v.len() == sum.len() {
            for (d, val) in v.iter().enumerate() {
                sum[d] += f64::from(*val);
            }
        }
    }
    let norm = sum.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm <= 1e-12 || !norm.is_finite() {
        return sum.into_iter().map(|_| 0.0).collect();
    }
    sum.into_iter().map(|x| (x / norm) as f32).collect()
}

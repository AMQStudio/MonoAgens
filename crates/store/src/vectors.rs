//! Partitioned `VectorIndex` trait (`usearch`, `hnsw_rs`, `BruteForceFlat`) with traversal-time filtering and Petrov fade.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use hnsw_rs::prelude::{DistCosine, FilterT, Hnsw};
use monoagens_core::{ActivationTrace, StoreError};
use monoagens_protocol::{EmbeddingTag, EventId, Time};
#[cfg(feature = "usearch")]
use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

#[cfg(feature = "usearch")]
use crate::schema::UsearchQuantization;
use crate::schema::VectorBackendConfig;

const DENSE_BITMAP_BITS: u64 = 1 << 20;

/// Selected vector index backend kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorBackendKind {
    #[cfg(feature = "usearch")]
    Usearch(UsearchQuantization),
    HnswRs,
    BruteForceFlat,
}

/// Bitmap of live and visible [`EventId`]s evaluated during HNSW traversal (not post-filtered).
#[derive(Debug, Clone, Default)]
pub struct IdBitmap {
    allow_all: bool,
    dense_words: Vec<u64>,
    overflow: BTreeSet<u64>,
}

impl IdBitmap {
    /// Creates an empty bitmap allowing no event ids.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a bitmap permitting any live event id in the partition.
    #[must_use]
    pub fn allow_all() -> Self {
        Self {
            allow_all: true,
            dense_words: Vec::new(),
            overflow: BTreeSet::new(),
        }
    }

    /// Builds a bitmap containing all supplied [`EventId`]s.
    #[must_use]
    pub fn from_ids(ids: impl IntoIterator<Item = EventId>) -> Self {
        let mut bm = Self::new();
        for id in ids {
            bm.insert(id);
        }
        bm
    }

    /// Adds `id` to the bitmap.
    pub fn insert(&mut self, id: EventId) {
        if self.allow_all {
            return;
        }
        let raw = id.as_u64();
        if raw < DENSE_BITMAP_BITS {
            let word = (raw / 64) as usize;
            let bit = raw % 64;
            if word >= self.dense_words.len() {
                self.dense_words.resize(word + 1, 0);
            }
            self.dense_words[word] |= 1_u64 << bit;
        } else {
            self.overflow.insert(raw);
        }
    }

    /// Removes `id` from the bitmap.
    pub fn remove(&mut self, id: EventId) {
        if self.allow_all {
            return;
        }
        let raw = id.as_u64();
        if raw < DENSE_BITMAP_BITS {
            let word = (raw / 64) as usize;
            let bit = raw % 64;
            if let Some(slot) = self.dense_words.get_mut(word) {
                *slot &= !(1_u64 << bit);
            }
        } else {
            self.overflow.remove(&raw);
        }
    }

    /// Returns `true` if `id` is permitted by this bitmap.
    #[inline]
    #[must_use]
    pub fn contains(&self, id: EventId) -> bool {
        if self.allow_all {
            return true;
        }
        let raw = id.as_u64();
        if raw < DENSE_BITMAP_BITS {
            let word = (raw / 64) as usize;
            let bit = raw % 64;
            self.dense_words
                .get(word)
                .is_some_and(|w| (*w & (1_u64 << bit)) != 0)
        } else {
            self.overflow.contains(&raw)
        }
    }

    /// Returns `true` if this bitmap is the `allow_all` wildcard.
    #[must_use]
    pub const fn is_allow_all(&self) -> bool {
        self.allow_all
    }
}

/// Pluggable partitioned vector index supporting traversal-time `live ∩ visible` filtering.
pub trait VectorIndex: Send + Sync {
    /// Returns the active backend kind.
    fn backend_kind(&self) -> VectorBackendKind;

    /// Inserts or replaces an embedding for `event_id` in `partition`.
    fn insert(
        &mut self,
        partition: &EmbeddingTag,
        event_id: EventId,
        vector: &[f32],
    ) -> Result<(), StoreError>;

    /// Searches `partition` with traversal-time filtering against `live_visible`, returning `(EventId, cosine_similarity)`.
    fn search(
        &self,
        partition: &EmbeddingTag,
        query: &[f32],
        k: usize,
        live_visible: &IdBitmap,
    ) -> Result<Vec<(EventId, f32)>, StoreError>;

    /// Removes `event_id` from `partition`; returns `true` if it was present.
    fn remove(&mut self, partition: &EmbeddingTag, event_id: EventId) -> Result<bool, StoreError>;

    /// Removes `event_id` from every partition; returns number of partitions modified.
    fn remove_event(&mut self, event_id: EventId) -> Result<usize, StoreError>;

    /// Rebuilds `partition` from scratch from the given `(EventId, vector)` entries.
    fn rebuild_partition(
        &mut self,
        partition: &EmbeddingTag,
        entries: &[(EventId, Vec<f32>)],
    ) -> Result<(), StoreError>;

    /// Returns the number of live vectors in `partition`.
    fn partition_len(&self, partition: &EmbeddingTag) -> usize;

    /// Returns `true` if `event_id` is currently live in `partition`.
    fn contains(&self, partition: &EmbeddingTag, event_id: EventId) -> bool;

    /// Lists all partitions currently known to this index.
    fn partitions(&self) -> Vec<EmbeddingTag>;
}

/// Instantiates a [`VectorIndex`] from explicit [`VectorBackendConfig`].
#[must_use]
pub fn build_vector_index(config: VectorBackendConfig) -> Box<dyn VectorIndex> {
    match config {
        #[cfg(feature = "usearch")]
        VectorBackendConfig::Usearch { quantization } => {
            Box::new(UsearchVectorIndex::new(quantization))
        }
        VectorBackendConfig::HnswRs => Box::new(HnswRsVectorIndex::new()),
        VectorBackendConfig::BruteForceFlat => Box::new(BruteForceFlatIndex::new()),
    }
}

/// Validates vector dimensionality (`dim_d > 0`, `vector.len() == dim_d`) and finiteness.
pub fn validate_vector(partition: &EmbeddingTag, vector: &[f32]) -> Result<(), StoreError> {
    if partition.dim_d == 0 {
        return Err(StoreError::Backend(format!(
            "invalid embedding tag {:?}: dim_d must be > 0",
            partition
        )));
    }
    if vector.len() != partition.dim_d as usize {
        return Err(StoreError::Backend(format!(
            "vector dimension mismatch for ({}, {}, {}): expected {}, got {}",
            partition.embed_model_ver,
            partition.dim_d,
            partition.prompt_regime_id,
            partition.dim_d,
            vector.len()
        )));
    }
    if vector.iter().any(|v| !v.is_finite()) {
        return Err(StoreError::Backend(
            "vector contains non-finite float value".to_owned(),
        ));
    }
    Ok(())
}

/// Exact cosine similarity in `[-1.0, 1.0]` (`0.0` for zero vectors).
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let xf = f64::from(x);
        let yf = f64::from(y);
        dot += xf * yf;
        norm_a += xf * xf;
        norm_b += yf * yf;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        (dot / denom).clamp(-1.0, 1.0) as f32
    }
}

fn sort_and_truncate(hits: &mut Vec<(EventId, f32)>, k: usize) {
    hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    hits.truncate(k);
}

/// Encodes an `f32` vector slice into little-endian bytes for SQLite persistence.
#[must_use]
pub fn encode_f32_blob(values: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 4);
    for &v in values {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Decodes little-endian bytes stored in `event_vectors` back into `Vec<f32>`.
pub fn decode_f32_blob(bytes: &[u8]) -> Result<Vec<f32>, StoreError> {
    if !bytes.len().is_multiple_of(4) {
        return Err(StoreError::Backend(format!(
            "corrupted vector blob length {} (not multiple of 4)",
            bytes.len()
        )));
    }
    let (chunks, _) = bytes.as_chunks::<4>();
    let mut out = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        out.push(f32::from_le_bytes(*chunk));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// 1. Exact BruteForceFlat fallback
// ---------------------------------------------------------------------------

/// Exact flat cosine-similarity vector index partitioned by [`EmbeddingTag`].
#[derive(Debug, Default)]
pub struct BruteForceFlatIndex {
    partitions: HashMap<EmbeddingTag, BTreeMap<EventId, Vec<f32>>>,
}

impl BruteForceFlatIndex {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl VectorIndex for BruteForceFlatIndex {
    fn backend_kind(&self) -> VectorBackendKind {
        VectorBackendKind::BruteForceFlat
    }

    fn insert(
        &mut self,
        partition: &EmbeddingTag,
        event_id: EventId,
        vector: &[f32],
    ) -> Result<(), StoreError> {
        validate_vector(partition, vector)?;
        self.partitions
            .entry(partition.clone())
            .or_default()
            .insert(event_id, vector.to_vec());
        Ok(())
    }

    fn search(
        &self,
        partition: &EmbeddingTag,
        query: &[f32],
        k: usize,
        live_visible: &IdBitmap,
    ) -> Result<Vec<(EventId, f32)>, StoreError> {
        validate_vector(partition, query)?;
        if k == 0 {
            return Ok(Vec::new());
        }
        let Some(entries) = self.partitions.get(partition) else {
            return Ok(Vec::new());
        };
        let mut hits = Vec::new();
        for (&event_id, vec) in entries {
            if live_visible.contains(event_id) {
                hits.push((event_id, cosine_similarity(query, vec)));
            }
        }
        sort_and_truncate(&mut hits, k);
        Ok(hits)
    }

    fn remove(&mut self, partition: &EmbeddingTag, event_id: EventId) -> Result<bool, StoreError> {
        let Some(entries) = self.partitions.get_mut(partition) else {
            return Ok(false);
        };
        Ok(entries.remove(&event_id).is_some())
    }

    fn remove_event(&mut self, event_id: EventId) -> Result<usize, StoreError> {
        let mut count = 0usize;
        for entries in self.partitions.values_mut() {
            if entries.remove(&event_id).is_some() {
                count += 1;
            }
        }
        Ok(count)
    }

    fn rebuild_partition(
        &mut self,
        partition: &EmbeddingTag,
        entries: &[(EventId, Vec<f32>)],
    ) -> Result<(), StoreError> {
        let mut map = BTreeMap::new();
        for (event_id, vec) in entries {
            validate_vector(partition, vec)?;
            map.insert(*event_id, vec.clone());
        }
        self.partitions.insert(partition.clone(), map);
        Ok(())
    }

    fn partition_len(&self, partition: &EmbeddingTag) -> usize {
        self.partitions.get(partition).map_or(0, BTreeMap::len)
    }

    fn contains(&self, partition: &EmbeddingTag, event_id: EventId) -> bool {
        self.partitions
            .get(partition)
            .is_some_and(|m| m.contains_key(&event_id))
    }

    fn partitions(&self) -> Vec<EmbeddingTag> {
        self.partitions.keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// 2. Pure-Rust hnsw_rs backend
// ---------------------------------------------------------------------------

struct HnswRsPartition {
    hnsw: Hnsw<'static, f32, DistCosine>,
    id_to_slot: BTreeMap<EventId, usize>,
    slot_to_id: Vec<EventId>,
    live_slots: Vec<bool>,
    vectors: BTreeMap<EventId, Vec<f32>>,
    deleted_count: usize,
}

impl HnswRsPartition {
    fn new(expected_elements: usize) -> Self {
        let max_elements = expected_elements.max(128);
        let max_nb_connection = 16;
        let max_layer = 16;
        let ef_construction = 128;
        let mut hnsw = Hnsw::<f32, DistCosine>::new(
            max_nb_connection,
            max_elements,
            max_layer,
            ef_construction,
            DistCosine,
        );
        hnsw.set_keeping_pruned(true);
        Self {
            hnsw,
            id_to_slot: BTreeMap::new(),
            slot_to_id: Vec::new(),
            live_slots: Vec::new(),
            vectors: BTreeMap::new(),
            deleted_count: 0,
        }
    }

    fn insert(&mut self, event_id: EventId, vector: &[f32]) {
        if let Some(&old_slot) = self.id_to_slot.get(&event_id)
            && old_slot < self.live_slots.len()
            && self.live_slots[old_slot]
        {
            self.live_slots[old_slot] = false;
            self.deleted_count += 1;
        }
        let slot = self.slot_to_id.len();
        self.slot_to_id.push(event_id);
        self.live_slots.push(true);
        self.id_to_slot.insert(event_id, slot);
        self.vectors.insert(event_id, vector.to_vec());
        self.hnsw.insert_slice((vector, slot));
    }

    fn remove(&mut self, event_id: EventId) -> bool {
        let Some(slot) = self.id_to_slot.remove(&event_id) else {
            return false;
        };
        if slot < self.live_slots.len() && self.live_slots[slot] {
            self.live_slots[slot] = false;
            self.deleted_count += 1;
        }
        self.vectors.remove(&event_id);
        true
    }
}

struct HnswRsSlotFilter<'a> {
    live_slots: &'a [bool],
    slot_to_id: &'a [EventId],
    live_visible: &'a IdBitmap,
}

impl FilterT for HnswRsSlotFilter<'_> {
    fn hnsw_filter(&self, id: &usize) -> bool {
        let slot = *id;
        slot < self.live_slots.len()
            && self.live_slots[slot]
            && self.live_visible.contains(self.slot_to_id[slot])
    }
}

/// Pure-Rust `hnsw_rs` vector index partitioned by [`EmbeddingTag`] with traversal-time filtering.
pub struct HnswRsVectorIndex {
    partitions: HashMap<EmbeddingTag, HnswRsPartition>,
}

impl Default for HnswRsVectorIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl HnswRsVectorIndex {
    #[must_use]
    pub fn new() -> Self {
        Self {
            partitions: HashMap::new(),
        }
    }

    fn rebuild_partition_from_stored(part: &mut HnswRsPartition) {
        let entries: Vec<(EventId, Vec<f32>)> = part
            .vectors
            .iter()
            .map(|(id, vec)| (*id, vec.clone()))
            .collect();
        let mut fresh = HnswRsPartition::new(entries.len());
        for (id, vec) in entries {
            fresh.insert(id, &vec);
        }
        *part = fresh;
    }
}

impl VectorIndex for HnswRsVectorIndex {
    fn backend_kind(&self) -> VectorBackendKind {
        VectorBackendKind::HnswRs
    }

    fn insert(
        &mut self,
        partition: &EmbeddingTag,
        event_id: EventId,
        vector: &[f32],
    ) -> Result<(), StoreError> {
        validate_vector(partition, vector)?;
        let part = self
            .partitions
            .entry(partition.clone())
            .or_insert_with(|| HnswRsPartition::new(128));
        part.insert(event_id, vector);
        Ok(())
    }

    fn search(
        &self,
        partition: &EmbeddingTag,
        query: &[f32],
        k: usize,
        live_visible: &IdBitmap,
    ) -> Result<Vec<(EventId, f32)>, StoreError> {
        validate_vector(partition, query)?;
        if k == 0 {
            return Ok(Vec::new());
        }
        let Some(part) = self.partitions.get(partition) else {
            return Ok(Vec::new());
        };
        if part.id_to_slot.is_empty() {
            return Ok(Vec::new());
        }

        let filter = HnswRsSlotFilter {
            live_slots: &part.live_slots,
            slot_to_id: &part.slot_to_id,
            live_visible,
        };
        // Expand ef proportionally so low-selectivity (e.g. 1%) traversal finds valid candidates.
        let ef = (k.saturating_mul(64))
            .max(256)
            .min(part.slot_to_id.len().max(k));
        let neighbours = part.hnsw.search_filter(query, k, ef, Some(&filter));

        let mut hits = Vec::with_capacity(neighbours.len());
        for n in neighbours {
            let slot = n.d_id;
            if slot < part.live_slots.len() && part.live_slots[slot] {
                let event_id = part.slot_to_id[slot];
                if live_visible.contains(event_id) {
                    let sim = (1.0_f32 - n.distance).clamp(-1.0, 1.0);
                    hits.push((event_id, sim));
                }
            }
        }
        sort_and_truncate(&mut hits, k);
        Ok(hits)
    }

    fn remove(&mut self, partition: &EmbeddingTag, event_id: EventId) -> Result<bool, StoreError> {
        let Some(part) = self.partitions.get_mut(partition) else {
            return Ok(false);
        };
        let removed = part.remove(event_id);
        if removed && part.deleted_count > part.id_to_slot.len().max(16) {
            Self::rebuild_partition_from_stored(part);
        }
        Ok(removed)
    }

    fn remove_event(&mut self, event_id: EventId) -> Result<usize, StoreError> {
        let mut count = 0usize;
        for part in self.partitions.values_mut() {
            if part.remove(event_id) {
                count += 1;
                if part.deleted_count > part.id_to_slot.len().max(16) {
                    Self::rebuild_partition_from_stored(part);
                }
            }
        }
        Ok(count)
    }

    fn rebuild_partition(
        &mut self,
        partition: &EmbeddingTag,
        entries: &[(EventId, Vec<f32>)],
    ) -> Result<(), StoreError> {
        let mut fresh = HnswRsPartition::new(entries.len());
        for (event_id, vec) in entries {
            validate_vector(partition, vec)?;
            fresh.insert(*event_id, vec);
        }
        self.partitions.insert(partition.clone(), fresh);
        Ok(())
    }

    fn partition_len(&self, partition: &EmbeddingTag) -> usize {
        self.partitions
            .get(partition)
            .map_or(0, |p| p.id_to_slot.len())
    }

    fn contains(&self, partition: &EmbeddingTag, event_id: EventId) -> bool {
        self.partitions
            .get(partition)
            .is_some_and(|p| p.id_to_slot.contains_key(&event_id))
    }

    fn partitions(&self) -> Vec<EmbeddingTag> {
        self.partitions.keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// 3. Primary USearch HNSW backend (i8 / bf16 quantized)
// ---------------------------------------------------------------------------

#[cfg(feature = "usearch")]
struct UsearchPartition {
    index: Index,
    live_ids: BTreeSet<EventId>,
}

#[cfg(feature = "usearch")]
impl UsearchPartition {
    fn new(
        dim_d: usize,
        quantization: UsearchQuantization,
        capacity: usize,
    ) -> Result<Self, StoreError> {
        let scalar_kind = match quantization {
            UsearchQuantization::Bf16 => ScalarKind::BF16,
            UsearchQuantization::I8 => ScalarKind::I8,
        };
        let options = IndexOptions {
            dimensions: dim_d,
            metric: MetricKind::Cos,
            quantization: scalar_kind,
            connectivity: 16,
            expansion_add: 128,
            expansion_search: 256,
            multi: false,
        };
        let index = Index::new(&options)
            .map_err(|err| StoreError::Backend(format!("usearch Index::new failed: {err}")))?;
        index
            .reserve(capacity.max(64))
            .map_err(|err| StoreError::Backend(format!("usearch reserve failed: {err}")))?;
        Ok(Self {
            index,
            live_ids: BTreeSet::new(),
        })
    }

    fn ensure_capacity(&self) -> Result<(), StoreError> {
        let size = self.index.size();
        let cap = self.index.capacity();
        if size >= cap {
            let new_cap = (cap.max(64)).saturating_mul(2);
            self.index
                .reserve(new_cap)
                .map_err(|err| StoreError::Backend(format!("usearch reserve failed: {err}")))?;
        }
        Ok(())
    }
}

/// Primary `usearch` HNSW vector index (`i8`/`bf16` quantized) with traversal-time `filtered_search`.
#[cfg(feature = "usearch")]
pub struct UsearchVectorIndex {
    quantization: UsearchQuantization,
    partitions: HashMap<EmbeddingTag, UsearchPartition>,
}

#[cfg(feature = "usearch")]
impl UsearchVectorIndex {
    #[must_use]
    pub fn new(quantization: UsearchQuantization) -> Self {
        Self {
            quantization,
            partitions: HashMap::new(),
        }
    }
}

#[cfg(feature = "usearch")]
impl VectorIndex for UsearchVectorIndex {
    fn backend_kind(&self) -> VectorBackendKind {
        VectorBackendKind::Usearch(self.quantization)
    }

    fn insert(
        &mut self,
        partition: &EmbeddingTag,
        event_id: EventId,
        vector: &[f32],
    ) -> Result<(), StoreError> {
        validate_vector(partition, vector)?;
        if !self.partitions.contains_key(partition) {
            let created = UsearchPartition::new(partition.dim_d as usize, self.quantization, 64)?;
            self.partitions.insert(partition.clone(), created);
        }
        let part = self
            .partitions
            .get_mut(partition)
            .expect("inserted immediately above");
        let key = event_id.as_u64();
        if part.index.contains(key) {
            part.index
                .remove(key)
                .map_err(|err| StoreError::Backend(format!("usearch remove failed: {err}")))?;
        }
        part.ensure_capacity()?;
        part.index
            .add(key, vector)
            .map_err(|err| StoreError::Backend(format!("usearch add failed: {err}")))?;
        part.live_ids.insert(event_id);
        Ok(())
    }

    fn search(
        &self,
        partition: &EmbeddingTag,
        query: &[f32],
        k: usize,
        live_visible: &IdBitmap,
    ) -> Result<Vec<(EventId, f32)>, StoreError> {
        validate_vector(partition, query)?;
        if k == 0 {
            return Ok(Vec::new());
        }
        let Some(part) = self.partitions.get(partition) else {
            return Ok(Vec::new());
        };
        if part.live_ids.is_empty() {
            return Ok(Vec::new());
        }

        let matches = part
            .index
            .filtered_search(query, k, |key| {
                let id = EventId::new(key);
                part.live_ids.contains(&id) && live_visible.contains(id)
            })
            .map_err(|err| StoreError::Backend(format!("usearch filtered_search failed: {err}")))?;

        let mut hits = Vec::with_capacity(matches.keys.len());
        for (key, dist) in matches.keys.into_iter().zip(matches.distances) {
            let id = EventId::new(key);
            if part.live_ids.contains(&id) && live_visible.contains(id) {
                let sim = (1.0_f32 - dist).clamp(-1.0, 1.0);
                hits.push((id, sim));
            }
        }
        sort_and_truncate(&mut hits, k);
        Ok(hits)
    }

    fn remove(&mut self, partition: &EmbeddingTag, event_id: EventId) -> Result<bool, StoreError> {
        let Some(part) = self.partitions.get_mut(partition) else {
            return Ok(false);
        };
        let existed = part.live_ids.remove(&event_id);
        if part.index.contains(event_id.as_u64()) {
            part.index
                .remove(event_id.as_u64())
                .map_err(|err| StoreError::Backend(format!("usearch remove failed: {err}")))?;
        }
        Ok(existed)
    }

    fn remove_event(&mut self, event_id: EventId) -> Result<usize, StoreError> {
        let mut count = 0usize;
        for part in self.partitions.values_mut() {
            if part.live_ids.remove(&event_id) {
                count += 1;
            }
            if part.index.contains(event_id.as_u64()) {
                part.index
                    .remove(event_id.as_u64())
                    .map_err(|err| StoreError::Backend(format!("usearch remove failed: {err}")))?;
            }
        }
        Ok(count)
    }

    fn rebuild_partition(
        &mut self,
        partition: &EmbeddingTag,
        entries: &[(EventId, Vec<f32>)],
    ) -> Result<(), StoreError> {
        let mut fresh = UsearchPartition::new(
            partition.dim_d as usize,
            self.quantization,
            entries.len().max(64),
        )?;
        for (event_id, vec) in entries {
            validate_vector(partition, vec)?;
            fresh.ensure_capacity()?;
            fresh
                .index
                .add(event_id.as_u64(), vec)
                .map_err(|err| StoreError::Backend(format!("usearch add failed: {err}")))?;
            fresh.live_ids.insert(*event_id);
        }
        self.partitions.insert(partition.clone(), fresh);
        Ok(())
    }

    fn partition_len(&self, partition: &EmbeddingTag) -> usize {
        self.partitions
            .get(partition)
            .map_or(0, |p| p.live_ids.len())
    }

    fn contains(&self, partition: &EmbeddingTag, event_id: EventId) -> bool {
        self.partitions
            .get(partition)
            .is_some_and(|p| p.live_ids.contains(&event_id))
    }

    fn partitions(&self) -> Vec<EmbeddingTag> {
        self.partitions.keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// Petrov vector fade evaluation helpers (B < -3.29)
// ---------------------------------------------------------------------------

/// Default Petrov 30-day vector fade threshold (`design.md` §7, `A-01`).
pub const DEFAULT_FADE_THRESHOLD: f64 = -3.29;

/// Candidate event usage trace passed to vector-fade maintenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FadeCandidate {
    pub event_id: EventId,
    pub trace: ActivationTrace,
}

/// Summary of vectors released by a Petrov fade maintenance pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FadeReport {
    pub evaluated: usize,
    pub released_event_ids: Vec<EventId>,
}

/// Evaluates which candidates fall below `fade_threshold` (`B < fade_threshold`) under Petrov base-level activation.
#[must_use]
pub fn select_faded_events(
    candidates: &[FadeCandidate],
    now_ms: Time,
    decay_d: f64,
    fade_threshold: f64,
) -> Vec<EventId> {
    if !fade_threshold.is_finite() && fade_threshold.is_sign_negative() {
        // Threshold = -infinity is Row 11's reachable neutral value (no vectors faded).
        return Vec::new();
    }
    let mut faded = Vec::new();
    for cand in candidates {
        if cand.trace.count == 0 {
            continue;
        }
        let b = cand.trace.base_level(now_ms, decay_d);
        if b < fade_threshold {
            faded.push(cand.event_id);
        }
    }
    faded
}

//! Deterministic in-memory [`Store`] implementation for unit tests and simulations.

use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Unbounded};

use monoagens_protocol::{EmbeddingTag, EventId, TaggedEmbedding};

use crate::event::{EventDetail, EventKind, EventRecord, LogSeq};
use crate::ports::{SnapshotCheckpoint, Store, StoreError};

/// Deterministic, ordered in-memory [`Store`] storing raw encoded event bytes keyed by [`LogSeq`].
#[derive(Debug, Clone, Default)]
pub struct MemStore {
    raw_logs: BTreeMap<LogSeq, Vec<u8>>,
    id_to_seq: BTreeMap<EventId, LogSeq>,
    vectors: BTreeMap<EventId, Vec<TaggedEmbedding>>,
    snapshots: Vec<SnapshotCheckpoint>,
    next_id: u64,
    next_seq: u64,
    fail_next_append: Option<String>,
    fail_next_append_kind: Option<(EventKind, String)>,
}

impl MemStore {
    /// Creates an empty `MemStore` starting at event id `1` and log sequence `1`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            raw_logs: BTreeMap::new(),
            id_to_seq: BTreeMap::new(),
            vectors: BTreeMap::new(),
            snapshots: Vec::new(),
            next_id: 1,
            next_seq: 1,
            fail_next_append: None,
            fail_next_append_kind: None,
        }
    }

    /// Indexes a tagged embedding vector for `event_id`.
    pub fn insert_vector(&mut self, event_id: EventId, embedding: TaggedEmbedding) {
        self.vectors.entry(event_id).or_default().push(embedding);
    }

    /// Configures the next `append` call to fail with `reason` (consumed once).
    pub fn fail_next_append(&mut self, reason: impl Into<String>) {
        self.fail_next_append = Some(reason.into());
    }

    /// Configures the next `append` call matching `kind` to fail with `reason` (consumed once).
    pub fn fail_next_append_for_kind(&mut self, kind: EventKind, reason: impl Into<String>) {
        self.fail_next_append_kind = Some((kind, reason.into()));
    }

    /// Inserts raw encoded record bytes directly (for legacy upcasting and future-version tests).
    pub fn insert_raw_record_bytes(&mut self, id: EventId, raw_bytes: Vec<u8>) -> LogSeq {
        let seq = LogSeq::new(self.next_seq);
        self.next_seq = self.next_seq.saturating_add(1);
        if id.as_u64() >= self.next_id {
            self.next_id = id.as_u64().saturating_add(1);
        }
        self.raw_logs.insert(seq, raw_bytes);
        self.id_to_seq.insert(id, seq);
        seq
    }

    /// Returns the raw stored bytes at `seq` without decoding or modifying them.
    #[must_use]
    pub fn raw_bytes_at(&self, seq: LogSeq) -> Option<&[u8]> {
        self.raw_logs.get(&seq).map(Vec::as_slice)
    }

    /// Corrupts one byte of the latest stored snapshot payload (for checksum-mismatch tests).
    pub fn corrupt_latest_snapshot_byte(&mut self, index: usize) -> bool {
        let Some(latest) = self.snapshots.last_mut() else {
            return false;
        };
        if latest.bytes.is_empty() {
            latest.bytes.push(0xFF);
        } else {
            let idx = index % latest.bytes.len();
            latest.bytes[idx] ^= 0x5A;
        }
        true
    }

    /// Returns the total number of stored events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.raw_logs.len()
    }

    /// Returns `true` if no events are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.raw_logs.is_empty()
    }

    fn decode_at(&self, seq: LogSeq, raw: &[u8]) -> Result<EventRecord, StoreError> {
        let mut rec = EventRecord::decode_and_upcast(raw)?;
        if rec.seq == LogSeq::ZERO {
            rec.seq = seq;
        }
        Ok(rec)
    }
}

impl Store for MemStore {
    fn append(&mut self, mut record: EventRecord) -> Result<EventId, StoreError> {
        if let Some(reason) = self.fail_next_append.take() {
            return Err(StoreError::AppendFailed(reason));
        }
        if let Some((fail_kind, _)) = &self.fail_next_append_kind
            && *fail_kind == record.kind
        {
            let (_, reason) = self.fail_next_append_kind.take().expect("checked");
            return Err(StoreError::AppendFailed(reason));
        }

        let assigned_id = if record.id.as_u64() == 0 {
            let id = EventId::new(self.next_id);
            self.next_id = self.next_id.saturating_add(1);
            record.id = id;
            id
        } else {
            if self.id_to_seq.contains_key(&record.id) {
                return Err(StoreError::AppendFailed(format!(
                    "duplicate event id {}",
                    record.id
                )));
            }
            if record.id.as_u64() >= self.next_id {
                self.next_id = record.id.as_u64().saturating_add(1);
            }
            record.id
        };

        let assigned_seq = LogSeq::new(self.next_seq);
        self.next_seq = self.next_seq.saturating_add(1);
        record.seq = assigned_seq;
        record.format_version = record.kind.current_version();

        let encoded = record.encode_bytes();
        self.raw_logs.insert(assigned_seq, encoded);
        self.id_to_seq.insert(assigned_id, assigned_seq);
        Ok(assigned_id)
    }

    fn read_in_order(
        &self,
        after: Option<EventId>,
        limit: usize,
    ) -> Result<Vec<EventRecord>, StoreError> {
        let after_seq = match after {
            Some(id) => {
                let Some(&seq) = self.id_to_seq.get(&id) else {
                    return Err(StoreError::NotFound(id));
                };
                Some(seq)
            }
            None => None,
        };
        self.read_after_seq(after_seq, limit)
    }

    fn read_after_seq(
        &self,
        after: Option<LogSeq>,
        limit: usize,
    ) -> Result<Vec<EventRecord>, StoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let iter: Box<dyn Iterator<Item = (&LogSeq, &Vec<u8>)>> = match after {
            Some(cursor) => Box::new(self.raw_logs.range((Excluded(cursor), Unbounded))),
            None => Box::new(self.raw_logs.iter()),
        };
        for (&seq, raw) in iter.take(limit) {
            out.push(self.decode_at(seq, raw)?);
        }
        Ok(out)
    }

    fn get(&self, id: EventId) -> Result<Option<EventRecord>, StoreError> {
        let Some(&seq) = self.id_to_seq.get(&id) else {
            return Ok(None);
        };
        let Some(raw) = self.raw_logs.get(&seq) else {
            return Ok(None);
        };
        Ok(Some(self.decode_at(seq, raw)?))
    }

    fn snapshot(&mut self, checkpoint: SnapshotCheckpoint) -> Result<(), StoreError> {
        self.snapshots.push(checkpoint);
        Ok(())
    }

    fn latest_checkpoint(&self) -> Result<Option<SnapshotCheckpoint>, StoreError> {
        Ok(self.snapshots.last().cloned())
    }

    fn candidates_by_vector(
        &self,
        tag: &EmbeddingTag,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<Vec<(EventId, f32)>, StoreError> {
        if limit == 0 || query_vector.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits: Vec<(EventId, f32)> = Vec::new();
        for (&event_id, embeddings) in &self.vectors {
            let mut best: Option<f32> = None;
            for emb in embeddings {
                if &emb.tag == tag && emb.values.len() == query_vector.len() {
                    let sim = cosine_similarity(query_vector, &emb.values);
                    best = Some(best.map_or(sim, |prev| prev.max(sim)));
                }
            }
            if let Some(score) = best {
                hits.push((event_id, score));
            }
        }
        hits.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        hits.truncate(limit);
        Ok(hits)
    }

    fn candidates_by_text(
        &self,
        query_event_id: EventId,
        limit: usize,
    ) -> Result<Vec<EventId>, StoreError> {
        self.candidates_by_text_partitioned(query_event_id, None, limit)
    }

    fn candidates_by_text_partitioned(
        &self,
        query_event_id: EventId,
        tag: Option<&EmbeddingTag>,
        limit: usize,
    ) -> Result<Vec<EventId>, StoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let Some(query_record) = self.get(query_event_id)? else {
            return Err(StoreError::NotFound(query_event_id));
        };
        let query_tokens = ascii_byte_tokens(query_record.payload.as_bytes());
        if query_tokens.is_empty() {
            return Ok(Vec::new());
        }

        let mut scored: Vec<(EventId, usize)> = Vec::new();
        for (&seq, raw) in &self.raw_logs {
            let record = self.decode_at(seq, raw)?;
            if record.id == query_event_id {
                continue;
            }
            if let Some(req_tag) = tag {
                let mut reported_tags: Vec<&EmbeddingTag> = Vec::new();
                if let Some(embs) = self.vectors.get(&record.id) {
                    for emb in embs {
                        reported_tags.push(&emb.tag);
                    }
                }
                if let Some(EventDetail::PerceptMeta { embeddings, .. }) = &record.detail {
                    for emb in embeddings {
                        reported_tags.push(&emb.tag);
                    }
                }
                if !reported_tags.is_empty() && !reported_tags.into_iter().any(|t| t == req_tag) {
                    continue;
                }
            }
            let cand_tokens = ascii_byte_tokens(record.payload.as_bytes());
            let overlap = cand_tokens
                .into_iter()
                .filter(|tok| query_tokens.contains(tok))
                .count();
            if overlap > 0 {
                scored.push((record.id, overlap));
            }
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(scored.into_iter().take(limit).map(|(id, _)| id).collect())
    }

    fn release_vector(&mut self, event_id: EventId) -> Result<(), StoreError> {
        self.vectors.remove(&event_id);
        Ok(())
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
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
        (dot / denom) as f32
    }
}

fn ascii_byte_tokens(bytes: &[u8]) -> std::collections::BTreeSet<Vec<u8>> {
    bytes
        .split(|b| !b.is_ascii_alphanumeric())
        .filter(|slice| !slice.is_empty())
        .map(|slice| slice.to_ascii_lowercase())
        .collect()
}

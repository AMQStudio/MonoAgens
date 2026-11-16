//! Durable segmented event log, FTS5 text index, partitioned vector indices,
//! `blake3`-checksummed `postcard` snapshots, and atomic `hard_forget` erasure for MonoAgens.

#![forbid(unsafe_code)]

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::sync::{Mutex, RwLock};

use monoagens_core::{
    ActivationTrace, EventDetail, EventKind, EventRecord, LogSeq, SnapshotCheckpoint, Store,
    StoreError,
};
use monoagens_protocol::{
    ActorId, Audience, EmbeddingTag, EventId, OriginActor, PlaceId, TaggedEmbedding, Taint, Time,
};
use rusqlite::Connection;

pub mod forget;
pub mod log;
pub mod schema;
pub mod shards;
pub mod snapshots;
pub mod text;
pub mod vectors;

pub use forget::{ByteScanHit, ForgetReport, ForgetSelector, raw_byte_scan_dir};
pub use schema::{
    HOT_DB_FILENAME, SHARDS_DIR_NAME, SNAPSHOTS_DIR_NAME, STORE_SCHEMA_VERSION, StoreConfig,
    SynchronousMode, UsearchQuantization, VectorBackendConfig,
};
pub use shards::ShardInfo;
#[cfg(feature = "usearch")]
pub use vectors::UsearchVectorIndex;
pub use vectors::{
    BruteForceFlatIndex, DEFAULT_FADE_THRESHOLD, FadeCandidate, FadeReport, HnswRsVectorIndex,
    IdBitmap, VectorBackendKind, VectorIndex, build_vector_index, select_faded_events,
};

use crate::schema::{
    configure_hot_connection, init_hot_schema, map_io_err, map_sqlite_err, open_readonly_shard,
};

/// Durable segmented storage implementing [`monoagens_core::Store`] over WAL-mode `hot.db`,
/// read-only sealed temporal shards, per-shard FTS5, pluggable [`VectorIndex`], and
/// `blake3`-checksummed `postcard` snapshots.
pub struct SegmentedStore {
    config: StoreConfig,
    hot_conn: Mutex<Connection>,
    vectors: RwLock<Box<dyn VectorIndex>>,
}

impl fmt::Debug for SegmentedStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SegmentedStore")
            .field("root_dir", &self.config.root_dir)
            .field("synchronous", &self.config.synchronous)
            .field("vector_backend", &self.config.vector_backend)
            .finish()
    }
}

impl SegmentedStore {
    /// Opens or initializes a segmented store at `config.root_dir`.
    pub fn open(config: StoreConfig) -> Result<Self, StoreError> {
        fs::create_dir_all(&config.root_dir).map_err(|e| map_io_err("create store root dir", e))?;
        fs::create_dir_all(config.shards_dir()).map_err(|e| map_io_err("create shards dir", e))?;
        fs::create_dir_all(config.snapshots_dir())
            .map_err(|e| map_io_err("create snapshots dir", e))?;

        let conn = Connection::open(config.hot_db_path()).map_err(map_sqlite_err)?;
        configure_hot_connection(&conn, config.synchronous)?;
        init_hot_schema(&conn)?;

        let mut vec_index = build_vector_index(config.vector_backend);
        let live_vectors = log::load_all_live_vectors(&conn)?;
        for (eid, tag, values) in live_vectors {
            vec_index.insert(&tag, eid, &values)?;
        }

        Ok(Self {
            config,
            hot_conn: Mutex::new(conn),
            vectors: RwLock::new(vec_index),
        })
    }

    /// Returns the store configuration.
    #[must_use]
    pub fn config(&self) -> &StoreConfig {
        &self.config
    }

    /// Returns the active [`VectorBackendKind`].
    #[must_use]
    pub fn vector_backend_kind(&self) -> VectorBackendKind {
        self.vectors
            .read()
            .expect("vector index lock poisoned")
            .backend_kind()
    }

    /// Indexes a tagged embedding vector for `event_id` in both `hot.db` and the active [`VectorIndex`].
    pub fn insert_vector(
        &mut self,
        event_id: EventId,
        embedding: TaggedEmbedding,
    ) -> Result<(), StoreError> {
        {
            let mut vecs = self.vectors.write().expect("vector index lock poisoned");
            vecs.insert(&embedding.tag, event_id, &embedding.values)?;
        }
        let conn = self.hot_conn.get_mut().expect("hot_conn mutex poisoned");
        log::upsert_hot_vector(conn, event_id, &embedding.tag, &embedding.values)?;
        Ok(())
    }

    /// Searches `tag` using traversal-time `live ∩ visible` bitmap filtering (`R-33`, `R-40`).
    pub fn candidates_by_vector_filtered(
        &self,
        tag: &EmbeddingTag,
        query_vector: &[f32],
        limit: usize,
        live_visible: &IdBitmap,
    ) -> Result<Vec<(EventId, f32)>, StoreError> {
        let vecs = self.vectors.read().expect("vector index lock poisoned");
        vecs.search(tag, query_vector, limit, live_visible)
    }

    /// Seals all `hot.db` events with `seq <= through_seq` into a new read-only temporal shard.
    pub fn seal_through_seq(
        &mut self,
        through_seq: LogSeq,
        now_ms: Time,
    ) -> Result<Option<ShardInfo>, StoreError> {
        let conn = self.hot_conn.get_mut().expect("hot_conn mutex poisoned");
        shards::seal_through_seq(conn, &self.config.shards_dir(), through_seq, now_ms)
    }

    /// Lists all sealed temporal shards in ascending `LogSeq` order.
    pub fn list_shards(&self) -> Result<Vec<ShardInfo>, StoreError> {
        let conn = self.hot_conn.lock().expect("hot_conn mutex poisoned");
        shards::list_shards(&conn)
    }

    /// Atomically executes `hard_forget` across SQLite, FTS5, sealed shards, `VectorIndex`,
    /// attachment references, and `Two-Epoch Replay` snapshots, verifying zero raw byte residue.
    pub fn hard_forget(&mut self, selector: &ForgetSelector) -> Result<ForgetReport, StoreError> {
        let conn = self.hot_conn.get_mut().expect("hot_conn mutex poisoned");
        let vecs = self.vectors.get_mut().expect("vector index lock poisoned");
        forget::execute_hard_forget(&self.config.root_dir, conn, vecs.as_mut(), selector)
    }

    /// Evaluates Petrov base-level activation `B < fade_threshold` on `candidates` and releases
    /// faded vectors while keeping all events intact in the log.
    pub fn fade_vectors(
        &mut self,
        candidates: &[FadeCandidate],
        now_ms: Time,
        decay_d: f64,
        fade_threshold: f64,
    ) -> Result<FadeReport, StoreError> {
        let to_release = select_faded_events(candidates, now_ms, decay_d, fade_threshold);
        for &eid in &to_release {
            self.release_vector(eid)?;
        }
        Ok(FadeReport {
            evaluated: candidates.len(),
            released_event_ids: to_release,
        })
    }

    /// Evaluates Petrov base-level activation from stored event timestamps and `MemoryUsed`
    /// references for all live indexed vectors, releasing any with `B < fade_threshold` while
    /// keeping the events in the log.
    pub fn fade_stale_vectors_by_log_age(
        &mut self,
        now_ms: Time,
        decay_d: f64,
        fade_threshold: f64,
    ) -> Result<FadeReport, StoreError> {
        let candidates = self.collect_live_vector_traces()?;
        self.fade_vectors(&candidates, now_ms, decay_d, fade_threshold)
    }

    /// Inserts raw pre-encoded event bytes verbatim into `hot.db` (for upcasting / future-version tests).
    #[allow(clippy::too_many_arguments)]
    pub fn insert_raw_record_bytes(
        &mut self,
        id: EventId,
        kind: EventKind,
        format_version: u16,
        timestamp_ms: Time,
        actor: &ActorId,
        place: &PlaceId,
        audience: &Audience,
        origin_actor: &OriginActor,
        taint: Taint,
        raw_bytes: Vec<u8>,
    ) -> Result<LogSeq, StoreError> {
        let conn = self.hot_conn.get_mut().expect("hot_conn mutex poisoned");
        log::insert_raw_hot_bytes(
            conn,
            id,
            kind,
            format_version,
            timestamp_ms,
            actor,
            place,
            audience,
            origin_actor,
            taint,
            raw_bytes,
        )
    }

    /// Returns raw stored bytes at `seq` in `hot.db` without decoding or rewriting.
    pub fn raw_bytes_at(&self, seq: LogSeq) -> Result<Option<Vec<u8>>, StoreError> {
        let conn = self.hot_conn.lock().expect("hot_conn mutex poisoned");
        log::raw_bytes_at_hot(&conn, seq)
    }

    /// Returns the current `Two-Epoch Replay` epoch stored in `hot.db`.
    pub fn current_epoch(&self) -> Result<u64, StoreError> {
        let conn = self.hot_conn.lock().expect("hot_conn mutex poisoned");
        log::read_meta_u64(&conn, "epoch")
    }

    /// Checkpoints and truncates `hot.db-wal`.
    pub fn checkpoint_wal(&mut self) -> Result<(), StoreError> {
        let conn = self.hot_conn.get_mut().expect("hot_conn mutex poisoned");
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(map_sqlite_err)
    }

    /// Runs a read-only inspection closure against `hot.db` (for diagnostics and tests).
    pub fn with_hot_connection<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let conn = self.hot_conn.lock().expect("hot_conn mutex poisoned");
        f(&conn)
    }

    fn collect_live_vector_traces(&self) -> Result<Vec<FadeCandidate>, StoreError> {
        let live_ids: BTreeSet<EventId> = {
            let conn = self.hot_conn.lock().expect("hot_conn mutex poisoned");
            let mut stmt = conn
                .prepare("SELECT DISTINCT event_id FROM event_vectors WHERE faded = 0 ORDER BY event_id ASC")
                .map_err(map_sqlite_err)?;
            let rows = stmt
                .query_map([], |row| Ok(EventId::new(row.get::<_, i64>(0)? as u64)))
                .map_err(map_sqlite_err)?;
            let mut ids = BTreeSet::new();
            for item in rows {
                ids.insert(item.map_err(map_sqlite_err)?);
            }
            ids
        };

        if live_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut traces: BTreeMap<EventId, ActivationTrace> = BTreeMap::new();
        let all_events = self.read_after_seq(None, usize::MAX)?;
        for rec in &all_events {
            if live_ids.contains(&rec.id) {
                traces.entry(rec.id).or_default().record(rec.timestamp_ms);
            }
            if rec.kind == EventKind::MemoryUsed {
                for &cause in &rec.caused_by {
                    if live_ids.contains(&cause) {
                        traces.entry(cause).or_default().record(rec.timestamp_ms);
                    }
                }
                if let Some(EventDetail::Verdict {
                    target_event_id, ..
                }) = &rec.detail
                    && live_ids.contains(target_event_id)
                {
                    traces
                        .entry(*target_event_id)
                        .or_default()
                        .record(rec.timestamp_ms);
                }
            }
        }

        Ok(traces
            .into_iter()
            .map(|(event_id, trace)| FadeCandidate { event_id, trace })
            .collect())
    }
}

impl Store for SegmentedStore {
    fn append(&mut self, record: EventRecord) -> Result<EventId, StoreError> {
        let shards_dir = self.config.shards_dir();
        let conn = self.hot_conn.get_mut().expect("hot_conn mutex poisoned");
        let existing_shards = shards::list_shards(conn)?;
        let stored = log::append_hot_record(conn, record, |id| {
            shards::shards_contain_event_id(&existing_shards, &shards_dir, id)
        })?;
        Ok(stored.id)
    }

    fn read_in_order(
        &self,
        after: Option<EventId>,
        limit: usize,
    ) -> Result<Vec<EventRecord>, StoreError> {
        let after_seq = match after {
            Some(id) => {
                let seq = {
                    let conn = self.hot_conn.lock().expect("hot_conn mutex poisoned");
                    if let Some(hot_seq) = log::get_hot_seq_for_id(&conn, id)? {
                        Some(hot_seq)
                    } else {
                        let existing_shards = shards::list_shards(&conn)?;
                        shards::get_shard_seq_for_id(
                            &existing_shards,
                            &self.config.shards_dir(),
                            id,
                        )?
                    }
                };
                let Some(found_seq) = seq else {
                    return Err(StoreError::NotFound(id));
                };
                Some(found_seq)
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

        let (existing_shards, hot_rows) = {
            let conn = self.hot_conn.lock().expect("hot_conn mutex poisoned");
            let existing_shards = shards::list_shards(&conn)?;
            let hot_rows = log::read_hot_after_seq(&conn, after, limit)?;
            (existing_shards, hot_rows)
        };

        let shards_dir = self.config.shards_dir();
        let mut combined: Vec<EventRecord> = Vec::new();
        for shard in &existing_shards {
            if after.is_some_and(|a| shard.max_seq <= a) {
                continue;
            }
            let remaining = limit.saturating_sub(combined.len());
            if remaining == 0 && hot_rows.first().is_none_or(|h| shard.min_seq > h.seq) {
                break;
            }
            let take_limit = if remaining == 0 { limit } else { remaining };
            let mut shard_rows =
                shards::read_shard_after_seq(&shard.path(&shards_dir), after, take_limit)?;
            combined.append(&mut shard_rows);
        }

        combined.extend(hot_rows);
        combined.sort_by_key(|r| r.seq);
        combined.truncate(limit);
        Ok(combined)
    }

    fn get(&self, id: EventId) -> Result<Option<EventRecord>, StoreError> {
        let existing_shards = {
            let conn = self.hot_conn.lock().expect("hot_conn mutex poisoned");
            if let Some(rec) = log::get_hot_event(&conn, id)? {
                return Ok(Some(rec));
            }
            shards::list_shards(&conn)?
        };
        shards::get_from_shards(&existing_shards, &self.config.shards_dir(), id)
    }

    fn snapshot(&mut self, checkpoint: SnapshotCheckpoint) -> Result<(), StoreError> {
        let snapshots_dir = self.config.snapshots_dir();
        let conn = self.hot_conn.get_mut().expect("hot_conn mutex poisoned");
        let snapshot_gen = log::read_meta_u64(conn, "snapshot_gen")?.saturating_add(1);
        snapshots::write_snapshot_atomic(&snapshots_dir, snapshot_gen, &checkpoint)?;
        log::write_meta_u64(conn, "snapshot_gen", snapshot_gen)?;
        Ok(())
    }

    fn latest_checkpoint(&self) -> Result<Option<SnapshotCheckpoint>, StoreError> {
        snapshots::read_latest_snapshot(&self.config.snapshots_dir())
    }

    fn candidates_by_vector(
        &self,
        tag: &EmbeddingTag,
        query_vector: &[f32],
        limit: usize,
    ) -> Result<Vec<(EventId, f32)>, StoreError> {
        let live_only = IdBitmap::allow_all();
        self.candidates_by_vector_filtered(tag, query_vector, limit, &live_only)
    }

    fn candidates_by_text(
        &self,
        query_event_id: EventId,
        limit: usize,
    ) -> Result<Vec<EventId>, StoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let Some(query_record) = self.get(query_event_id)? else {
            return Err(StoreError::NotFound(query_event_id));
        };
        let Some(fts_query) = text::build_fts5_match_query(query_record.payload.as_bytes()) else {
            return Ok(Vec::new());
        };

        let (existing_shards, mut scored) = {
            let conn = self.hot_conn.lock().expect("hot_conn mutex poisoned");
            let existing_shards = shards::list_shards(&conn)?;
            let hot_hits =
                text::query_fts_table(&conn, "hot_fts", &fts_query, query_event_id, limit)?;
            (existing_shards, hot_hits)
        };

        let shards_dir = self.config.shards_dir();
        for shard in &existing_shards {
            let shard_conn = open_readonly_shard(&shard.path(&shards_dir))?;
            let shard_hits =
                text::query_fts_table(&shard_conn, "shard_fts", &fts_query, query_event_id, limit)?;
            scored.extend(shard_hits);
        }

        // Best BM25 match is most negative; deduplicate keeping best score per EventId.
        let mut best_by_id: BTreeMap<EventId, f64> = BTreeMap::new();
        for (eid, score) in scored {
            best_by_id
                .entry(eid)
                .and_modify(|prev| {
                    if score < *prev {
                        *prev = score;
                    }
                })
                .or_insert(score);
        }

        let mut ranked: Vec<(EventId, f64)> = best_by_id.into_iter().collect();
        ranked.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
        Ok(ranked.into_iter().take(limit).map(|(id, _)| id).collect())
    }

    fn release_vector(&mut self, event_id: EventId) -> Result<(), StoreError> {
        {
            let mut vecs = self.vectors.write().expect("vector index lock poisoned");
            let _ = vecs.remove_event(event_id)?;
        }
        let conn = self.hot_conn.get_mut().expect("hot_conn mutex poisoned");
        log::delete_hot_vectors_for_event(conn, event_id)?;
        Ok(())
    }
}

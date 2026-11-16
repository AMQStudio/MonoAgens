//! Atomic `hard_forget` erasure (`R-34`): SQLite rows, FTS5 tokens, vector indices, attachments,
//! clean post-erasure snapshot anchored to `Two-Epoch Replay`, and raw byte scan verification.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_core::{
    AdaptiveWeightsAnchor, CoreState, EpochAnchor, EventDetail, EventKind, EventRecord, LogSeq,
    NumericFeatures, SnapshotCheckpoint, StoreError,
};
use monoagens_protocol::{
    ActorId, Audience, EventId, OpaquePayload, OriginActor, PlaceId, Taint, Time,
};
use rusqlite::{Connection, TransactionBehavior, params};

use crate::log;
use crate::schema::{
    HOT_DB_FILENAME, SHARDS_DIR_NAME, SNAPSHOTS_DIR_NAME, map_io_err, map_sqlite_err,
};
use crate::shards::{self, ShardInfo};
use crate::snapshots;
use crate::text;
use crate::vectors::{VectorIndex, decode_f32_blob};

/// Selector identifying events and/or actors to permanently scrub via `hard_forget`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ForgetSelector {
    pub event_ids: Vec<EventId>,
    pub actor_ids: Vec<ActorId>,
    pub reason: String,
    pub now_ms: Time,
    pub record_tombstone: bool,
}

impl ForgetSelector {
    /// Creates a selector targeting specific [`EventId`]s and recording an epoch tombstone.
    #[must_use]
    pub fn events(event_ids: impl IntoIterator<Item = EventId>, now_ms: Time) -> Self {
        Self {
            event_ids: event_ids.into_iter().collect(),
            actor_ids: Vec::new(),
            reason: "authorized_hard_forget".to_owned(),
            now_ms,
            record_tombstone: true,
        }
    }

    /// Creates a selector targeting all events for specific [`ActorId`]s and recording an epoch tombstone.
    #[must_use]
    pub fn actors(actor_ids: impl IntoIterator<Item = ActorId>, now_ms: Time) -> Self {
        Self {
            event_ids: Vec::new(),
            actor_ids: actor_ids.into_iter().collect(),
            reason: "authorized_hard_forget".to_owned(),
            now_ms,
            record_tombstone: true,
        }
    }
}

/// Occurrence of a forbidden byte sequence found during raw disk inspection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ByteScanHit {
    pub file_path: PathBuf,
    pub offset: usize,
    pub needle_len: usize,
}

/// Audit report returned by `hard_forget`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForgetReport {
    pub epoch: u64,
    pub erased_event_ids: Vec<EventId>,
    pub erased_vectors: usize,
    pub rebuilt_shards: usize,
    pub removed_shards: usize,
    pub snapshot_id: String,
    pub snapshot_cursor: LogSeq,
    pub byte_scan_clean: bool,
}

/// Scans all storage files under `root_dir` (`hot.db`, WAL/SHM, shards, snapshots) for raw byte matches.
pub fn raw_byte_scan_dir(
    root_dir: &Path,
    needles: &[&[u8]],
) -> Result<Vec<ByteScanHit>, StoreError> {
    let active_needles: Vec<&[u8]> = needles.iter().copied().filter(|n| !n.is_empty()).collect();
    if active_needles.is_empty() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for name in [
        HOT_DB_FILENAME,
        "hot.db-wal",
        "hot.db-shm",
        "hot.db-journal",
    ] {
        let p = root_dir.join(name);
        if p.is_file() {
            files.push(p);
        }
    }
    for sub in [SHARDS_DIR_NAME, SNAPSHOTS_DIR_NAME] {
        let dir = root_dir.join(sub);
        if dir.is_dir() {
            for entry in fs::read_dir(&dir).map_err(|e| map_io_err("read store subdir", e))? {
                let entry = entry.map_err(|e| map_io_err("read subdir entry", e))?;
                let p = entry.path();
                if p.is_file() {
                    files.push(p);
                }
            }
        }
    }
    files.sort();

    let mut hits = Vec::new();
    for file_path in files {
        let bytes = fs::read(&file_path).map_err(|e| map_io_err("read file for byte scan", e))?;
        for &needle in &active_needles {
            if let Some(offset) = find_subslice(&bytes, needle) {
                hits.push(ByteScanHit {
                    file_path: file_path.clone(),
                    offset,
                    needle_len: needle.len(),
                });
            }
        }
    }
    Ok(hits)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Executes atomic `hard_forget` across `hot.db`, FTS5, sealed shards, `VectorIndex`, attachments,
/// and `Two-Epoch Replay` snapshots, finishing with a raw byte scan check (`R-34`).
pub fn execute_hard_forget(
    root_dir: &Path,
    hot_conn: &mut Connection,
    vector_index: &mut dyn VectorIndex,
    selector: &ForgetSelector,
) -> Result<ForgetReport, StoreError> {
    let shards_dir = root_dir.join(SHARDS_DIR_NAME);
    let snapshots_dir = root_dir.join(SNAPSHOTS_DIR_NAME);

    let erase_events_seed: BTreeSet<EventId> = selector.event_ids.iter().copied().collect();
    let erase_actors: BTreeSet<ActorId> = selector.actor_ids.iter().cloned().collect();

    if erase_events_seed.is_empty() && erase_actors.is_empty() {
        return Err(StoreError::Backend(
            "hard_forget selector must specify at least one event_id or actor_id".to_owned(),
        ));
    }

    // 1. Capture prior snapshot (if any) before mutating snapshots so adaptive weights survive.
    let prior_checkpoint = snapshots::read_latest_snapshot(&snapshots_dir)?;

    // 2. Identify all matching hot events + payloads, plus surviving hot payloads for false-positive filtering.
    //    `prior_loaded` events are never erasable by `hard_forget` (Decision 20).
    let hot_scan = collect_hot_matching_events(hot_conn, &erase_events_seed, &erase_actors)?;

    let mut all_erased_ids: BTreeSet<EventId> = erase_events_seed
        .into_iter()
        .filter(|eid| !hot_scan.protected_prior_ids.contains(eid))
        .collect();
    let mut erased_payload_candidates: Vec<Vec<u8>> = Vec::new();
    let mut surviving_payload_bytes: Vec<Vec<u8>> = Vec::new();

    for (eid, raw) in hot_scan.erased {
        all_erased_ids.insert(eid);
        extract_payload_if_distinct(&raw, &mut erased_payload_candidates);
    }
    for raw in hot_scan.surviving_raw {
        extract_payload_if_distinct(&raw, &mut surviving_payload_bytes);
    }

    // 3. Scrub sealed shards (rewriting or deleting affected shards cleanly).
    let existing_shards = shards::list_shards(hot_conn)?;
    let mut rebuilt_shards = 0usize;
    let mut removed_shards = 0usize;
    let mut updated_shards: Vec<(String, Option<ShardInfo>)> = Vec::new();

    for shard in &existing_shards {
        let outcome =
            shards::rewrite_shard_without(shard, &shards_dir, &all_erased_ids, &erase_actors)?;
        for pid in &outcome.protected_prior_ids {
            all_erased_ids.remove(pid);
        }
        for (eid, raw) in &outcome.erased_events {
            all_erased_ids.insert(*eid);
            extract_payload_if_distinct(raw, &mut erased_payload_candidates);
        }
        for raw in outcome.surviving_raw {
            extract_payload_if_distinct(&raw, &mut surviving_payload_bytes);
        }
        if !outcome.erased_events.is_empty() {
            match outcome.updated_shard {
                Some(updated) => {
                    rebuilt_shards += 1;
                    updated_shards.push((shard.shard_id.clone(), Some(updated)));
                }
                None => {
                    removed_shards += 1;
                    updated_shards.push((shard.shard_id.clone(), None));
                }
            }
        }
    }

    let erased_vec: Vec<EventId> = all_erased_ids.iter().copied().collect();

    // 4. Atomically scrub hot.db tables + bump epoch inside one SQLite transaction.
    let new_epoch: u64;
    let mut snapshot_gen: u64;
    let erased_vectors: usize;
    {
        let tx = hot_conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_err)?;

        let current_epoch = log::read_meta_u64(&tx, "epoch")?;
        new_epoch = current_epoch.saturating_add(1);
        snapshot_gen = log::read_meta_u64(&tx, "snapshot_gen")?;

        let mut removed_vecs = 0usize;
        for &eid in &erased_vec {
            text::delete_event_text(&tx, "hot_fts", eid)?;
            let vec_deleted = tx
                .execute(
                    "DELETE FROM event_vectors WHERE event_id = ?1",
                    params![eid.as_u64() as i64],
                )
                .map_err(map_sqlite_err)?;
            removed_vecs += vec_deleted;
            tx.execute(
                "DELETE FROM event_attachments WHERE event_id = ?1",
                params![eid.as_u64() as i64],
            )
            .map_err(map_sqlite_err)?;
            tx.execute(
                "DELETE FROM events WHERE event_id = ?1",
                params![eid.as_u64() as i64],
            )
            .map_err(map_sqlite_err)?;
        }
        erased_vectors = removed_vecs;

        for (shard_id, maybe_updated) in &updated_shards {
            match maybe_updated {
                Some(upd) => {
                    tx.execute(
                        "UPDATE shards
                         SET min_seq = ?2, max_seq = ?3, min_ts_ms = ?4, max_ts_ms = ?5, blake3_hex = ?6
                         WHERE shard_id = ?1",
                        params![
                            shard_id,
                            upd.min_seq.as_u64() as i64,
                            upd.max_seq.as_u64() as i64,
                            upd.min_ts_ms.as_millis() as i64,
                            upd.max_ts_ms.as_millis() as i64,
                            &upd.blake3_hex,
                        ],
                    )
                    .map_err(map_sqlite_err)?;
                }
                None => {
                    tx.execute("DELETE FROM shards WHERE shard_id = ?1", params![shard_id])
                        .map_err(map_sqlite_err)?;
                }
            }
        }

        log::write_meta_u64(&tx, "epoch", new_epoch)?;
        tx.commit().map_err(map_sqlite_err)?;
    }

    // 5. Purge/rebuild affected VectorIndex partitions from surviving live vectors in hot.db.
    rebuild_vector_index_after_erasure(hot_conn, vector_index, &erased_vec)?;

    // 6. Scrub SQLite B-tree/freelist/WAL residue completely via secure VACUUM + WAL checkpoint TRUNCATE.
    hot_conn
        .execute_batch(
            "PRAGMA secure_delete = ON;
             VACUUM;
             PRAGMA secure_delete = ON;
             PRAGMA wal_checkpoint(TRUNCATE);",
        )
        .map_err(map_sqlite_err)?;

    // 7. Optionally append the HardForget tombstone event (carrying only IDs/epoch, zero erased payloads).
    let mut cursor_seq = LogSeq::new(log::read_meta_u64(hot_conn, "next_seq")?.saturating_sub(1));
    let mut tombstone_record: Option<EventRecord> = None;
    if selector.record_tombstone {
        let current_shards = shards::list_shards(hot_conn)?;
        let rec = EventRecord::new_root(
            EventKind::HardForget,
            selector.now_ms,
            ActorId::new("core"),
            PlaceId::new("core:hard_forget"),
            Audience::new([ActorId::new("core")]),
            OriginActor::new(ActorId::new("core")),
            Taint::Trusted,
            NumericFeatures::default(),
            OpaquePayload::default(),
            Vec::new(),
            Some(EventDetail::HardForget {
                target_event_ids: erased_vec.clone(),
                target_actor_ids: selector.actor_ids.clone(),
                epoch: new_epoch,
                reason: selector.reason.clone(),
            }),
        );
        let stored = log::append_hot_record(hot_conn, rec, |id| {
            shards::shards_contain_event_id(&current_shards, &shards_dir, id)
        })?;
        cursor_seq = stored.seq;
        tombstone_record = Some(stored);
        hot_conn
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
            .map_err(map_sqlite_err)?;
    }

    // 8. Build clean post-erasure CoreState synced with Two-Epoch Replay anchor & write snapshot atomically.
    let clean_state = build_clean_post_forget_state(
        hot_conn,
        &shards_dir,
        prior_checkpoint.as_ref(),
        &all_erased_ids,
        &erase_actors,
        new_epoch,
        cursor_seq,
        tombstone_record.as_ref(),
    )?;
    let state_bytes = clean_state.encode_snapshot();
    let state_blake3 = *blake3::hash(&state_bytes).as_bytes();
    let snapshot_id = format!("hard_forget_epoch_{new_epoch}_seq_{}", cursor_seq.as_u64());
    let checkpoint = SnapshotCheckpoint {
        snapshot_id: snapshot_id.clone(),
        cursor: cursor_seq,
        epoch: new_epoch,
        bytes: state_bytes,
        blake3: state_blake3,
    };

    snapshot_gen = snapshot_gen.saturating_add(1);
    log::write_meta_u64(hot_conn, "snapshot_gen", snapshot_gen)?;
    let snap_path = snapshots::write_snapshot_atomic(&snapshots_dir, snapshot_gen, &checkpoint)?;
    snapshots::remove_snapshots_except(&snapshots_dir, Some(&snap_path))?;

    hot_conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(map_sqlite_err)?;

    // 9. Raw byte scan across hot.db, WAL/SHM, shards, and snapshots verifying zero residue (R-34).
    let scan_needles: Vec<&[u8]> = erased_payload_candidates
        .iter()
        .filter(|cand| {
            cand.len() >= 4
                && !surviving_payload_bytes
                    .iter()
                    .any(|surv| find_subslice(surv, cand).is_some())
        })
        .map(Vec::as_slice)
        .collect();

    let hits = raw_byte_scan_dir(root_dir, &scan_needles)?;
    if !hits.is_empty() {
        return Err(StoreError::Backend(format!(
            "hard_forget raw byte scan found {} residue hit(s) (first at {:?} offset {})",
            hits.len(),
            hits[0].file_path,
            hits[0].offset
        )));
    }

    Ok(ForgetReport {
        epoch: new_epoch,
        erased_event_ids: erased_vec,
        erased_vectors,
        rebuilt_shards,
        removed_shards,
        snapshot_id,
        snapshot_cursor: cursor_seq,
        byte_scan_clean: true,
    })
}

struct HotMatchScan {
    erased: Vec<(EventId, Vec<u8>)>,
    surviving_raw: Vec<Vec<u8>>,
    protected_prior_ids: BTreeSet<EventId>,
}

fn collect_hot_matching_events(
    hot_conn: &Connection,
    erase_events: &BTreeSet<EventId>,
    erase_actors: &BTreeSet<ActorId>,
) -> Result<HotMatchScan, StoreError> {
    let prior_kind_code = i64::from(EventKind::PriorLoaded.code());
    let mut stmt = hot_conn
        .prepare(
            "SELECT event_id, kind_code, actor, origin_actor, raw_bytes FROM events ORDER BY seq ASC",
        )
        .map_err(map_sqlite_err)?;
    let rows = stmt
        .query_map([], |row| {
            let eid: i64 = row.get(0)?;
            let kind_code: i64 = row.get(1)?;
            let actor: String = row.get(2)?;
            let origin: String = row.get(3)?;
            let raw: Vec<u8> = row.get(4)?;
            Ok((
                EventId::new(eid as u64),
                kind_code,
                ActorId::new(actor),
                ActorId::new(origin),
                raw,
            ))
        })
        .map_err(map_sqlite_err)?;

    let mut erased = Vec::new();
    let mut surviving_raw = Vec::new();
    let mut protected_prior_ids = BTreeSet::new();
    for item in rows {
        let (eid, kind_code, actor, origin, raw) = item.map_err(map_sqlite_err)?;
        if kind_code == prior_kind_code {
            protected_prior_ids.insert(eid);
            surviving_raw.push(raw);
            continue;
        }
        if erase_events.contains(&eid)
            || erase_actors.contains(&actor)
            || erase_actors.contains(&origin)
        {
            erased.push((eid, raw));
        } else {
            surviving_raw.push(raw);
        }
    }
    Ok(HotMatchScan {
        erased,
        surviving_raw,
        protected_prior_ids,
    })
}

fn extract_payload_if_distinct(raw_record_bytes: &[u8], out: &mut Vec<Vec<u8>>) {
    if let Ok(rec) = EventRecord::decode_and_upcast(raw_record_bytes)
        && !rec.payload.is_empty()
    {
        out.push(rec.payload.0);
    }
}

fn rebuild_vector_index_after_erasure(
    hot_conn: &Connection,
    vector_index: &mut dyn VectorIndex,
    erased_ids: &[EventId],
) -> Result<(), StoreError> {
    for &eid in erased_ids {
        let _ = vector_index.remove_event(eid)?;
    }
    let partitions = vector_index.partitions();
    for tag in partitions {
        let mut stmt = hot_conn
            .prepare(
                "SELECT event_id, vector_blob FROM event_vectors
                 WHERE embed_model_ver = ?1 AND dim_d = ?2 AND prompt_regime_id = ?3 AND faded = 0
                 ORDER BY event_id ASC",
            )
            .map_err(map_sqlite_err)?;
        let rows = stmt
            .query_map(
                params![
                    &tag.embed_model_ver,
                    i64::from(tag.dim_d),
                    &tag.prompt_regime_id
                ],
                |row| {
                    let eid: i64 = row.get(0)?;
                    let blob: Vec<u8> = row.get(1)?;
                    Ok((EventId::new(eid as u64), blob))
                },
            )
            .map_err(map_sqlite_err)?;

        let mut entries = Vec::new();
        for item in rows {
            let (eid, blob) = item.map_err(map_sqlite_err)?;
            entries.push((eid, decode_f32_blob(&blob)?));
        }
        vector_index.rebuild_partition(&tag, &entries)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_clean_post_forget_state(
    hot_conn: &Connection,
    shards_dir: &Path,
    prior_checkpoint: Option<&SnapshotCheckpoint>,
    erased_events: &BTreeSet<EventId>,
    erased_actors: &BTreeSet<ActorId>,
    new_epoch: u64,
    cursor_seq: LogSeq,
    tombstone: Option<&EventRecord>,
) -> Result<CoreState, StoreError> {
    let mut state = CoreState::new();

    // Preserve prior adaptive weight snapshot anchor if one existed before erasure (R-34).
    let preserved_weights: Option<AdaptiveWeightsAnchor> = prior_checkpoint
        .and_then(|ckpt| CoreState::decode_snapshot(&ckpt.bytes).ok())
        .map(|prev| {
            prev.epoch_anchor
                .frozen_weights
                .unwrap_or(prev.adaptive_weights)
        });

    let all_shards = shards::list_shards(hot_conn)?;
    for shard in &all_shards {
        let shard_events = shards::read_shard_after_seq(&shard.path(shards_dir), None, usize::MAX)?;
        for rec in shard_events {
            if rec.kind == EventKind::PriorLoaded
                || (!erased_events.contains(&rec.id)
                    && !erased_actors.contains(&rec.actor)
                    && !erased_actors.contains(rec.origin_actor.actor_id()))
            {
                state.apply_event(&rec);
            }
        }
    }

    let hot_events = log::read_hot_after_seq(hot_conn, None, usize::MAX)?;
    for rec in hot_events {
        if let Some(tomb) = tombstone
            && rec.id == tomb.id
        {
            continue;
        }
        if rec.kind == EventKind::PriorLoaded
            || (!erased_events.contains(&rec.id)
                && !erased_actors.contains(&rec.actor)
                && !erased_actors.contains(rec.origin_actor.actor_id()))
        {
            state.apply_event(&rec);
        }
    }

    if let Some(tomb) = tombstone {
        state.apply_event(tomb);
    }

    state.epoch = new_epoch;
    if cursor_seq > state.last_seq {
        state.last_seq = cursor_seq;
    }
    let anchor_weights = preserved_weights.unwrap_or(state.adaptive_weights);
    state.adaptive_weights = anchor_weights;
    state.epoch_anchor = EpochAnchor {
        epoch: new_epoch,
        anchor_seq: cursor_seq,
        frozen: true,
        frozen_weights: Some(anchor_weights),
    };

    for actor in erased_actors {
        state.actor_facts.remove(actor);
        state.actor_exchanges.remove(actor);
    }

    Ok(state)
}

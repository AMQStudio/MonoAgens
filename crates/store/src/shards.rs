//! Read-only sealed temporal SQLite shards with per-shard FTS5 (`'secure-delete'=1`).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_core::{EventRecord, LogSeq, StoreError};
use monoagens_protocol::{ActorId, EventId, Time};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::schema::{init_shard_schema, map_io_err, map_sqlite_err, open_readonly_shard};
use crate::text;

/// Metadata describing one sealed read-only temporal shard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShardInfo {
    pub shard_id: String,
    pub min_seq: LogSeq,
    pub max_seq: LogSeq,
    pub min_ts_ms: Time,
    pub max_ts_ms: Time,
    pub file_name: String,
    pub blake3_hex: String,
    pub created_at_ms: Time,
}

impl ShardInfo {
    /// Returns the filesystem path of this shard inside `shards_dir`.
    #[must_use]
    pub fn path(&self, shards_dir: &Path) -> PathBuf {
        shards_dir.join(&self.file_name)
    }
}

struct RawEventRow {
    seq: i64,
    event_id: i64,
    kind_code: i64,
    kind_name: String,
    format_version: i64,
    timestamp_ms: i64,
    actor: String,
    place: String,
    audience_json: String,
    origin_actor: String,
    taint: i64,
    raw_bytes: Vec<u8>,
}

struct RawAttachmentRow {
    event_id: i64,
    handle_id: String,
    modality_slot: String,
    byte_len: i64,
    expires_at_ms: i64,
    embed_model_ver: String,
}

/// Lists all sealed shards registered in `hot.db`, ordered by ascending `min_seq`.
pub fn list_shards(hot_conn: &Connection) -> Result<Vec<ShardInfo>, StoreError> {
    let mut stmt = hot_conn
        .prepare(
            "SELECT shard_id, min_seq, max_seq, min_ts_ms, max_ts_ms, file_name, blake3_hex, created_at_ms
             FROM shards ORDER BY min_seq ASC",
        )
        .map_err(map_sqlite_err)?;

    let rows = stmt
        .query_map([], |row| {
            Ok(ShardInfo {
                shard_id: row.get(0)?,
                min_seq: LogSeq::new(row.get::<_, i64>(1)? as u64),
                max_seq: LogSeq::new(row.get::<_, i64>(2)? as u64),
                min_ts_ms: Time::from_millis(row.get::<_, i64>(3)? as u64),
                max_ts_ms: Time::from_millis(row.get::<_, i64>(4)? as u64),
                file_name: row.get(5)?,
                blake3_hex: row.get(6)?,
                created_at_ms: Time::from_millis(row.get::<_, i64>(7)? as u64),
            })
        })
        .map_err(map_sqlite_err)?;

    let mut out = Vec::new();
    for item in rows {
        out.push(item.map_err(map_sqlite_err)?);
    }
    Ok(out)
}

/// Returns `true` if `event_id` exists in any sealed shard.
pub fn shards_contain_event_id(
    shards: &[ShardInfo],
    shards_dir: &Path,
    event_id: EventId,
) -> Result<bool, StoreError> {
    for shard in shards {
        let conn = open_readonly_shard(&shard.path(shards_dir))?;
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM events WHERE event_id = ?1 LIMIT 1",
                params![event_id.as_u64() as i64],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sqlite_err)?;
        if found.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Returns the `LogSeq` for `id` across sealed shards, if present.
pub fn get_shard_seq_for_id(
    shards: &[ShardInfo],
    shards_dir: &Path,
    id: EventId,
) -> Result<Option<LogSeq>, StoreError> {
    for shard in shards {
        let conn = open_readonly_shard(&shard.path(shards_dir))?;
        let found: Option<i64> = conn
            .query_row(
                "SELECT seq FROM events WHERE event_id = ?1 LIMIT 1",
                params![id.as_u64() as i64],
                |r| r.get(0),
            )
            .optional()
            .map_err(map_sqlite_err)?;
        if let Some(seq) = found {
            return Ok(Some(LogSeq::new(seq as u64)));
        }
    }
    Ok(None)
}

/// Fetches a single event by `EventId` from sealed shards, upcasting older formats in memory.
pub fn get_from_shards(
    shards: &[ShardInfo],
    shards_dir: &Path,
    id: EventId,
) -> Result<Option<EventRecord>, StoreError> {
    for shard in shards {
        let conn = open_readonly_shard(&shard.path(shards_dir))?;
        let found: Option<(i64, Vec<u8>)> = conn
            .query_row(
                "SELECT seq, raw_bytes FROM events WHERE event_id = ?1 LIMIT 1",
                params![id.as_u64() as i64],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(map_sqlite_err)?;
        if let Some((seq_raw, raw)) = found {
            let mut rec = EventRecord::decode_and_upcast(&raw)?;
            if rec.seq == LogSeq::ZERO {
                rec.seq = LogSeq::new(seq_raw as u64);
            }
            return Ok(Some(rec));
        }
    }
    Ok(None)
}

/// Reads up to `limit` events strictly after `after` from a single sealed read-only shard.
pub fn read_shard_after_seq(
    shard_path: &Path,
    after: Option<LogSeq>,
    limit: usize,
) -> Result<Vec<EventRecord>, StoreError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let conn = open_readonly_shard(shard_path)?;
    let after_val = after.map_or(0_i64, |s| s.as_u64() as i64);
    let mut stmt = conn
        .prepare("SELECT seq, raw_bytes FROM events WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2")
        .map_err(map_sqlite_err)?;
    let rows = stmt
        .query_map(params![after_val, limit as i64], |row| {
            let seq: i64 = row.get(0)?;
            let raw: Vec<u8> = row.get(1)?;
            Ok((LogSeq::new(seq as u64), raw))
        })
        .map_err(map_sqlite_err)?;

    let mut out = Vec::new();
    for item in rows {
        let (seq, raw) = item.map_err(map_sqlite_err)?;
        let mut rec = EventRecord::decode_and_upcast(&raw)?;
        if rec.seq == LogSeq::ZERO {
            rec.seq = seq;
        }
        out.push(rec);
    }
    Ok(out)
}

/// Seals all `hot.db` events with `seq <= through_seq` into a new read-only temporal shard file.
pub fn seal_through_seq(
    hot_conn: &mut Connection,
    shards_dir: &Path,
    through_seq: LogSeq,
    now_ms: Time,
) -> Result<Option<ShardInfo>, StoreError> {
    if through_seq == LogSeq::ZERO {
        return Ok(None);
    }
    fs::create_dir_all(shards_dir).map_err(|e| map_io_err("create shards dir", e))?;

    let rows = collect_hot_rows_through_seq(hot_conn, through_seq)?;
    if rows.is_empty() {
        return Ok(None);
    }
    let attachments = collect_hot_attachments_through_seq(hot_conn, through_seq)?;

    let min_seq = LogSeq::new(rows.first().expect("non-empty").seq as u64);
    let max_seq = LogSeq::new(rows.last().expect("non-empty").seq as u64);
    let min_ts_ms = Time::from_millis(
        rows.iter()
            .map(|r| r.timestamp_ms as u64)
            .min()
            .unwrap_or(0),
    );
    let max_ts_ms = Time::from_millis(
        rows.iter()
            .map(|r| r.timestamp_ms as u64)
            .max()
            .unwrap_or(0),
    );

    let shard_id = format!("shard_{:016x}_{:016x}", min_seq.as_u64(), max_seq.as_u64());
    let file_name = format!("{shard_id}.db");
    let final_path = shards_dir.join(&file_name);
    let tmp_path = shards_dir.join(format!(".{shard_id}.{}.tmp", std::process::id()));

    if tmp_path.exists() {
        let _ = fs::remove_file(&tmp_path);
    }

    let blake3_hex = write_shard_db_file(&tmp_path, &shard_id, &rows, &attachments)?;
    set_readonly(&tmp_path)?;
    fs::rename(&tmp_path, &final_path).map_err(|e| map_io_err("rename sealed shard", e))?;

    let info = ShardInfo {
        shard_id: shard_id.clone(),
        min_seq,
        max_seq,
        min_ts_ms,
        max_ts_ms,
        file_name,
        blake3_hex,
        created_at_ms: now_ms,
    };

    let tx = hot_conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_err)?;

    tx.execute(
        "INSERT INTO shards (
            shard_id, min_seq, max_seq, min_ts_ms, max_ts_ms, file_name, blake3_hex, created_at_ms
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            &info.shard_id,
            info.min_seq.as_u64() as i64,
            info.max_seq.as_u64() as i64,
            info.min_ts_ms.as_millis() as i64,
            info.max_ts_ms.as_millis() as i64,
            &info.file_name,
            &info.blake3_hex,
            info.created_at_ms.as_millis() as i64,
        ],
    )
    .map_err(map_sqlite_err)?;

    for row in &rows {
        text::delete_event_text(&tx, "hot_fts", EventId::new(row.event_id as u64))?;
    }
    tx.execute(
        "DELETE FROM event_attachments WHERE event_id IN (SELECT event_id FROM events WHERE seq <= ?1)",
        params![through_seq.as_u64() as i64],
    )
    .map_err(map_sqlite_err)?;
    tx.execute(
        "DELETE FROM events WHERE seq <= ?1",
        params![through_seq.as_u64() as i64],
    )
    .map_err(map_sqlite_err)?;
    tx.commit().map_err(map_sqlite_err)?;

    hot_conn
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(map_sqlite_err)?;

    Ok(Some(info))
}

/// Verifies a sealed shard's on-disk `blake3` checksum.
pub fn verify_shard_checksum(shard: &ShardInfo, shards_dir: &Path) -> Result<(), StoreError> {
    let bytes = fs::read(shard.path(shards_dir)).map_err(|e| map_io_err("read shard file", e))?;
    let actual = blake3::hash(&bytes).to_hex().to_string();
    if actual != shard.blake3_hex {
        return Err(StoreError::Backend(format!(
            "shard '{}' blake3 checksum mismatch: expected {}, got {}",
            shard.shard_id, shard.blake3_hex, actual
        )));
    }
    Ok(())
}

/// Outcome of rewriting a sealed shard during `hard_forget`.
pub struct ShardRewriteOutcome {
    pub updated_shard: Option<ShardInfo>,
    pub erased_events: Vec<(EventId, Vec<u8>)>,
    pub surviving_raw: Vec<Vec<u8>>,
    pub protected_prior_ids: Vec<EventId>,
}

/// Rebuilds a sealed shard without erased events/actors during `hard_forget`, returning `None` if empty.
pub fn rewrite_shard_without(
    shard: &ShardInfo,
    shards_dir: &Path,
    erase_events: &BTreeSet<EventId>,
    erase_actors: &BTreeSet<ActorId>,
) -> Result<ShardRewriteOutcome, StoreError> {
    let shard_path = shard.path(shards_dir);
    let conn = open_readonly_shard(&shard_path)?;

    let mut stmt = conn
        .prepare(
            "SELECT seq, event_id, kind_code, kind_name, format_version, timestamp_ms,
                    actor, place, audience_json, origin_actor, taint, raw_bytes
             FROM events ORDER BY seq ASC",
        )
        .map_err(map_sqlite_err)?;
    let rows = stmt
        .query_map([], |row| {
            Ok(RawEventRow {
                seq: row.get(0)?,
                event_id: row.get(1)?,
                kind_code: row.get(2)?,
                kind_name: row.get(3)?,
                format_version: row.get(4)?,
                timestamp_ms: row.get(5)?,
                actor: row.get(6)?,
                place: row.get(7)?,
                audience_json: row.get(8)?,
                origin_actor: row.get(9)?,
                taint: row.get(10)?,
                raw_bytes: row.get(11)?,
            })
        })
        .map_err(map_sqlite_err)?;

    let prior_kind_code = i64::from(monoagens_core::EventKind::PriorLoaded.code());
    let mut surviving_rows = Vec::new();
    let mut erased_events = Vec::new();
    let mut surviving_raw = Vec::new();
    let mut protected_prior_ids = Vec::new();
    let mut matched_any = false;

    for item in rows {
        let row = item.map_err(map_sqlite_err)?;
        let eid = EventId::new(row.event_id as u64);
        if row.kind_code == prior_kind_code {
            protected_prior_ids.push(eid);
            surviving_raw.push(row.raw_bytes.clone());
            surviving_rows.push(row);
            continue;
        }
        let actor = ActorId::new(row.actor.clone());
        let origin = ActorId::new(row.origin_actor.clone());
        if erase_events.contains(&eid)
            || erase_actors.contains(&actor)
            || erase_actors.contains(&origin)
        {
            matched_any = true;
            erased_events.push((eid, row.raw_bytes));
        } else {
            surviving_raw.push(row.raw_bytes.clone());
            surviving_rows.push(row);
        }
    }

    if !matched_any {
        return Ok(ShardRewriteOutcome {
            updated_shard: Some(shard.clone()),
            erased_events,
            surviving_raw,
            protected_prior_ids,
        });
    }

    let surviving_ids: BTreeSet<i64> = surviving_rows.iter().map(|r| r.event_id).collect();
    let mut att_stmt = conn
        .prepare(
            "SELECT event_id, handle_id, modality_slot, byte_len, expires_at_ms, embed_model_ver
             FROM event_attachments",
        )
        .map_err(map_sqlite_err)?;
    let att_rows = att_stmt
        .query_map([], |row| {
            Ok(RawAttachmentRow {
                event_id: row.get(0)?,
                handle_id: row.get(1)?,
                modality_slot: row.get(2)?,
                byte_len: row.get(3)?,
                expires_at_ms: row.get(4)?,
                embed_model_ver: row.get(5)?,
            })
        })
        .map_err(map_sqlite_err)?;
    let mut surviving_atts = Vec::new();
    for item in att_rows {
        let att = item.map_err(map_sqlite_err)?;
        if surviving_ids.contains(&att.event_id) {
            surviving_atts.push(att);
        }
    }
    drop(att_stmt);
    drop(stmt);
    drop(conn);

    if surviving_rows.is_empty() {
        clear_readonly_and_remove(&shard_path)?;
        return Ok(ShardRewriteOutcome {
            updated_shard: None,
            erased_events,
            surviving_raw,
            protected_prior_ids,
        });
    }

    let tmp_path = shards_dir.join(format!(
        ".rewrite_{}.{}.tmp",
        shard.shard_id,
        std::process::id()
    ));
    if tmp_path.exists() {
        let _ = clear_readonly_and_remove(&tmp_path);
    }

    let blake3_hex =
        write_shard_db_file(&tmp_path, &shard.shard_id, &surviving_rows, &surviving_atts)?;
    set_readonly(&tmp_path)?;
    fs::rename(&tmp_path, &shard_path).map_err(|e| map_io_err("replace rewritten shard", e))?;

    let mut updated = shard.clone();
    updated.min_seq = LogSeq::new(surviving_rows.first().expect("non-empty").seq as u64);
    updated.max_seq = LogSeq::new(surviving_rows.last().expect("non-empty").seq as u64);
    updated.min_ts_ms = Time::from_millis(
        surviving_rows
            .iter()
            .map(|r| r.timestamp_ms as u64)
            .min()
            .unwrap_or(0),
    );
    updated.max_ts_ms = Time::from_millis(
        surviving_rows
            .iter()
            .map(|r| r.timestamp_ms as u64)
            .max()
            .unwrap_or(0),
    );
    updated.blake3_hex = blake3_hex;

    Ok(ShardRewriteOutcome {
        updated_shard: Some(updated),
        erased_events,
        surviving_raw,
        protected_prior_ids,
    })
}

fn collect_hot_rows_through_seq(
    hot_conn: &Connection,
    through_seq: LogSeq,
) -> Result<Vec<RawEventRow>, StoreError> {
    let mut stmt = hot_conn
        .prepare(
            "SELECT seq, event_id, kind_code, kind_name, format_version, timestamp_ms,
                    actor, place, audience_json, origin_actor, taint, raw_bytes
             FROM events WHERE seq <= ?1 ORDER BY seq ASC",
        )
        .map_err(map_sqlite_err)?;
    let rows = stmt
        .query_map(params![through_seq.as_u64() as i64], |row| {
            Ok(RawEventRow {
                seq: row.get(0)?,
                event_id: row.get(1)?,
                kind_code: row.get(2)?,
                kind_name: row.get(3)?,
                format_version: row.get(4)?,
                timestamp_ms: row.get(5)?,
                actor: row.get(6)?,
                place: row.get(7)?,
                audience_json: row.get(8)?,
                origin_actor: row.get(9)?,
                taint: row.get(10)?,
                raw_bytes: row.get(11)?,
            })
        })
        .map_err(map_sqlite_err)?;

    let mut out = Vec::new();
    for item in rows {
        out.push(item.map_err(map_sqlite_err)?);
    }
    Ok(out)
}

fn collect_hot_attachments_through_seq(
    hot_conn: &Connection,
    through_seq: LogSeq,
) -> Result<Vec<RawAttachmentRow>, StoreError> {
    let mut stmt = hot_conn
        .prepare(
            "SELECT a.event_id, a.handle_id, a.modality_slot, a.byte_len, a.expires_at_ms, a.embed_model_ver
             FROM event_attachments a
             INNER JOIN events e ON e.event_id = a.event_id
             WHERE e.seq <= ?1",
        )
        .map_err(map_sqlite_err)?;
    let rows = stmt
        .query_map(params![through_seq.as_u64() as i64], |row| {
            Ok(RawAttachmentRow {
                event_id: row.get(0)?,
                handle_id: row.get(1)?,
                modality_slot: row.get(2)?,
                byte_len: row.get(3)?,
                expires_at_ms: row.get(4)?,
                embed_model_ver: row.get(5)?,
            })
        })
        .map_err(map_sqlite_err)?;

    let mut out = Vec::new();
    for item in rows {
        out.push(item.map_err(map_sqlite_err)?);
    }
    Ok(out)
}

fn write_shard_db_file(
    path: &Path,
    shard_id: &str,
    rows: &[RawEventRow],
    attachments: &[RawAttachmentRow],
) -> Result<String, StoreError> {
    {
        let mut conn = Connection::open(path).map_err(map_sqlite_err)?;
        init_shard_schema(&conn)?;

        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(map_sqlite_err)?;

        tx.execute(
            "INSERT INTO shard_meta (key, int_val, text_val) VALUES ('shard_id', NULL, ?1)",
            params![shard_id],
        )
        .map_err(map_sqlite_err)?;

        for row in rows {
            tx.execute(
                "INSERT INTO events (
                    seq, event_id, kind_code, kind_name, format_version,
                    timestamp_ms, actor, place, audience_json, origin_actor, taint, raw_bytes
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    row.seq,
                    row.event_id,
                    row.kind_code,
                    &row.kind_name,
                    row.format_version,
                    row.timestamp_ms,
                    &row.actor,
                    &row.place,
                    &row.audience_json,
                    &row.origin_actor,
                    row.taint,
                    &row.raw_bytes,
                ],
            )
            .map_err(map_sqlite_err)?;

            if let Ok(decoded) = EventRecord::decode_and_upcast(&row.raw_bytes) {
                text::index_event_text(
                    &tx,
                    "shard_fts",
                    EventId::new(row.event_id as u64),
                    decoded.payload.as_bytes(),
                )?;
            }
        }

        for att in attachments {
            tx.execute(
                "INSERT INTO event_attachments (
                    event_id, handle_id, modality_slot, byte_len, expires_at_ms, embed_model_ver
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    att.event_id,
                    &att.handle_id,
                    &att.modality_slot,
                    att.byte_len,
                    att.expires_at_ms,
                    &att.embed_model_ver,
                ],
            )
            .map_err(map_sqlite_err)?;
        }

        tx.commit().map_err(map_sqlite_err)?;
    }

    let bytes = fs::read(path).map_err(|e| map_io_err("hash sealed shard", e))?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn set_readonly(path: &Path) -> Result<(), StoreError> {
    let mut perms = fs::metadata(path)
        .map_err(|e| map_io_err("stat shard", e))?
        .permissions();
    perms.set_readonly(true);
    fs::set_permissions(path, perms).map_err(|e| map_io_err("set shard readonly", e))?;
    Ok(())
}

fn clear_readonly_and_remove(path: &Path) -> Result<(), StoreError> {
    if path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(path)
                .map_err(|e| map_io_err("stat shard for removal", e))?
                .permissions();
            let mode = perms.mode();
            if mode & 0o200 == 0 {
                perms.set_mode(mode | 0o200);
                let _ = fs::set_permissions(path, perms);
            }
        }
        fs::remove_file(path).map_err(|e| map_io_err("remove shard file", e))?;
    }
    Ok(())
}

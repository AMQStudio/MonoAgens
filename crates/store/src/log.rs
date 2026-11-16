//! WAL-mode `hot.db` transactional append and ordered scans.

use monoagens_core::{EventKind, EventRecord, LogSeq, StoreError};
use monoagens_protocol::{
    ActorId, Audience, EmbeddingTag, EventId, OriginActor, PlaceId, Taint, Time,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

use crate::schema::map_sqlite_err;
use crate::text;
use crate::vectors::{decode_f32_blob, encode_f32_blob};

/// Reads an integer metadata value from `store_meta`.
pub fn read_meta_u64(conn: &Connection, key: &str) -> Result<u64, StoreError> {
    let val: Option<i64> = conn
        .query_row(
            "SELECT int_val FROM store_meta WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite_err)?;
    Ok(val.unwrap_or(0) as u64)
}

/// Updates an integer metadata value in `store_meta`.
pub fn write_meta_u64(conn: &Connection, key: &str, value: u64) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO store_meta (key, int_val, text_val) VALUES (?1, ?2, NULL)
         ON CONFLICT(key) DO UPDATE SET int_val = excluded.int_val",
        params![key, value as i64],
    )
    .map_err(map_sqlite_err)?;
    Ok(())
}

/// Returns `true` if `event_id` already exists in `hot.db`.
pub fn hot_contains_event_id(conn: &Connection, event_id: EventId) -> Result<bool, StoreError> {
    let found: Option<i64> = conn
        .query_row(
            "SELECT 1 FROM events WHERE event_id = ?1 LIMIT 1",
            params![event_id.as_u64() as i64],
            |row| row.get(0),
        )
        .optional()
        .map_err(map_sqlite_err)?;
    Ok(found.is_some())
}

/// Appends an [`EventRecord`] transactionally to `hot.db`, assigning monotonic `LogSeq` and `EventId`.
///
/// `id_exists_in_shards` must be true if `record.id` already exists in a sealed shard.
pub fn append_hot_record<F>(
    conn: &mut Connection,
    mut record: EventRecord,
    id_exists_in_shards: F,
) -> Result<EventRecord, StoreError>
where
    F: Fn(EventId) -> Result<bool, StoreError>,
{
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_err)?;

    let mut next_seq = read_meta_u64(&tx, "next_seq")?.max(1);
    let mut next_id = read_meta_u64(&tx, "next_event_id")?.max(1);

    let assigned_id = if record.id.as_u64() == 0 {
        let id = EventId::new(next_id);
        next_id = next_id.saturating_add(1);
        record.id = id;
        id
    } else {
        if hot_contains_event_id(&tx, record.id)? || id_exists_in_shards(record.id)? {
            return Err(StoreError::AppendFailed(format!(
                "duplicate event id {}",
                record.id
            )));
        }
        if record.id.as_u64() >= next_id {
            next_id = record.id.as_u64().saturating_add(1);
        }
        record.id
    };

    let assigned_seq = LogSeq::new(next_seq);
    next_seq = next_seq.saturating_add(1);
    record.seq = assigned_seq;
    record.format_version = record.kind.current_version();

    let raw_bytes = record.encode_bytes();
    let audience_json = encode_audience_list(&record.audience);

    tx.execute(
        "INSERT INTO events (
            seq, event_id, kind_code, kind_name, format_version,
            timestamp_ms, actor, place, audience_json, origin_actor, taint, raw_bytes
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            assigned_seq.as_u64() as i64,
            assigned_id.as_u64() as i64,
            i64::from(record.kind.code()),
            record.kind.as_str(),
            i64::from(record.format_version),
            record.timestamp_ms.as_millis() as i64,
            record.actor.as_str(),
            record.place.as_str(),
            audience_json,
            record.origin_actor.actor_id().as_str(),
            taint_code(record.taint),
            raw_bytes,
        ],
    )
    .map_err(|err| StoreError::AppendFailed(format!("sqlite insert failed: {err}")))?;

    text::index_event_text(&tx, "hot_fts", assigned_id, record.payload.as_bytes())?;

    for att in &record.attachments {
        tx.execute(
            "INSERT INTO event_attachments (
                event_id, handle_id, modality_slot, byte_len, expires_at_ms, embed_model_ver
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                assigned_id.as_u64() as i64,
                &att.handle_id,
                format!("{:?}", att.modality_slot),
                att.byte_len as i64,
                att.expires_at_ms.as_millis() as i64,
                &att.embed_model_ver,
            ],
        )
        .map_err(map_sqlite_err)?;
    }

    write_meta_u64(&tx, "next_seq", next_seq)?;
    write_meta_u64(&tx, "next_event_id", next_id)?;
    tx.commit().map_err(map_sqlite_err)?;

    Ok(record)
}

/// Inserts raw pre-encoded event bytes verbatim into `hot.db` (for legacy upcasting and future-version tests).
///
/// The stored bytes are never rewritten or modified.
#[allow(clippy::too_many_arguments)]
pub fn insert_raw_hot_bytes(
    conn: &mut Connection,
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
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(map_sqlite_err)?;

    let mut next_seq = read_meta_u64(&tx, "next_seq")?.max(1);
    let mut next_id = read_meta_u64(&tx, "next_event_id")?.max(1);
    if id.as_u64() >= next_id {
        next_id = id.as_u64().saturating_add(1);
    }

    let seq = LogSeq::new(next_seq);
    next_seq = next_seq.saturating_add(1);

    tx.execute(
        "INSERT INTO events (
            seq, event_id, kind_code, kind_name, format_version,
            timestamp_ms, actor, place, audience_json, origin_actor, taint, raw_bytes
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            seq.as_u64() as i64,
            id.as_u64() as i64,
            i64::from(kind.code()),
            kind.as_str(),
            i64::from(format_version),
            timestamp_ms.as_millis() as i64,
            actor.as_str(),
            place.as_str(),
            encode_audience_list(audience),
            origin_actor.actor_id().as_str(),
            taint_code(taint),
            raw_bytes,
        ],
    )
    .map_err(map_sqlite_err)?;

    write_meta_u64(&tx, "next_seq", next_seq)?;
    write_meta_u64(&tx, "next_event_id", next_id)?;
    tx.commit().map_err(map_sqlite_err)?;
    Ok(seq)
}

/// Reads events from `hot.db` strictly after `after` in ascending `LogSeq` order, upcasting in memory.
pub fn read_hot_after_seq(
    conn: &Connection,
    after: Option<LogSeq>,
    limit: usize,
) -> Result<Vec<EventRecord>, StoreError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
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

/// Fetches a single event from `hot.db` by `EventId`, upcasting older formats in memory.
pub fn get_hot_event(conn: &Connection, id: EventId) -> Result<Option<EventRecord>, StoreError> {
    let row: Option<(i64, Vec<u8>)> = conn
        .query_row(
            "SELECT seq, raw_bytes FROM events WHERE event_id = ?1",
            params![id.as_u64() as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(map_sqlite_err)?;

    let Some((seq_raw, raw)) = row else {
        return Ok(None);
    };
    let mut rec = EventRecord::decode_and_upcast(&raw)?;
    if rec.seq == LogSeq::ZERO {
        rec.seq = LogSeq::new(seq_raw as u64);
    }
    Ok(Some(rec))
}

/// Returns the `LogSeq` for `id` in `hot.db`, if present.
pub fn get_hot_seq_for_id(conn: &Connection, id: EventId) -> Result<Option<LogSeq>, StoreError> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT seq FROM events WHERE event_id = ?1",
            params![id.as_u64() as i64],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sqlite_err)?;
    Ok(seq.map(|s| LogSeq::new(s as u64)))
}

/// Returns the raw stored bytes at `seq` in `hot.db` without decoding or modifying them.
pub fn raw_bytes_at_hot(conn: &Connection, seq: LogSeq) -> Result<Option<Vec<u8>>, StoreError> {
    conn.query_row(
        "SELECT raw_bytes FROM events WHERE seq = ?1",
        params![seq.as_u64() as i64],
        |r| r.get(0),
    )
    .optional()
    .map_err(map_sqlite_err)
}

/// Persists a vector into `event_vectors` in `hot.db`.
pub fn upsert_hot_vector(
    conn: &Connection,
    event_id: EventId,
    tag: &EmbeddingTag,
    values: &[f32],
) -> Result<(), StoreError> {
    let blob = encode_f32_blob(values);
    conn.execute(
        "INSERT INTO event_vectors (event_id, embed_model_ver, dim_d, prompt_regime_id, vector_blob, faded)
         VALUES (?1, ?2, ?3, ?4, ?5, 0)
         ON CONFLICT(event_id, embed_model_ver, dim_d, prompt_regime_id)
         DO UPDATE SET vector_blob = excluded.vector_blob, faded = 0",
        params![
            event_id.as_u64() as i64,
            &tag.embed_model_ver,
            i64::from(tag.dim_d),
            &tag.prompt_regime_id,
            blob,
        ],
    )
    .map_err(map_sqlite_err)?;
    Ok(())
}

/// Releases (deletes under `PRAGMA secure_delete = ON`) all vectors for `event_id` in `event_vectors`.
pub fn delete_hot_vectors_for_event(
    conn: &Connection,
    event_id: EventId,
) -> Result<(), StoreError> {
    conn.execute(
        "DELETE FROM event_vectors WHERE event_id = ?1",
        params![event_id.as_u64() as i64],
    )
    .map_err(map_sqlite_err)?;
    Ok(())
}

/// Loads all non-faded vectors from `event_vectors` (used when reopening a persisted store).
pub fn load_all_live_vectors(
    conn: &Connection,
) -> Result<Vec<(EventId, EmbeddingTag, Vec<f32>)>, StoreError> {
    let mut stmt = conn
        .prepare(
            "SELECT event_id, embed_model_ver, dim_d, prompt_regime_id, vector_blob
             FROM event_vectors WHERE faded = 0 ORDER BY event_id ASC",
        )
        .map_err(map_sqlite_err)?;
    let rows = stmt
        .query_map([], |row| {
            let eid: i64 = row.get(0)?;
            let model: String = row.get(1)?;
            let dim: i64 = row.get(2)?;
            let regime: String = row.get(3)?;
            let blob: Vec<u8> = row.get(4)?;
            Ok((eid, model, dim, regime, blob))
        })
        .map_err(map_sqlite_err)?;

    let mut out = Vec::new();
    for item in rows {
        let (eid, embed_model_ver, dim, prompt_regime_id, blob) = item.map_err(map_sqlite_err)?;
        let values = decode_f32_blob(&blob)?;
        out.push((
            EventId::new(eid as u64),
            EmbeddingTag {
                embed_model_ver,
                dim_d: dim as u32,
                prompt_regime_id,
            },
            values,
        ));
    }
    Ok(out)
}

fn encode_audience_list(audience: &Audience) -> String {
    let members: Vec<&str> = audience.members.iter().map(ActorId::as_str).collect();
    // Deterministic compact serialization without adding serde_json dependency to store.
    let mut out = String::new();
    for (i, m) in members.iter().enumerate() {
        if i > 0 {
            out.push('\x1f');
        }
        out.push_str(m);
    }
    out
}

fn taint_code(taint: Taint) -> i64 {
    match taint {
        Taint::Untrusted => 0,
        Taint::Provisional => 1,
        Taint::Trusted => 2,
    }
}

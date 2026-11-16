//! FTS5 full-text indexing and search with `'secure-delete'=1`.

use std::collections::BTreeSet;

use monoagens_core::StoreError;
use monoagens_protocol::EventId;
use rusqlite::{Connection, params};

use crate::schema::map_sqlite_err;

const MAX_QUERY_TOKENS: usize = 64;

/// Converts opaque payload bytes into safe UTF-8 text for FTS5 indexing (stripping NULs).
#[must_use]
pub fn extract_indexable_text(payload: &[u8]) -> String {
    let lossy = String::from_utf8_lossy(payload);
    if lossy.contains('\0') {
        lossy.replace('\0', " ")
    } else {
        lossy.into_owned()
    }
}

/// Builds a safe double-quoted FTS5 `OR` query string from alphanumeric tokens in `payload`.
#[must_use]
pub fn build_fts5_match_query(payload: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(payload);
    let mut unique = BTreeSet::new();
    let mut terms = Vec::new();
    for token in text.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() {
            continue;
        }
        let lower = token.to_lowercase();
        if unique.insert(lower.clone()) {
            let escaped = lower.replace('"', "\"\"");
            terms.push(format!("\"{escaped}\""));
            if terms.len() >= MAX_QUERY_TOKENS {
                break;
            }
        }
    }
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// Inserts an event's text into the named FTS5 virtual table (`hot_fts` or `shard_fts`).
pub fn index_event_text(
    conn: &Connection,
    table: &str,
    event_id: EventId,
    payload: &[u8],
) -> Result<(), StoreError> {
    let text = extract_indexable_text(payload);
    if text.trim().is_empty() {
        return Ok(());
    }
    let sql = format!("INSERT INTO {table} (event_id, body_text) VALUES (?1, ?2)");
    conn.execute(&sql, params![event_id.as_u64() as i64, text])
        .map_err(map_sqlite_err)?;
    Ok(())
}

/// Removes an event's tokens from the named FTS5 virtual table (with `'secure-delete'=1` active).
pub fn delete_event_text(
    conn: &Connection,
    table: &str,
    event_id: EventId,
) -> Result<(), StoreError> {
    let sql = format!("DELETE FROM {table} WHERE CAST(event_id AS INTEGER) = ?1");
    conn.execute(&sql, params![event_id.as_u64() as i64])
        .map_err(map_sqlite_err)?;
    Ok(())
}

/// Queries an FTS5 virtual table returning `(EventId, bm25_score)` ordered best-first (lowest BM25).
pub fn query_fts_table(
    conn: &Connection,
    table: &str,
    fts_query: &str,
    exclude_event_id: EventId,
    limit: usize,
) -> Result<Vec<(EventId, f64)>, StoreError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT CAST(event_id AS INTEGER) AS eid, bm25({table}) AS score
         FROM {table}
         WHERE {table} MATCH ?1 AND CAST(event_id AS INTEGER) != ?2
         ORDER BY score ASC, eid ASC
         LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql).map_err(map_sqlite_err)?;
    let rows = stmt
        .query_map(
            params![fts_query, exclude_event_id.as_u64() as i64, limit as i64],
            |row| {
                let eid: i64 = row.get(0)?;
                let score: f64 = row.get(1)?;
                Ok((EventId::new(eid as u64), score))
            },
        )
        .map_err(map_sqlite_err)?;

    let mut out = Vec::new();
    for item in rows {
        out.push(item.map_err(map_sqlite_err)?);
    }
    Ok(out)
}

/// Checks whether `'secure-delete'=1` is enabled in the named FTS5 table's `%_config` shadow table.
pub fn is_fts5_secure_delete_enabled(conn: &Connection, table: &str) -> Result<bool, StoreError> {
    let sql = format!("SELECT v FROM {table}_config WHERE k = 'secure-delete'");
    let mut stmt = conn.prepare(&sql).map_err(map_sqlite_err)?;
    let mut rows = stmt.query([]).map_err(map_sqlite_err)?;
    if let Some(row) = rows.next().map_err(map_sqlite_err)? {
        let v: i64 = row.get(0).map_err(map_sqlite_err)?;
        return Ok(v == 1);
    }
    Ok(false)
}

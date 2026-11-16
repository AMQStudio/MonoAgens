//! `blake3`-checksummed `postcard` state snapshots written atomically via temp file + rename.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use monoagens_core::{LogSeq, SnapshotCheckpoint, StoreError};
use serde::{Deserialize, Serialize};

use crate::schema::map_io_err;

const SNAPSHOT_FILE_MAGIC: &[u8; 4] = b"MGSP";
const SNAPSHOT_ENVELOPE_VERSION: u16 = 1;
const BLAKE3_LEN: usize = 32;

/// `postcard`-serialized wrapper tying `CoreState` bytes to `LogSeq` cursor and `Two-Epoch Replay` epoch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotEnvelope {
    pub format_version: u16,
    pub snapshot_id: String,
    pub cursor: u64,
    pub epoch: u64,
    pub payload_bytes: Vec<u8>,
    pub payload_blake3: [u8; 32],
}

/// Writes `checkpoint` atomically using `postcard` + `blake3` via a temp file + `sync_all` + `rename`.
pub fn write_snapshot_atomic(
    snapshots_dir: &Path,
    generation: u64,
    checkpoint: &SnapshotCheckpoint,
) -> Result<PathBuf, StoreError> {
    fs::create_dir_all(snapshots_dir).map_err(|e| map_io_err("create snapshots dir", e))?;

    let computed_payload_hash = *blake3::hash(&checkpoint.bytes).as_bytes();
    if computed_payload_hash != checkpoint.blake3 {
        return Err(StoreError::SnapshotCorrupted {
            snapshot_id: checkpoint.snapshot_id.clone(),
            detail: "checkpoint payload blake3 does not match payload bytes on write".to_owned(),
        });
    }

    let envelope = SnapshotEnvelope {
        format_version: SNAPSHOT_ENVELOPE_VERSION,
        snapshot_id: checkpoint.snapshot_id.clone(),
        cursor: checkpoint.cursor.as_u64(),
        epoch: checkpoint.epoch,
        payload_bytes: checkpoint.bytes.clone(),
        payload_blake3: checkpoint.blake3,
    };

    let encoded = postcard::to_stdvec(&envelope).map_err(|err| {
        StoreError::Backend(format!(
            "postcard snapshot encode failed for '{}': {err}",
            checkpoint.snapshot_id
        ))
    })?;
    let outer_hash = *blake3::hash(&encoded).as_bytes();

    let final_name = format!(
        "snap_{:016x}_{:016x}_{:016x}.snap",
        checkpoint.epoch,
        checkpoint.cursor.as_u64(),
        generation
    );
    let final_path = snapshots_dir.join(&final_name);
    let tmp_path = snapshots_dir.join(format!(".{final_name}.{}.tmp", std::process::id()));

    {
        let mut file =
            File::create(&tmp_path).map_err(|e| map_io_err("create temp snapshot", e))?;
        file.write_all(SNAPSHOT_FILE_MAGIC)
            .map_err(|e| map_io_err("write snapshot magic", e))?;
        file.write_all(&outer_hash)
            .map_err(|e| map_io_err("write snapshot outer hash", e))?;
        file.write_all(&encoded)
            .map_err(|e| map_io_err("write snapshot payload", e))?;
        file.sync_all()
            .map_err(|e| map_io_err("sync temp snapshot", e))?;
    }

    fs::rename(&tmp_path, &final_path).map_err(|e| map_io_err("rename snapshot", e))?;
    if let Ok(dir) = File::open(snapshots_dir) {
        let _ = dir.sync_all();
    }
    Ok(final_path)
}

/// Lists all `.snap` files in `snapshots_dir` sorted chronologically by `(epoch, cursor, generation)`.
pub fn list_snapshot_files(snapshots_dir: &Path) -> Result<Vec<PathBuf>, StoreError> {
    if !snapshots_dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(snapshots_dir).map_err(|e| map_io_err("read snapshots dir", e))? {
        let entry = entry.map_err(|e| map_io_err("read snapshot entry", e))?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("snap") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Reads and verifies the latest snapshot file in `snapshots_dir`, failing loudly on checksum mismatch.
pub fn read_latest_snapshot(
    snapshots_dir: &Path,
) -> Result<Option<SnapshotCheckpoint>, StoreError> {
    let files = list_snapshot_files(snapshots_dir)?;
    let Some(latest_path) = files.last() else {
        return Ok(None);
    };
    let ckpt = read_snapshot_file(latest_path)?;
    Ok(Some(ckpt))
}

/// Reads and verifies a single snapshot file, naming the snapshot if corrupted.
pub fn read_snapshot_file(path: &Path) -> Result<SnapshotCheckpoint, StoreError> {
    let fallback_id = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_owned();
    let bytes = fs::read(path).map_err(|e| map_io_err("read snapshot file", e))?;

    let header_len = SNAPSHOT_FILE_MAGIC.len() + BLAKE3_LEN;
    if bytes.len() < header_len || &bytes[..SNAPSHOT_FILE_MAGIC.len()] != SNAPSHOT_FILE_MAGIC {
        return Err(StoreError::SnapshotCorrupted {
            snapshot_id: fallback_id,
            detail: "invalid snapshot file magic or truncated header".to_owned(),
        });
    }

    let mut expected_outer = [0u8; BLAKE3_LEN];
    expected_outer.copy_from_slice(&bytes[SNAPSHOT_FILE_MAGIC.len()..header_len]);
    let body = &bytes[header_len..];
    let actual_outer = *blake3::hash(body).as_bytes();

    // Try decoding envelope even on mismatch so diagnostics can include the logical snapshot_id if intact.
    let decoded_envelope = postcard::from_bytes::<SnapshotEnvelope>(body);
    let diagnostic_id = decoded_envelope
        .as_ref()
        .map(|env| env.snapshot_id.clone())
        .unwrap_or(fallback_id);

    if actual_outer != expected_outer {
        return Err(StoreError::SnapshotCorrupted {
            snapshot_id: diagnostic_id,
            detail: format!(
                "blake3 checksum mismatch: expected {}, got {}",
                blake3::Hash::from_bytes(expected_outer).to_hex(),
                blake3::Hash::from_bytes(actual_outer).to_hex()
            ),
        });
    }

    let envelope = decoded_envelope.map_err(|err| StoreError::SnapshotCorrupted {
        snapshot_id: diagnostic_id.clone(),
        detail: format!("postcard decode failed: {err}"),
    })?;

    if envelope.format_version != SNAPSHOT_ENVELOPE_VERSION {
        return Err(StoreError::SnapshotCorrupted {
            snapshot_id: envelope.snapshot_id,
            detail: format!(
                "unsupported snapshot envelope version {} (expected {})",
                envelope.format_version, SNAPSHOT_ENVELOPE_VERSION
            ),
        });
    }

    let actual_payload = *blake3::hash(&envelope.payload_bytes).as_bytes();
    if actual_payload != envelope.payload_blake3 {
        return Err(StoreError::SnapshotCorrupted {
            snapshot_id: envelope.snapshot_id,
            detail: "inner payload blake3 checksum mismatch".to_owned(),
        });
    }

    Ok(SnapshotCheckpoint {
        snapshot_id: envelope.snapshot_id,
        cursor: LogSeq::new(envelope.cursor),
        epoch: envelope.epoch,
        bytes: envelope.payload_bytes,
        blake3: envelope.payload_blake3,
    })
}

/// Removes all `.snap` files in `snapshots_dir` except `keep_path` (used on post-`hard_forget` cleanup).
pub fn remove_snapshots_except(
    snapshots_dir: &Path,
    keep_path: Option<&Path>,
) -> Result<usize, StoreError> {
    let mut removed = 0usize;
    for path in list_snapshot_files(snapshots_dir)? {
        if keep_path.is_some_and(|keep| keep == path.as_path()) {
            continue;
        }
        fs::remove_file(&path).map_err(|e| map_io_err("remove pre-forget snapshot", e))?;
        removed += 1;
    }
    Ok(removed)
}

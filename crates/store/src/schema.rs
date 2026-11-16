//! SQLite schema, connection pragmas, and configuration for `hot.db` and sealed shards.

use std::path::{Path, PathBuf};

use monoagens_core::StoreError;
use rusqlite::{Connection, OpenFlags};

/// Filename of the active WAL-mode SQLite database inside the store root directory.
pub const HOT_DB_FILENAME: &str = "hot.db";
/// Subdirectory holding read-only sealed temporal shard SQLite files.
pub const SHARDS_DIR_NAME: &str = "shards";
/// Subdirectory holding `blake3`-checksummed `postcard` state snapshots.
pub const SNAPSHOTS_DIR_NAME: &str = "snapshots";
/// Current SQLite schema version stored in `store_meta`.
pub const STORE_SCHEMA_VERSION: u64 = 1;

/// SQLite `PRAGMA synchronous` durability mode (`NORMAL` or stronger).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SynchronousMode {
    #[default]
    Normal,
    Full,
    Extra,
}

impl SynchronousMode {
    #[must_use]
    pub const fn as_pragma_value(self) -> &'static str {
        match self {
            Self::Normal => "NORMAL",
            Self::Full => "FULL",
            Self::Extra => "EXTRA",
        }
    }
}

/// Scalar quantization format for the primary `usearch` HNSW backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UsearchQuantization {
    #[default]
    Bf16,
    I8,
}

/// Explicit selection of the [`crate::VectorIndex`] backend implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VectorBackendConfig {
    #[cfg(feature = "usearch")]
    Usearch {
        quantization: UsearchQuantization,
    },
    HnswRs,
    BruteForceFlat,
}

#[allow(clippy::derivable_impls)]
impl Default for VectorBackendConfig {
    fn default() -> Self {
        #[cfg(feature = "usearch")]
        {
            Self::Usearch {
                quantization: UsearchQuantization::default(),
            }
        }
        #[cfg(not(feature = "usearch"))]
        {
            Self::HnswRs
        }
    }
}

/// Configuration for opening a [`crate::SegmentedStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreConfig {
    pub root_dir: PathBuf,
    pub synchronous: SynchronousMode,
    pub vector_backend: VectorBackendConfig,
}

impl StoreConfig {
    /// Creates a `StoreConfig` rooted at `root_dir` with default durability and vector backend.
    #[must_use]
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
            synchronous: SynchronousMode::default(),
            vector_backend: VectorBackendConfig::default(),
        }
    }

    /// Sets the `PRAGMA synchronous` durability level.
    #[must_use]
    pub const fn with_synchronous(mut self, synchronous: SynchronousMode) -> Self {
        self.synchronous = synchronous;
        self
    }

    /// Sets an explicit vector index backend configuration.
    #[must_use]
    pub const fn with_vector_backend(mut self, vector_backend: VectorBackendConfig) -> Self {
        self.vector_backend = vector_backend;
        self
    }

    /// Returns the path to `hot.db`.
    #[must_use]
    pub fn hot_db_path(&self) -> PathBuf {
        self.root_dir.join(HOT_DB_FILENAME)
    }

    /// Returns the path to the `shards/` directory.
    #[must_use]
    pub fn shards_dir(&self) -> PathBuf {
        self.root_dir.join(SHARDS_DIR_NAME)
    }

    /// Returns the path to the `snapshots/` directory.
    #[must_use]
    pub fn snapshots_dir(&self) -> PathBuf {
        self.root_dir.join(SNAPSHOTS_DIR_NAME)
    }
}

/// Maps any `rusqlite::Error` into a [`StoreError::Backend`].
#[must_use]
pub fn map_sqlite_err(err: rusqlite::Error) -> StoreError {
    StoreError::Backend(format!("sqlite error: {err}"))
}

/// Maps any `std::io::Error` into a [`StoreError::Backend`].
#[must_use]
pub fn map_io_err(context: &str, err: std::io::Error) -> StoreError {
    StoreError::Backend(format!("{context}: {err}"))
}

/// Configures WAL mode, `PRAGMA secure_delete = ON`, and `PRAGMA synchronous` on `hot.db`.
pub fn configure_hot_connection(
    conn: &Connection,
    synchronous: SynchronousMode,
) -> Result<(), StoreError> {
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(map_sqlite_err)?;
    conn.pragma_update(None, "synchronous", synchronous.as_pragma_value())
        .map_err(map_sqlite_err)?;
    conn.pragma_update(None, "secure_delete", "ON")
        .map_err(map_sqlite_err)?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(map_sqlite_err)?;
    conn.pragma_update(None, "busy_timeout", 5000_i64)
        .map_err(map_sqlite_err)?;
    Ok(())
}

/// Initializes all tables, indexes, metadata defaults, and FTS5 `'secure-delete'=1` in `hot.db`.
pub fn init_hot_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS store_meta (
            key TEXT PRIMARY KEY,
            int_val INTEGER,
            text_val TEXT
        );

        CREATE TABLE IF NOT EXISTS events (
            seq INTEGER PRIMARY KEY,
            event_id INTEGER UNIQUE NOT NULL,
            kind_code INTEGER NOT NULL,
            kind_name TEXT NOT NULL,
            format_version INTEGER NOT NULL,
            timestamp_ms INTEGER NOT NULL,
            actor TEXT NOT NULL,
            place TEXT NOT NULL,
            audience_json TEXT NOT NULL,
            origin_actor TEXT NOT NULL,
            taint INTEGER NOT NULL,
            raw_bytes BLOB NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_events_ts ON events(timestamp_ms, seq);
        CREATE INDEX IF NOT EXISTS idx_events_actor ON events(actor);

        CREATE TABLE IF NOT EXISTS event_attachments (
            event_id INTEGER NOT NULL,
            handle_id TEXT NOT NULL,
            modality_slot TEXT NOT NULL,
            byte_len INTEGER NOT NULL,
            expires_at_ms INTEGER NOT NULL,
            embed_model_ver TEXT NOT NULL,
            PRIMARY KEY (event_id, handle_id)
        );

        CREATE TABLE IF NOT EXISTS event_vectors (
            event_id INTEGER NOT NULL,
            embed_model_ver TEXT NOT NULL,
            dim_d INTEGER NOT NULL,
            prompt_regime_id TEXT NOT NULL,
            vector_blob BLOB NOT NULL,
            faded INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (event_id, embed_model_ver, dim_d, prompt_regime_id)
        );

        CREATE TABLE IF NOT EXISTS shards (
            shard_id TEXT PRIMARY KEY,
            min_seq INTEGER NOT NULL,
            max_seq INTEGER NOT NULL,
            min_ts_ms INTEGER NOT NULL,
            max_ts_ms INTEGER NOT NULL,
            file_name TEXT NOT NULL,
            blake3_hex TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS hot_fts USING fts5(
            event_id UNINDEXED,
            body_text,
            tokenize = 'unicode61'
        );
        INSERT INTO hot_fts(hot_fts, rank) VALUES('secure-delete', 1);

        INSERT OR IGNORE INTO store_meta (key, int_val, text_val) VALUES ('schema_version', 1, NULL);
        INSERT OR IGNORE INTO store_meta (key, int_val, text_val) VALUES ('next_seq', 1, NULL);
        INSERT OR IGNORE INTO store_meta (key, int_val, text_val) VALUES ('next_event_id', 1, NULL);
        INSERT OR IGNORE INTO store_meta (key, int_val, text_val) VALUES ('epoch', 0, NULL);
        INSERT OR IGNORE INTO store_meta (key, int_val, text_val) VALUES ('snapshot_gen', 0, NULL);
        ",
    )
    .map_err(map_sqlite_err)?;

    Ok(())
}

/// Initializes a sealed temporal shard SQLite database with `secure_delete = ON` and FTS5 `'secure-delete'=1`.
pub fn init_shard_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.pragma_update(None, "journal_mode", "DELETE")
        .map_err(map_sqlite_err)?;
    conn.pragma_update(None, "synchronous", "FULL")
        .map_err(map_sqlite_err)?;
    conn.pragma_update(None, "secure_delete", "ON")
        .map_err(map_sqlite_err)?;
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS shard_meta (
            key TEXT PRIMARY KEY,
            int_val INTEGER,
            text_val TEXT
        );

        CREATE TABLE IF NOT EXISTS events (
            seq INTEGER PRIMARY KEY,
            event_id INTEGER UNIQUE NOT NULL,
            kind_code INTEGER NOT NULL,
            kind_name TEXT NOT NULL,
            format_version INTEGER NOT NULL,
            timestamp_ms INTEGER NOT NULL,
            actor TEXT NOT NULL,
            place TEXT NOT NULL,
            audience_json TEXT NOT NULL,
            origin_actor TEXT NOT NULL,
            taint INTEGER NOT NULL,
            raw_bytes BLOB NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_shard_events_ts ON events(timestamp_ms, seq);
        CREATE INDEX IF NOT EXISTS idx_shard_events_actor ON events(actor);

        CREATE TABLE IF NOT EXISTS event_attachments (
            event_id INTEGER NOT NULL,
            handle_id TEXT NOT NULL,
            modality_slot TEXT NOT NULL,
            byte_len INTEGER NOT NULL,
            expires_at_ms INTEGER NOT NULL,
            embed_model_ver TEXT NOT NULL,
            PRIMARY KEY (event_id, handle_id)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS shard_fts USING fts5(
            event_id UNINDEXED,
            body_text,
            tokenize = 'unicode61'
        );
        INSERT INTO shard_fts(shard_fts, rank) VALUES('secure-delete', 1);
        ",
    )
    .map_err(map_sqlite_err)?;
    Ok(())
}

/// Opens a sealed shard SQLite database strictly in read-only mode.
pub fn open_readonly_shard(path: &Path) -> Result<Connection, StoreError> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(map_sqlite_err)
}

//! Scope-keyed `BLAKE3` AEAD-at-rest attachment custody, `<= 256 KiB` inline stripping,
//! `blob/put`/`blob/get` staging (`60 s` TTL, SHA-256 verification), SQLite `attachment_refs`,
//! and synchronous `hard_forget` cascade erasure (`4.4`, `R-37`).

use core::fmt;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chacha20poly1305::aead::{Aead, Generate, Key, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use monoagens_protocol::{
    ActorId, AttachmentHandle, BlobGetParams, BlobGetResult, BlobPutParams, BlobPutResult, BodyId,
    EventId, MAX_INLINE_ATTACHMENT_BYTES, MetaValue, ModalitySlot, OpaquePayload, Percept, Time,
    Waist2Error,
};
use monoagens_reason::{AttachmentBlobStore, AttachmentError, EMBEDDING_GEMMA_2_VER};
use rusqlite::{Connection, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::clock::Clock;
use crate::seams::{AttachmentCustodySeam, SeamError};
use crate::secrets::AttachmentSecrets;

/// Default out-of-band staging TTL (`60 s`).
pub const STAGING_TTL_MS: u64 = 60_000;

/// Default persisted attachment handle expiry (`365` days if not specified).
pub const DEFAULT_CUSTODY_TTL_MS: u64 = 365 * 24 * 3600 * 1_000;

const ATTACHMENTS_DIR_NAME: &str = "attachments";
const STAGING_DIR_NAME: &str = "_staging";
const ATTACHMENTS_DB_FILENAME: &str = "attachments.db";
const AEAD_FILE_MAGIC: &[u8; 5] = b"MAGA1";
const AEAD_NONCE_LEN: usize = 24;
const AEAD_TAG_LEN: usize = 16;
const SCOPE_META_KEY: &str = "scope_id";
const NAMESPACED_SCOPE_META_KEY: &str = "monoagens.scope_id";

/// Errors returned by the Gateway attachment custody store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentCustodyError {
    MissingMasterKey,
    EntropyFailure(String),
    Io {
        path: String,
        detail: String,
    },
    Sqlite(String),
    InlineLimitExceeded {
        byte_len: usize,
        max_bytes: usize,
    },
    NotFound {
        scope_id: String,
        handle_id: String,
    },
    Expired {
        handle_id: String,
        expires_at_ms: Time,
        now_ms: Time,
    },
    AeadTamper {
        path: String,
        detail: String,
    },
    HashMismatch {
        expected: String,
        actual: String,
    },
    InvalidScope(String),
}

impl fmt::Display for AttachmentCustodyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMasterKey => f.write_str(
                "attachment AEAD master key missing; configure [attachments].master_key in secrets.toml or MONOAGENS_ATTACHMENT_MASTER_KEY",
            ),
            Self::EntropyFailure(detail) => write!(f, "CSPRNG error: {detail}"),
            Self::Io { path, detail } => write!(f, "attachment I/O error at '{path}': {detail}"),
            Self::Sqlite(detail) => write!(f, "attachment sqlite error: {detail}"),
            Self::InlineLimitExceeded {
                byte_len,
                max_bytes,
            } => write!(
                f,
                "inline payload {byte_len} bytes exceeds {max_bytes}-byte limit; transfer via blob/put instead"
            ),
            Self::NotFound {
                scope_id,
                handle_id,
            } => write!(
                f,
                "attachment '{handle_id}' not found in scope '{scope_id}'"
            ),
            Self::Expired {
                handle_id,
                expires_at_ms,
                now_ms,
            } => write!(
                f,
                "attachment '{handle_id}' expired at {expires_at_ms} (now {now_ms})"
            ),
            Self::AeadTamper { path, detail } => write!(
                f,
                "AEAD authentication/integrity failure for '{path}': {detail}"
            ),
            Self::HashMismatch { expected, actual } => {
                write!(f, "hash mismatch: expected {expected}, got {actual}")
            }
            Self::InvalidScope(msg) => write!(f, "invalid attachment scope: {msg}"),
        }
    }
}

impl std::error::Error for AttachmentCustodyError {}

/// Zeroizing 32-byte master key loaded from the secrets layer (`secrets.toml` / env)
/// or generated via OS CSPRNG for ephemeral/test stores.
#[derive(Clone)]
pub struct AttachmentMasterKey([u8; 32]);

impl AttachmentMasterKey {
    /// Derives the 32-byte master key from [`AttachmentSecrets`] (never hardcoded in repo).
    pub fn from_secrets(secrets: &AttachmentSecrets) -> Result<Self, AttachmentCustodyError> {
        let Some(raw) = &secrets.master_key else {
            return Err(AttachmentCustodyError::MissingMasterKey);
        };
        Ok(Self::from_secret_str(raw.expose()))
    }

    /// Derives a 32-byte master key from a secret string (64-hex raw key or BLAKE3 KDF).
    #[must_use]
    pub fn from_secret_str(raw: &str) -> Self {
        let trimmed = raw.trim();
        if trimmed.len() == 64
            && let Some(decoded) = decode_hex_32(trimmed)
        {
            return Self(decoded);
        }
        let derived = blake3::derive_key("monoagens.attachments.master.v1", trimmed.as_bytes());
        Self(derived)
    }

    /// Generates a fresh 256-bit random master key via OS CSPRNG (for tests / ephemeral runs).
    pub fn generate_ephemeral() -> Result<Self, AttachmentCustodyError> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes)
            .map_err(|e| AttachmentCustodyError::EntropyFailure(e.to_string()))?;
        Ok(Self(bytes))
    }

    /// Derives a scope-specific 32-byte salt for `blake3::keyed_hash`.
    fn scope_salt(&self, scope_id: &str) -> [u8; 32] {
        derive_scoped_subkey(&self.0, b"monoagens.attachments.scope_salt.v1", scope_id)
    }

    /// Derives a scope-specific 32-byte `XChaCha20Poly1305` key.
    fn scope_aead_key(&self, scope_id: &str) -> [u8; 32] {
        derive_scoped_subkey(&self.0, b"monoagens.attachments.aead_key.v1", scope_id)
    }

    /// Derives the staging AEAD key.
    fn staging_aead_key(&self) -> [u8; 32] {
        derive_scoped_subkey(
            &self.0,
            b"monoagens.attachments.staging_aead.v1",
            "_staging",
        )
    }
}

impl Drop for AttachmentMasterKey {
    fn drop(&mut self) {
        self.0.fill(0);
    }
}

impl fmt::Debug for AttachmentMasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AttachmentMasterKey([REDACTED])")
    }
}

/// Audit summary returned by synchronous [`AttachmentStore::hard_forget`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HardForgetAttachmentReport {
    pub removed_refs: usize,
    pub wiped_blobs: usize,
    pub wiped_staged_blobs: usize,
    pub wiped_cache_entries: usize,
    pub overwritten_bytes: u64,
}

/// Scope-keyed `BLAKE3` + `XChaCha20Poly1305` AEAD attachment store backed by SQLite
/// `attachment_refs` and synchronous `hard_forget` wiping (`4.4`).
pub struct AttachmentStore {
    root_dir: PathBuf,
    master_key: AttachmentMasterKey,
    conn: Connection,
    decoded_cache: BTreeMap<(String, String), Vec<u8>>,
    next_staging_seq: u64,
    default_embed_model_ver: String,
}

impl fmt::Debug for AttachmentStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AttachmentStore")
            .field("root_dir", &self.root_dir)
            .field("cached_entries", &self.decoded_cache.len())
            .finish()
    }
}

impl Drop for AttachmentStore {
    fn drop(&mut self) {
        for (_, mut bytes) in std::mem::take(&mut self.decoded_cache) {
            zero_bytes(&mut bytes);
        }
    }
}

impl AttachmentStore {
    /// Opens or initializes an [`AttachmentStore`] under `root_dir` (`<root_dir>/attachments/`).
    pub fn open(
        root_dir: impl Into<PathBuf>,
        master_key: AttachmentMasterKey,
    ) -> Result<Self, AttachmentCustodyError> {
        let root_dir = root_dir.into();
        let att_dir = root_dir.join(ATTACHMENTS_DIR_NAME);
        let staging_dir = att_dir.join(STAGING_DIR_NAME);
        fs::create_dir_all(&staging_dir).map_err(|e| AttachmentCustodyError::Io {
            path: staging_dir.display().to_string(),
            detail: e.to_string(),
        })?;

        let db_path = att_dir.join(ATTACHMENTS_DB_FILENAME);
        let conn = Connection::open(&db_path)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = FULL;
            PRAGMA secure_delete = ON;
            PRAGMA foreign_keys = ON;
            PRAGMA busy_timeout = 5000;

            CREATE TABLE IF NOT EXISTS attachment_blobs (
                scope_id TEXT NOT NULL,
                blob_hex TEXT NOT NULL,
                handle_id TEXT UNIQUE NOT NULL,
                modality_slot TEXT NOT NULL,
                byte_len INTEGER NOT NULL,
                sha256_hex TEXT NOT NULL,
                rel_path TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                expires_at_ms INTEGER NOT NULL,
                PRIMARY KEY (scope_id, blob_hex)
            );

            CREATE TABLE IF NOT EXISTS attachment_refs (
                ref_id INTEGER PRIMARY KEY AUTOINCREMENT,
                scope_id TEXT NOT NULL,
                blob_hex TEXT NOT NULL,
                handle_id TEXT NOT NULL,
                event_id INTEGER,
                actor_id TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_att_refs_scope_hex
                ON attachment_refs(scope_id, blob_hex);
            CREATE INDEX IF NOT EXISTS idx_att_refs_actor
                ON attachment_refs(actor_id);
            CREATE INDEX IF NOT EXISTS idx_att_refs_event
                ON attachment_refs(event_id);

            CREATE TABLE IF NOT EXISTS staged_blobs (
                blob_ref TEXT PRIMARY KEY,
                scope_id TEXT,
                sha256_hex TEXT NOT NULL,
                byte_len INTEGER NOT NULL,
                rel_path TEXT NOT NULL,
                created_at_ms INTEGER NOT NULL,
                expires_at_ms INTEGER NOT NULL
            );
            ",
        )
        .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;

        Ok(Self {
            root_dir,
            master_key,
            conn,
            decoded_cache: BTreeMap::new(),
            next_staging_seq: 1,
            default_embed_model_ver: EMBEDDING_GEMMA_2_VER.to_owned(),
        })
    }

    /// Returns the attachments directory `<root_dir>/attachments`.
    #[must_use]
    pub fn attachments_dir(&self) -> PathBuf {
        self.root_dir.join(ATTACHMENTS_DIR_NAME)
    }

    /// Computes the scope-keyed `BLAKE3` hex digest and canonical relative path
    /// `attachments/<scope_id>/<prefix>/<hex>`.
    pub fn scoped_digest_and_rel_path(
        &self,
        scope_id: &str,
        plaintext: &[u8],
    ) -> Result<(String, PathBuf), AttachmentCustodyError> {
        let dir_scope = sanitize_scope_dir(scope_id)?;
        let salt = self.master_key.scope_salt(scope_id);
        let hex = blake3::keyed_hash(&salt, plaintext).to_hex().to_string();
        let prefix = &hex[..2];
        let rel = PathBuf::from(ATTACHMENTS_DIR_NAME)
            .join(dir_scope)
            .join(prefix)
            .join(&hex);
        Ok((hex, rel))
    }

    /// Stores inline media bytes (`<= 256 KiB`) into `attachments/<scope_id>/<prefix>/<hex>`
    /// encrypted with AEAD at rest, registers `attachment_refs`, and returns an opaque [`AttachmentHandle`].
    ///
    /// Rejects any inline payload larger than `256 KiB` (`MAX_INLINE_ATTACHMENT_BYTES`).
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_inline_bytes(
        &mut self,
        scope_id: &str,
        actor_id: &ActorId,
        event_id: Option<EventId>,
        modality_slot: ModalitySlot,
        bytes: &[u8],
        now_ms: Time,
    ) -> Result<AttachmentHandle, AttachmentCustodyError> {
        if bytes.len() > MAX_INLINE_ATTACHMENT_BYTES {
            return Err(AttachmentCustodyError::InlineLimitExceeded {
                byte_len: bytes.len(),
                max_bytes: MAX_INLINE_ATTACHMENT_BYTES,
            });
        }
        self.persist_scoped_blob(
            scope_id,
            actor_id,
            event_id,
            modality_slot,
            bytes,
            now_ms,
            now_ms.saturating_add_ms(DEFAULT_CUSTODY_TTL_MS),
        )
    }

    /// Strips inline media from `percept.payload` (enforcing `<= 256 KiB`) into `scope_id`,
    /// replacing raw bytes with an opaque [`AttachmentHandle`] and clearing `percept.payload`.
    pub fn strip_inline_media_from_percept(
        &mut self,
        scope_id: &str,
        modality_slot: ModalitySlot,
        percept: &mut Percept,
    ) -> Result<AttachmentHandle, AttachmentCustodyError> {
        let handle = self.ingest_inline_bytes(
            scope_id,
            &percept.actor,
            None,
            modality_slot,
            percept.payload.as_bytes(),
            percept.timestamp_ms,
        )?;
        percept.payload = OpaquePayload::default();
        percept.attachments.push(handle.clone());
        Ok(handle)
    }

    /// Persists an already-validated blob (inline `<= 256 KiB` or promoted out-of-band)
    /// into scope custody (`attachments/<scope_id>/<prefix>/<hex>` + SQLite `attachment_refs`).
    #[allow(clippy::too_many_arguments)]
    fn persist_scoped_blob(
        &mut self,
        scope_id: &str,
        actor_id: &ActorId,
        event_id: Option<EventId>,
        modality_slot: ModalitySlot,
        plaintext: &[u8],
        now_ms: Time,
        expires_at_ms: Time,
    ) -> Result<AttachmentHandle, AttachmentCustodyError> {
        let (blob_hex, rel_path) = self.scoped_digest_and_rel_path(scope_id, plaintext)?;
        let abs_path = self.root_dir.join(&rel_path);
        let handle_id = format!("att:{}:{}", sanitize_scope_dir(scope_id)?, &blob_hex[..32]);
        let sha256_hex = sha256_hex(plaintext);

        if !abs_path.exists() {
            let aead_key = self.master_key.scope_aead_key(scope_id);
            let aad = build_custody_aad(scope_id, &blob_hex);
            let sealed = encrypt_aead(&aead_key, &aad, plaintext)?;
            write_atomic_bytes(&abs_path, &sealed)?;
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;

        tx.execute(
            "INSERT OR IGNORE INTO attachment_blobs
             (scope_id, blob_hex, handle_id, modality_slot, byte_len, sha256_hex, rel_path, created_at_ms, expires_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                scope_id,
                &blob_hex,
                &handle_id,
                modality_to_str(modality_slot),
                plaintext.len() as i64,
                &sha256_hex,
                rel_path.to_string_lossy().as_ref(),
                now_ms.as_millis() as i64,
                expires_at_ms.as_millis() as i64,
            ],
        )
        .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;

        tx.execute(
            "INSERT INTO attachment_refs (scope_id, blob_hex, handle_id, event_id, actor_id, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                scope_id,
                &blob_hex,
                &handle_id,
                event_id.map(|id| id.as_u64() as i64),
                actor_id.as_str(),
                now_ms.as_millis() as i64,
            ],
        )
        .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;

        tx.commit()
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;

        Ok(AttachmentHandle {
            handle_id,
            modality_slot,
            byte_len: plaintext.len() as u64,
            expires_at_ms,
            embed_model_ver: self.default_embed_model_ver.clone(),
        })
    }

    /// Stages an out-of-band blob via Waist-2 `blob/put` with SHA-256 verification and `60 s` TTL.
    pub fn put_staged_blob(
        &mut self,
        scope_id: Option<&str>,
        params: BlobPutParams,
        now_ms: Time,
    ) -> Result<BlobPutResult, Waist2Error> {
        let bytes = match params.inline_bytes {
            Some(payload) => payload.0,
            None if params.byte_len == 0 => Vec::new(),
            None => {
                return Err(Waist2Error::hash_mismatch(
                    &params.sha256,
                    "missing_inline_bytes",
                ));
            }
        };

        if bytes.len() as u64 != params.byte_len {
            return Err(Waist2Error::hash_mismatch(
                &params.sha256,
                &format!("byte_len_mismatch:{}", bytes.len()),
            ));
        }

        let actual_sha = sha256_hex(&bytes);
        if !actual_sha.eq_ignore_ascii_case(&params.sha256) {
            return Err(Waist2Error::hash_mismatch(&params.sha256, &actual_sha));
        }

        let explicit_scope = scope_id.or_else(|| extract_scope_from_meta(&params.headers.meta));
        if let Some(sc) = explicit_scope
            && let Err(err) = sanitize_scope_dir(sc)
        {
            return Err(Waist2Error::invalid_receipt_transition(err.to_string()));
        }

        let seq = self.next_staging_seq;
        self.next_staging_seq = self.next_staging_seq.saturating_add(1);
        let stage_id = format!("stg_{}_{}_{}", now_ms.as_millis(), seq, &actual_sha[..16]);
        let blob_ref = format!("blob://staged/{stage_id}");
        let rel_path = PathBuf::from(ATTACHMENTS_DIR_NAME)
            .join(STAGING_DIR_NAME)
            .join(&stage_id);
        let abs_path = self.root_dir.join(&rel_path);

        let key = self.master_key.staging_aead_key();
        let aad = build_staging_aad(explicit_scope, &blob_ref, &actual_sha);
        let sealed = encrypt_aead(&key, &aad, &bytes)
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
        write_atomic_bytes(&abs_path, &sealed)
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;

        let expires_at_ms = now_ms.saturating_add_ms(STAGING_TTL_MS);
        self.conn
            .execute(
                "INSERT INTO staged_blobs
                 (blob_ref, scope_id, sha256_hex, byte_len, rel_path, created_at_ms, expires_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    &blob_ref,
                    explicit_scope,
                    &actual_sha,
                    bytes.len() as i64,
                    rel_path.to_string_lossy().as_ref(),
                    now_ms.as_millis() as i64,
                    expires_at_ms.as_millis() as i64,
                ],
            )
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;

        Ok(BlobPutResult {
            blob_ref,
            sha256: actual_sha,
            expires_at_ms,
        })
    }

    /// Retrieves a staged or committed blob via Waist-2 `blob/get`, enforcing scope isolation
    /// (zero cross-tenant oracle), staging TTL, AEAD integrity, and optional `expected_sha256`.
    pub fn get_blob(
        &self,
        scope_id: Option<&str>,
        params: BlobGetParams,
        now_ms: Time,
    ) -> Result<BlobGetResult, Waist2Error> {
        let requester_scope = scope_id.or_else(|| extract_scope_from_meta(&params.headers.meta));

        // 1. Check staged blobs first.
        if params.blob_ref.starts_with("blob://staged/") {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT scope_id, sha256_hex, byte_len, rel_path, expires_at_ms
                     FROM staged_blobs WHERE blob_ref = ?1",
                )
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
            let mut rows = stmt
                .query(params![&params.blob_ref])
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
            let Some(row) = rows
                .next()
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?
            else {
                return Err(Waist2Error::invalid_receipt_transition(format!(
                    "blob '{}' not found",
                    params.blob_ref
                )));
            };

            let bound_scope: Option<String> = row
                .get(0)
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
            let stored_sha: String = row
                .get(1)
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
            let byte_len: i64 = row
                .get(2)
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
            let rel_path: String = row
                .get(3)
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
            let expires_ms: i64 = row
                .get(4)
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;

            // Zero cross-tenant oracle: mismatched scope receives the exact same not-found error.
            if let Some(bound) = bound_scope.as_deref()
                && requester_scope != Some(bound)
            {
                return Err(Waist2Error::invalid_receipt_transition(format!(
                    "blob '{}' not found",
                    params.blob_ref
                )));
            }
            if now_ms > Time::from_millis(expires_ms as u64) {
                return Err(Waist2Error::invalid_receipt_transition(format!(
                    "blob '{}' staging TTL expired",
                    params.blob_ref
                )));
            }

            let abs_path = self.root_dir.join(&rel_path);
            let sealed = fs::read(&abs_path)
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
            let key = self.master_key.staging_aead_key();
            let aad = build_staging_aad(bound_scope.as_deref(), &params.blob_ref, &stored_sha);
            let plaintext = decrypt_aead(&key, &aad, &sealed, &abs_path)
                .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;

            let actual_sha = sha256_hex(&plaintext);
            if let Some(expected) = &params.expected_sha256
                && !actual_sha.eq_ignore_ascii_case(expected)
            {
                return Err(Waist2Error::hash_mismatch(expected, &actual_sha));
            }
            return Ok(BlobGetResult {
                blob_ref: params.blob_ref,
                sha256: actual_sha,
                byte_len: byte_len as u64,
                bytes: Some(OpaquePayload::from_bytes(plaintext)),
            });
        }

        // 2. Committed custody lookup strictly scoped to `requester_scope`.
        let Some(req_scope) = requester_scope else {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "blob '{}' not found",
                params.blob_ref
            )));
        };

        let mut stmt = self
            .conn
            .prepare(
                "SELECT blob_hex, sha256_hex, byte_len, rel_path, expires_at_ms
                 FROM attachment_blobs WHERE scope_id = ?1 AND handle_id = ?2",
            )
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
        let mut rows = stmt
            .query(params![req_scope, &params.blob_ref])
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
        let Some(row) = rows
            .next()
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?
        else {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "blob '{}' not found",
                params.blob_ref
            )));
        };

        let blob_hex: String = row
            .get(0)
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
        let stored_sha: String = row
            .get(1)
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
        let byte_len: i64 = row
            .get(2)
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
        let rel_path: String = row
            .get(3)
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;
        let expires_ms: i64 = row
            .get(4)
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;

        if now_ms > Time::from_millis(expires_ms as u64) {
            return Err(Waist2Error::invalid_receipt_transition(format!(
                "blob '{}' expired",
                params.blob_ref
            )));
        }

        let plaintext = self
            .decrypt_custody_file(req_scope, &blob_hex, &self.root_dir.join(&rel_path))
            .map_err(|e| Waist2Error::invalid_receipt_transition(e.to_string()))?;

        let actual_sha = sha256_hex(&plaintext);
        if !actual_sha.eq_ignore_ascii_case(&stored_sha) {
            return Err(Waist2Error::hash_mismatch(&stored_sha, &actual_sha));
        }
        if let Some(expected) = &params.expected_sha256
            && !actual_sha.eq_ignore_ascii_case(expected)
        {
            return Err(Waist2Error::hash_mismatch(expected, &actual_sha));
        }

        Ok(BlobGetResult {
            blob_ref: params.blob_ref,
            sha256: actual_sha,
            byte_len: byte_len as u64,
            bytes: Some(OpaquePayload::from_bytes(plaintext)),
        })
    }

    /// Promotes a staged `blob/put` blob into permanent scope custody (`attachments/<scope_id>/<prefix>/<hex>`)
    /// and wipes the staging copy.
    pub fn promote_staged_blob(
        &mut self,
        blob_ref: &str,
        target_scope_id: &str,
        actor_id: &ActorId,
        event_id: Option<EventId>,
        modality_slot: ModalitySlot,
        now_ms: Time,
    ) -> Result<AttachmentHandle, AttachmentCustodyError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT scope_id, sha256_hex, rel_path, expires_at_ms
                 FROM staged_blobs WHERE blob_ref = ?1",
            )
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let mut rows = stmt
            .query(params![blob_ref])
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let Some(row) = rows
            .next()
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?
        else {
            return Err(AttachmentCustodyError::NotFound {
                scope_id: target_scope_id.to_owned(),
                handle_id: blob_ref.to_owned(),
            });
        };

        let bound_scope: Option<String> = row
            .get(0)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let stored_sha: String = row
            .get(1)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let rel_path: String = row
            .get(2)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let expires_ms: i64 = row
            .get(3)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        drop(rows);
        drop(stmt);

        if let Some(bound) = bound_scope.as_deref()
            && bound != target_scope_id
        {
            return Err(AttachmentCustodyError::NotFound {
                scope_id: target_scope_id.to_owned(),
                handle_id: blob_ref.to_owned(),
            });
        }
        let expires_at = Time::from_millis(expires_ms as u64);
        if now_ms > expires_at {
            return Err(AttachmentCustodyError::Expired {
                handle_id: blob_ref.to_owned(),
                expires_at_ms: expires_at,
                now_ms,
            });
        }

        let abs_stage_path = self.root_dir.join(&rel_path);
        let sealed = fs::read(&abs_stage_path).map_err(|e| AttachmentCustodyError::Io {
            path: abs_stage_path.display().to_string(),
            detail: e.to_string(),
        })?;
        let key = self.master_key.staging_aead_key();
        let aad = build_staging_aad(bound_scope.as_deref(), blob_ref, &stored_sha);
        let mut plaintext = decrypt_aead(&key, &aad, &sealed, &abs_stage_path)?;

        let handle = self.persist_scoped_blob(
            target_scope_id,
            actor_id,
            event_id,
            modality_slot,
            &plaintext,
            now_ms,
            now_ms.saturating_add_ms(DEFAULT_CUSTODY_TTL_MS),
        )?;
        zero_bytes(&mut plaintext);

        self.conn
            .execute(
                "DELETE FROM staged_blobs WHERE blob_ref = ?1",
                params![blob_ref],
            )
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let _ = secure_overwrite_and_unlink(&abs_stage_path)?;
        Ok(handle)
    }

    /// Overwrites and unlinks all staged blobs whose `60 s` TTL has elapsed (`now_ms > expires_at_ms`).
    pub fn purge_expired_staging(&mut self, now_ms: Time) -> Result<usize, AttachmentCustodyError> {
        let mut stmt = self
            .conn
            .prepare("SELECT blob_ref, rel_path FROM staged_blobs WHERE expires_at_ms < ?1")
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let rows = stmt
            .query_map(params![now_ms.as_millis() as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let mut expired = Vec::new();
        for item in rows {
            expired.push(item.map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?);
        }
        drop(stmt);

        if expired.is_empty() {
            return Ok(0);
        }

        for (blob_ref, rel_path) in &expired {
            let _ = secure_overwrite_and_unlink(&self.root_dir.join(rel_path))?;
            self.conn
                .execute(
                    "DELETE FROM staged_blobs WHERE blob_ref = ?1",
                    params![blob_ref],
                )
                .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        }
        Ok(expired.len())
    }

    /// Fetches and decrypts bytes for `handle_id` strictly within `scope_id`, verifying
    /// TTL, AEAD tag, and keyed-BLAKE3 digest, and populating the decoded cache.
    pub fn fetch_scoped_bytes(
        &mut self,
        scope_id: &str,
        handle_id: &str,
        now_ms: Time,
    ) -> Result<Vec<u8>, AttachmentCustodyError> {
        let cache_key = (scope_id.to_owned(), handle_id.to_owned());
        if let Some(cached) = self.decoded_cache.get(&cache_key) {
            return Ok(cached.clone());
        }

        let mut stmt = self
            .conn
            .prepare(
                "SELECT blob_hex, rel_path, expires_at_ms
                 FROM attachment_blobs WHERE scope_id = ?1 AND handle_id = ?2",
            )
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let mut rows = stmt
            .query(params![scope_id, handle_id])
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let Some(row) = rows
            .next()
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?
        else {
            return Err(AttachmentCustodyError::NotFound {
                scope_id: scope_id.to_owned(),
                handle_id: handle_id.to_owned(),
            });
        };

        let blob_hex: String = row
            .get(0)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let rel_path: String = row
            .get(1)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let expires_ms: i64 = row
            .get(2)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        drop(rows);
        drop(stmt);

        let expires_at = Time::from_millis(expires_ms as u64);
        if now_ms > expires_at {
            return Err(AttachmentCustodyError::Expired {
                handle_id: handle_id.to_owned(),
                expires_at_ms: expires_at,
                now_ms,
            });
        }

        let abs_path = self.root_dir.join(&rel_path);
        let plaintext = self.decrypt_custody_file(scope_id, &blob_hex, &abs_path)?;
        self.decoded_cache.insert(cache_key, plaintext.clone());
        Ok(plaintext)
    }

    /// Reads and decrypts a custody file without modifying `decoded_cache`.
    pub fn fetch_scoped_bytes_uncached(
        &self,
        scope_id: &str,
        handle_id: &str,
        now_ms: Time,
    ) -> Result<Vec<u8>, AttachmentCustodyError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT blob_hex, rel_path, expires_at_ms
                 FROM attachment_blobs WHERE scope_id = ?1 AND handle_id = ?2",
            )
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let mut rows = stmt
            .query(params![scope_id, handle_id])
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let Some(row) = rows
            .next()
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?
        else {
            return Err(AttachmentCustodyError::NotFound {
                scope_id: scope_id.to_owned(),
                handle_id: handle_id.to_owned(),
            });
        };
        let blob_hex: String = row
            .get(0)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let rel_path: String = row
            .get(1)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let expires_ms: i64 = row
            .get(2)
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let expires_at = Time::from_millis(expires_ms as u64);
        if now_ms > expires_at {
            return Err(AttachmentCustodyError::Expired {
                handle_id: handle_id.to_owned(),
                expires_at_ms: expires_at,
                now_ms,
            });
        }
        self.decrypt_custody_file(scope_id, &blob_hex, &self.root_dir.join(&rel_path))
    }

    /// Returns the number of currently cached decoded entries.
    #[must_use]
    pub fn decoded_cache_len(&self) -> usize {
        self.decoded_cache.len()
    }

    /// Returns the current `attachment_refs` reference count for `(scope_id, handle_id)`.
    pub fn ref_count(
        &self,
        scope_id: &str,
        handle_id: &str,
    ) -> Result<usize, AttachmentCustodyError> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM attachment_refs WHERE scope_id = ?1 AND handle_id = ?2",
                params![scope_id, handle_id],
                |row| row.get(0),
            )
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        Ok(usize::try_from(count.max(0)).unwrap_or(0))
    }

    /// Returns the relative custody path stored in SQLite for `(scope_id, handle_id)`, if present.
    pub fn rel_path_for(
        &self,
        scope_id: &str,
        handle_id: &str,
    ) -> Result<Option<PathBuf>, AttachmentCustodyError> {
        let mut stmt = self
            .conn
            .prepare("SELECT rel_path FROM attachment_blobs WHERE scope_id = ?1 AND handle_id = ?2")
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        let mut rows = stmt
            .query(params![scope_id, handle_id])
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
        if let Some(row) = rows
            .next()
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?
        {
            let p: String = row
                .get(0)
                .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
            return Ok(Some(PathBuf::from(p)));
        }
        Ok(None)
    }

    /// Synchronously executes `hard_forget` across `attachment_refs`, `attachment_blobs`,
    /// `staged_blobs`, zero-ref files on disk (overwriting with zeros and unlinking),
    /// in-memory `decoded_cache` (zeroizing), and SQLite `VACUUM` + WAL truncate.
    pub fn hard_forget(
        &mut self,
        event_ids: &[EventId],
        actor_ids: &[ActorId],
        scope_ids: &[&str],
    ) -> Result<HardForgetAttachmentReport, AttachmentCustodyError> {
        let mut target_scopes: BTreeSet<String> =
            scope_ids.iter().map(|s| (*s).to_owned()).collect();
        for a in actor_ids {
            target_scopes.insert(a.as_str().to_owned());
        }

        let mut report = HardForgetAttachmentReport::default();

        let (zero_ref_blobs, staged_to_wipe) = {
            let tx = self
                .conn
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;

            let mut affected_pairs: BTreeSet<(String, String)> = BTreeSet::new();

            for eid in event_ids {
                let mut stmt = tx
                    .prepare(
                        "SELECT DISTINCT scope_id, blob_hex FROM attachment_refs WHERE event_id = ?1",
                    )
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                let rows = stmt
                    .query_map(params![eid.as_u64() as i64], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                for item in rows {
                    affected_pairs
                        .insert(item.map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?);
                }
                drop(stmt);

                let deleted = tx
                    .execute(
                        "DELETE FROM attachment_refs WHERE event_id = ?1",
                        params![eid.as_u64() as i64],
                    )
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                report.removed_refs += deleted;
            }

            for aid in actor_ids {
                let mut stmt = tx
                    .prepare(
                        "SELECT DISTINCT scope_id, blob_hex FROM attachment_refs WHERE actor_id = ?1",
                    )
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                let rows = stmt
                    .query_map(params![aid.as_str()], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                for item in rows {
                    affected_pairs
                        .insert(item.map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?);
                }
                drop(stmt);

                let deleted = tx
                    .execute(
                        "DELETE FROM attachment_refs WHERE actor_id = ?1",
                        params![aid.as_str()],
                    )
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                report.removed_refs += deleted;
            }

            for sc in scope_ids {
                let mut stmt = tx
                    .prepare(
                        "SELECT DISTINCT scope_id, blob_hex FROM attachment_refs WHERE scope_id = ?1",
                    )
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                let rows = stmt
                    .query_map(params![*sc], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                for item in rows {
                    affected_pairs
                        .insert(item.map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?);
                }
                drop(stmt);

                let deleted = tx
                    .execute(
                        "DELETE FROM attachment_refs WHERE scope_id = ?1",
                        params![*sc],
                    )
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                report.removed_refs += deleted;
            }

            // Collect all zero-ref blobs among affected `(scope_id, blob_hex)` pairs.
            let mut zero_refs: Vec<(String, String, String, String)> = Vec::new();
            for (sc, hex) in &affected_pairs {
                let remaining: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM attachment_refs WHERE scope_id = ?1 AND blob_hex = ?2",
                        params![sc, hex],
                        |r| r.get(0),
                    )
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                if remaining == 0 {
                    let mut stmt = tx
                        .prepare(
                            "SELECT handle_id, rel_path FROM attachment_blobs
                             WHERE scope_id = ?1 AND blob_hex = ?2",
                        )
                        .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                    let rows = stmt
                        .query_map(params![sc, hex], |r| {
                            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                        })
                        .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                    for item in rows {
                        let (hid, rpath) =
                            item.map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                        zero_refs.push((sc.clone(), hex.clone(), hid, rpath));
                    }
                    drop(stmt);

                    tx.execute(
                        "DELETE FROM attachment_blobs WHERE scope_id = ?1 AND blob_hex = ?2",
                        params![sc, hex],
                    )
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                }
            }

            // Also collect any staged blobs bound to erased scopes.
            let mut staged = Vec::new();
            for sc in &target_scopes {
                let mut stmt = tx
                    .prepare("SELECT blob_ref, rel_path FROM staged_blobs WHERE scope_id = ?1")
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                let rows = stmt
                    .query_map(params![sc], |r| {
                        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                    })
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
                for item in rows {
                    staged.push(item.map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?);
                }
                drop(stmt);

                tx.execute("DELETE FROM staged_blobs WHERE scope_id = ?1", params![sc])
                    .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
            }

            tx.commit()
                .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;
            (zero_refs, staged)
        };

        // Zeroize decoded caches and overwrite/unlink zero-ref files synchronously.
        for (sc, _hex, hid, rel_path) in zero_ref_blobs {
            if let Some(mut cached) = self.decoded_cache.remove(&(sc.clone(), hid.clone())) {
                zero_bytes(&mut cached);
                report.wiped_cache_entries += 1;
            }
            let abs = self.root_dir.join(&rel_path);
            report.overwritten_bytes = report
                .overwritten_bytes
                .saturating_add(secure_overwrite_and_unlink(&abs)?);
            prune_empty_parent_dirs(&abs, &self.attachments_dir());
            report.wiped_blobs += 1;
        }

        for (_bref, rel_path) in staged_to_wipe {
            let abs = self.root_dir.join(&rel_path);
            report.overwritten_bytes = report
                .overwritten_bytes
                .saturating_add(secure_overwrite_and_unlink(&abs)?);
            report.wiped_staged_blobs += 1;
        }

        // Scrub SQLite pages, freelist, and WAL completely.
        self.conn
            .execute_batch(
                "PRAGMA secure_delete = ON;
                 VACUUM;
                 PRAGMA secure_delete = ON;
                 PRAGMA wal_checkpoint(TRUNCATE);",
            )
            .map_err(|e| AttachmentCustodyError::Sqlite(e.to_string()))?;

        Ok(report)
    }

    fn decrypt_custody_file(
        &self,
        scope_id: &str,
        expected_blob_hex: &str,
        abs_path: &Path,
    ) -> Result<Vec<u8>, AttachmentCustodyError> {
        let sealed = fs::read(abs_path).map_err(|e| AttachmentCustodyError::Io {
            path: abs_path.display().to_string(),
            detail: e.to_string(),
        })?;
        let aead_key = self.master_key.scope_aead_key(scope_id);
        let aad = build_custody_aad(scope_id, expected_blob_hex);
        let plaintext = decrypt_aead(&aead_key, &aad, &sealed, abs_path)?;

        let salt = self.master_key.scope_salt(scope_id);
        let actual_hex = blake3::keyed_hash(&salt, &plaintext).to_hex().to_string();
        if actual_hex != expected_blob_hex {
            return Err(AttachmentCustodyError::AeadTamper {
                path: abs_path.display().to_string(),
                detail: format!(
                    "keyed BLAKE3 mismatch after decrypt: expected {expected_blob_hex}, got {actual_hex}"
                ),
            });
        }
        Ok(plaintext)
    }
}

/// Scoped read-only adapter implementing [`monoagens_reason::AttachmentBlobStore`]
/// so `crates/reason` resolves authorized [`AttachmentHandle`]s strictly within one tenant scope.
#[derive(Clone, Debug)]
pub struct ScopedAttachmentBlobReader {
    store: Arc<Mutex<AttachmentStore>>,
    scope_id: String,
    now_ms: Time,
}

impl ScopedAttachmentBlobReader {
    /// Creates a scoped reader bound to `scope_id` at `now_ms`.
    #[must_use]
    pub fn new(
        store: Arc<Mutex<AttachmentStore>>,
        scope_id: impl Into<String>,
        now_ms: Time,
    ) -> Self {
        Self {
            store,
            scope_id: scope_id.into(),
            now_ms,
        }
    }
}

impl AttachmentBlobStore for ScopedAttachmentBlobReader {
    fn fetch_bytes(&self, handle: &AttachmentHandle) -> Result<Vec<u8>, AttachmentError> {
        let mut guard = self.store.lock().expect("attachment store lock");
        guard
            .fetch_scoped_bytes(&self.scope_id, &handle.handle_id, self.now_ms)
            .map_err(|err| match err {
                AttachmentCustodyError::NotFound { handle_id, .. } => {
                    AttachmentError::NotFound { handle_id }
                }
                AttachmentCustodyError::Expired {
                    handle_id,
                    expires_at_ms,
                    now_ms,
                } => AttachmentError::Expired {
                    handle_id,
                    expires_at_ms,
                    now: now_ms,
                },
                other => AttachmentError::StoreBackend {
                    handle_id: handle.handle_id.clone(),
                    detail: other.to_string(),
                },
            })
    }
}

/// Gateway [`AttachmentCustodySeam`] bridge connecting [`AttachmentStore`] and [`Clock`]
/// into Waist-2 `percept/emit`, `blob/put`, `blob/get`, and `hard_forget`.
#[derive(Clone)]
pub struct GatewayAttachmentBridge {
    store: Arc<Mutex<AttachmentStore>>,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for GatewayAttachmentBridge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GatewayAttachmentBridge").finish()
    }
}

impl GatewayAttachmentBridge {
    /// Creates a new [`GatewayAttachmentBridge`] wrapping `store` and `clock`.
    #[must_use]
    pub fn new(store: Arc<Mutex<AttachmentStore>>, clock: Arc<dyn Clock>) -> Self {
        Self { store, clock }
    }

    /// Returns the shared [`AttachmentStore`] handle.
    #[must_use]
    pub fn store(&self) -> &Arc<Mutex<AttachmentStore>> {
        &self.store
    }
}

impl AttachmentCustodySeam for GatewayAttachmentBridge {
    fn ingest_percept_attachments(
        &mut self,
        _body_id: &BodyId,
        percept: &mut Percept,
    ) -> Result<(), SeamError> {
        let mut guard = self.store.lock().expect("attachment store lock");
        let now = self.clock.now();
        let _ = guard
            .purge_expired_staging(now)
            .map_err(|e| SeamError::new("attachments", e.to_string()))?;

        if percept.payload.len() > MAX_INLINE_ATTACHMENT_BYTES {
            return Err(SeamError::new(
                "attachments",
                format!(
                    "inline payload {} bytes exceeds {} KiB limit",
                    percept.payload.len(),
                    MAX_INLINE_ATTACHMENT_BYTES / 1024
                ),
            ));
        }

        let scope_id = percept.actor.as_str().to_owned();

        // Promote any staged `blob://staged/...` handles into scope custody.
        for handle in &mut percept.attachments {
            if handle.handle_id.starts_with("blob://staged/") {
                let promoted = guard
                    .promote_staged_blob(
                        &handle.handle_id,
                        &scope_id,
                        &percept.actor,
                        None,
                        handle.modality_slot,
                        now,
                    )
                    .map_err(|e| SeamError::new("attachments", e.to_string()))?;
                *handle = promoted;
            }
        }

        // Strip inline media payloads (`f_modal.is_some()` or non-UTF-8 binary) into opaque handles.
        let is_binary =
            !percept.payload.is_empty() && std::str::from_utf8(percept.payload.as_bytes()).is_err();
        if !percept.payload.is_empty() && (percept.f_modal.is_some() || is_binary) {
            let slot = percept
                .f_modal
                .as_ref()
                .map_or(ModalitySlot::SensorBlob, |fm| {
                    infer_modality_from_fmodal(fm.0[0])
                });
            guard
                .strip_inline_media_from_percept(&scope_id, slot, percept)
                .map_err(|e| SeamError::new("attachments", e.to_string()))?;
        }

        Ok(())
    }

    fn handle_blob_put(&mut self, params: BlobPutParams) -> Result<BlobPutResult, Waist2Error> {
        let mut guard = self.store.lock().expect("attachment store lock");
        let now = self.clock.now();
        let _ = guard.purge_expired_staging(now);
        guard.put_staged_blob(None, params, now)
    }

    fn handle_blob_get(&self, params: BlobGetParams) -> Result<BlobGetResult, Waist2Error> {
        let guard = self.store.lock().expect("attachment store lock");
        guard.get_blob(None, params, self.clock.now())
    }

    fn cascade_hard_forget(
        &mut self,
        event_ids: &[EventId],
        actor_ids: &[ActorId],
    ) -> Result<(), SeamError> {
        let mut guard = self.store.lock().expect("attachment store lock");
        guard
            .hard_forget(event_ids, actor_ids, &[])
            .map(|_| ())
            .map_err(|e| SeamError::new("attachments", e.to_string()))
    }
}

fn infer_modality_from_fmodal(slot_val: f32) -> ModalitySlot {
    let code = slot_val.round() as i32;
    match code {
        1 => ModalitySlot::Image,
        2 => ModalitySlot::AudioVoice,
        3 => ModalitySlot::Video,
        _ => ModalitySlot::SensorBlob,
    }
}

fn modality_to_str(slot: ModalitySlot) -> &'static str {
    match slot {
        ModalitySlot::Image => "image",
        ModalitySlot::AudioVoice => "audio_voice",
        ModalitySlot::Video => "video",
        ModalitySlot::SensorBlob => "sensor_blob",
    }
}

fn extract_scope_from_meta(meta: &BTreeMap<String, MetaValue>) -> Option<&str> {
    meta.get(NAMESPACED_SCOPE_META_KEY)
        .or_else(|| meta.get(SCOPE_META_KEY))
        .and_then(|v| match v {
            MetaValue::String(s) if !s.trim().is_empty() => Some(s.trim()),
            _ => None,
        })
}

fn sanitize_scope_dir(scope_id: &str) -> Result<String, AttachmentCustodyError> {
    if scope_id.trim().is_empty() {
        return Err(AttachmentCustodyError::InvalidScope(
            "scope_id must not be empty".to_owned(),
        ));
    }
    if scope_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Ok(scope_id.to_owned());
    }
    let hash = blake3::hash(scope_id.as_bytes()).to_hex();
    Ok(format!("scope_{}", &hash[..32]))
}

fn derive_scoped_subkey(master: &[u8; 32], domain: &[u8], scope_id: &str) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_keyed(master);
    hasher.update(domain);
    let sb = scope_id.as_bytes();
    hasher.update(&u32::try_from(sb.len()).unwrap_or(u32::MAX).to_le_bytes());
    hasher.update(sb);
    *hasher.finalize().as_bytes()
}

fn build_custody_aad(scope_id: &str, blob_hex: &str) -> Vec<u8> {
    let sb = scope_id.as_bytes();
    let hb = blob_hex.as_bytes();
    let mut aad = Vec::with_capacity(32 + sb.len() + hb.len());
    aad.extend_from_slice(b"monoagens.attachment.v1");
    aad.extend_from_slice(&u32::try_from(sb.len()).unwrap_or(u32::MAX).to_le_bytes());
    aad.extend_from_slice(sb);
    aad.extend_from_slice(&u32::try_from(hb.len()).unwrap_or(u32::MAX).to_le_bytes());
    aad.extend_from_slice(hb);
    aad
}

fn build_staging_aad(scope_id: Option<&str>, blob_ref: &str, sha256_hex: &str) -> Vec<u8> {
    let sc = scope_id.unwrap_or("");
    let mut aad = Vec::with_capacity(48 + sc.len() + blob_ref.len() + sha256_hex.len());
    aad.extend_from_slice(b"monoagens.staging.v1");
    aad.extend_from_slice(&(sc.len() as u32).to_le_bytes());
    aad.extend_from_slice(sc.as_bytes());
    aad.extend_from_slice(&(blob_ref.len() as u32).to_le_bytes());
    aad.extend_from_slice(blob_ref.as_bytes());
    aad.extend_from_slice(sha256_hex.as_bytes());
    aad
}

fn encrypt_aead(
    key_bytes: &[u8; 32],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, AttachmentCustodyError> {
    let cipher = XChaCha20Poly1305::new(&Key::<XChaCha20Poly1305>::from(*key_bytes));
    let nonce = XNonce::generate();
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| AttachmentCustodyError::AeadTamper {
            path: "<encrypt>".to_owned(),
            detail: "XChaCha20Poly1305 encryption failed".to_owned(),
        })?;
    let mut out = Vec::with_capacity(AEAD_FILE_MAGIC.len() + AEAD_NONCE_LEN + ciphertext.len());
    out.extend_from_slice(AEAD_FILE_MAGIC);
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn decrypt_aead(
    key_bytes: &[u8; 32],
    aad: &[u8],
    sealed: &[u8],
    path: &Path,
) -> Result<Vec<u8>, AttachmentCustodyError> {
    let min_len = AEAD_FILE_MAGIC.len() + AEAD_NONCE_LEN + AEAD_TAG_LEN;
    if sealed.len() < min_len {
        return Err(AttachmentCustodyError::AeadTamper {
            path: path.display().to_string(),
            detail: format!("truncated AEAD blob ({} < {min_len} bytes)", sealed.len()),
        });
    }
    if &sealed[..AEAD_FILE_MAGIC.len()] != AEAD_FILE_MAGIC {
        return Err(AttachmentCustodyError::AeadTamper {
            path: path.display().to_string(),
            detail: "invalid AEAD magic header".to_owned(),
        });
    }
    let nonce_start = AEAD_FILE_MAGIC.len();
    let nonce_end = nonce_start + AEAD_NONCE_LEN;
    let nonce_arr: [u8; AEAD_NONCE_LEN] =
        sealed[nonce_start..nonce_end].try_into().map_err(|_| {
            AttachmentCustodyError::AeadTamper {
                path: path.display().to_string(),
                detail: "invalid nonce length".to_owned(),
            }
        })?;
    let nonce = XNonce::from(nonce_arr);
    let cipher = XChaCha20Poly1305::new(&Key::<XChaCha20Poly1305>::from(*key_bytes));
    cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &sealed[nonce_end..],
                aad,
            },
        )
        .map_err(|_| AttachmentCustodyError::AeadTamper {
            path: path.display().to_string(),
            detail: "AEAD tag verification failed (tampered or wrong scope key)".to_owned(),
        })
}

fn write_atomic_bytes(path: &Path, bytes: &[u8]) -> Result<(), AttachmentCustodyError> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|e| AttachmentCustodyError::Io {
            path: parent.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    let tmp_name = format!(
        ".tmp_{}_{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("blob"),
        std::process::id()
    );
    let tmp_path = path.with_file_name(tmp_name);
    {
        let mut f = fs::File::create(&tmp_path).map_err(|e| AttachmentCustodyError::Io {
            path: tmp_path.display().to_string(),
            detail: e.to_string(),
        })?;
        f.write_all(bytes).map_err(|e| AttachmentCustodyError::Io {
            path: tmp_path.display().to_string(),
            detail: e.to_string(),
        })?;
        f.sync_all().map_err(|e| AttachmentCustodyError::Io {
            path: tmp_path.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    fs::rename(&tmp_path, path).map_err(|e| AttachmentCustodyError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    Ok(())
}

fn secure_overwrite_and_unlink(path: &Path) -> Result<u64, AttachmentCustodyError> {
    if !path.exists() {
        return Ok(0);
    }
    let meta = fs::metadata(path).map_err(|e| AttachmentCustodyError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    let len = meta.len();
    if len > 0 {
        let mut file =
            OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|e| AttachmentCustodyError::Io {
                    path: path.display().to_string(),
                    detail: e.to_string(),
                })?;
        file.seek(SeekFrom::Start(0))
            .map_err(|e| AttachmentCustodyError::Io {
                path: path.display().to_string(),
                detail: e.to_string(),
            })?;
        let zeros = [0u8; 4096];
        let mut rem = len;
        while rem > 0 {
            let chunk = usize::try_from(rem.min(zeros.len() as u64)).unwrap_or(zeros.len());
            file.write_all(&zeros[..chunk])
                .map_err(|e| AttachmentCustodyError::Io {
                    path: path.display().to_string(),
                    detail: e.to_string(),
                })?;
            rem -= chunk as u64;
        }
        file.sync_all().map_err(|e| AttachmentCustodyError::Io {
            path: path.display().to_string(),
            detail: e.to_string(),
        })?;
    }
    fs::remove_file(path).map_err(|e| AttachmentCustodyError::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    Ok(len)
}

fn prune_empty_parent_dirs(file_path: &Path, stop_at: &Path) {
    let mut cur = file_path.parent();
    while let Some(dir) = cur {
        if dir == stop_at || !dir.starts_with(stop_at) {
            break;
        }
        if fs::remove_dir(dir).is_err() {
            break;
        }
        cur = dir.parent();
    }
}

fn zero_bytes(buf: &mut [u8]) {
    buf.fill(0);
}

/// Computes lowercase hex SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut out, "{byte:02x}");
    }
    out
}

fn decode_hex_32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    let bytes = hex.as_bytes();
    for i in 0..32 {
        let hi = from_hex_nibble(bytes[2 * i])?;
        let lo = from_hex_nibble(bytes[2 * i + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

const fn from_hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

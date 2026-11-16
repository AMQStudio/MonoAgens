//! Persistent soul directory layout (`soul.json`, `store/`, and `runs/`).

use std::fs;
use std::path::{Path, PathBuf};

use monoagens_store::{SegmentedStore, StoreConfig};
use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::record::RunRecord;

const SOUL_MANIFEST_FILENAME: &str = "soul.json";

/// Metadata persisted in a Lab soul directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoulManifest {
    pub version: u32,
    pub soul_id: String,
    pub seed: u64,
}

/// Handle to a persistent soul directory holding identity/manifest, segmented store, and run records.
#[derive(Debug, Clone)]
pub struct SoulDir {
    root: PathBuf,
    manifest: SoulManifest,
}

impl SoulDir {
    /// Opens an existing soul directory or initializes a new one at `root`.
    pub fn open_or_init(
        root: impl AsRef<Path>,
        soul_id: impl Into<String>,
        seed: u64,
    ) -> Result<Self, LabError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        fs::create_dir_all(root.join("store"))?;
        fs::create_dir_all(root.join("runs"))?;

        let manifest_path = root.join(SOUL_MANIFEST_FILENAME);
        let manifest = if manifest_path.exists() {
            let raw = fs::read_to_string(&manifest_path)?;
            serde_json::from_str(&raw).map_err(|e| LabError::Serialization(e.to_string()))?
        } else {
            let m = SoulManifest {
                version: 1,
                soul_id: soul_id.into(),
                seed,
            };
            let raw = serde_json::to_string_pretty(&m)
                .map_err(|e| LabError::Serialization(e.to_string()))?;
            fs::write(&manifest_path, raw)?;
            m
        };

        Ok(Self { root, manifest })
    }

    /// Returns the soul directory root path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the loaded soul manifest.
    #[must_use]
    pub const fn manifest(&self) -> &SoulManifest {
        &self.manifest
    }

    /// Opens the underlying durable [`SegmentedStore`] inside this soul directory.
    pub fn open_store(&self) -> Result<SegmentedStore, LabError> {
        let cfg = StoreConfig::new(self.root.join("store"));
        SegmentedStore::open(cfg).map_err(LabError::from)
    }

    /// Persists a [`RunRecord`] encoded with `postcard` under `runs/<run_id>.postcard`.
    pub fn save_run_record(&self, record: &RunRecord) -> Result<PathBuf, LabError> {
        let path = self
            .root
            .join("runs")
            .join(format!("{}.postcard", record.run_id));
        let bytes = record.encode_postcard()?;
        fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Loads a `postcard`-encoded [`RunRecord`] from `path`.
    pub fn load_run_record(path: impl AsRef<Path>) -> Result<RunRecord, LabError> {
        let bytes = fs::read(path.as_ref())?;
        RunRecord::decode_postcard(&bytes)
    }
}

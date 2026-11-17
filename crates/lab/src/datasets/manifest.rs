//! Strict TOML manifest schema, registry consistency checks, and dataset status reporting.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::LabError;
use crate::benchmark::{BenchmarkDatasetSpec, BenchmarkDomain};
use crate::scenario::ensure_not_sealed_path;

/// Literal placeholder used in the manifest for unpinned revisions, SHA-256 hashes, and sizes.
pub const PENDING_PIN: &str = "pending_pin";

/// Default relative path to the owner's local, git-ignored working manifest.
pub const DEFAULT_LOCAL_MANIFEST_PATH: &str = "lab/datasets.manifest.toml";

/// Default relative path to the committed manifest template.
pub const DEFAULT_TEMPLATE_MANIFEST_PATH: &str = "lab/datasets.manifest.example.toml";

/// Distinguishes benchmark datasets from non-corpus scorer model weights in `lab/datasets.manifest.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestEntryKind {
    /// External benchmark dataset tracked 1-to-1 against `standard_benchmark_registry()`.
    #[default]
    Dataset,
    /// Non-corpus scorer model weights used locally by `lab/adapters/eval_helpers`.
    ScorerModel,
}

impl ManifestEntryKind {
    /// Returns `true` for standard benchmark dataset entries.
    #[must_use]
    pub fn is_dataset(&self) -> bool {
        matches!(self, Self::Dataset)
    }
}

impl fmt::Display for ManifestEntryKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Dataset => "dataset",
            Self::ScorerModel => "scorer_model",
        };
        f.write_str(s)
    }
}

/// Supported upstream source kinds in `[[datasets.sources]]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Git,
    Huggingface,
    Url,
    PaperOnly,
}

impl fmt::Display for SourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Git => "git",
            Self::Huggingface => "huggingface",
            Self::Url => "url",
            Self::PaperOnly => "paper_only",
        };
        f.write_str(s)
    }
}

/// Owner license clearance status (`pending` | `cleared` | `refused`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseStatus {
    Pending,
    Cleared,
    Refused,
}

impl fmt::Display for LicenseStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Pending => "pending",
            Self::Cleared => "cleared",
            Self::Refused => "refused",
        };
        f.write_str(s)
    }
}

/// Public availability status of an external dataset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetAvailability {
    Available,
    Unavailable,
    NotMappable,
}

impl fmt::Display for DatasetAvailability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
            Self::NotMappable => "not_mappable",
        };
        f.write_str(s)
    }
}

/// Core behavioral mappability under `R-53`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreApplicability {
    CoreBehavioral,
    NotMappable,
    Unavailable,
}

impl fmt::Display for CoreApplicability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::CoreBehavioral => "core_behavioral",
            Self::NotMappable => "not_mappable",
            Self::Unavailable => "unavailable",
        };
        f.write_str(s)
    }
}

/// Execution lane for a dataset in the scorecard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetLane {
    CoreBehavioral,
    EmbedderProbe,
}

impl fmt::Display for DatasetLane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::CoreBehavioral => "core_behavioral",
            Self::EmbedderProbe => "embedder_probe",
        };
        f.write_str(s)
    }
}

/// Contamination exposure classification (`R-52`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContaminationRisk {
    VeryLow,
    Low,
    Moderate,
    High,
    Severe,
}

impl fmt::Display for ContaminationRisk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::VeryLow => "very_low",
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
            Self::Severe => "severe",
        };
        f.write_str(s)
    }
}

/// Checksum provenance state (`source` | `owner` | `pending` | `na`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumsRecorded {
    Source,
    Owner,
    Pending,
    Na,
}

impl fmt::Display for ChecksumsRecorded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Source => "source",
            Self::Owner => "owner",
            Self::Pending => "pending",
            Self::Na => "na",
        };
        f.write_str(s)
    }
}

/// Optional deterministic archive extraction format for fetched files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchiveKind {
    None,
    TarGz,
    Zip,
}

/// Pinned SHA-256 digest or `"pending_pin"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinnedSha256 {
    PendingPin,
    Sha256(String),
}

impl PinnedSha256 {
    #[must_use]
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::PendingPin)
    }

    #[must_use]
    pub fn as_hex(&self) -> Option<&str> {
        match self {
            Self::PendingPin => None,
            Self::Sha256(hex) => Some(hex.as_str()),
        }
    }
}

impl fmt::Display for PinnedSha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PendingPin => f.write_str(PENDING_PIN),
            Self::Sha256(h) => f.write_str(h),
        }
    }
}

impl Serialize for PinnedSha256 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::PendingPin => serializer.serialize_str(PENDING_PIN),
            Self::Sha256(h) => serializer.serialize_str(h),
        }
    }
}

impl<'de> Deserialize<'de> for PinnedSha256 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        if raw == PENDING_PIN {
            return Ok(Self::PendingPin);
        }
        if raw.len() == 64 && raw.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Ok(Self::Sha256(raw));
        }
        Err(serde::de::Error::custom(format!(
            "invalid sha256 '{raw}': expected 'pending_pin' or 64 lowercase hex chars"
        )))
    }
}

/// Pinned byte size or `"pending_pin"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinnedSize {
    PendingPin,
    Bytes(u64),
}

impl PinnedSize {
    #[must_use]
    pub fn is_pending(self) -> bool {
        matches!(self, Self::PendingPin)
    }

    #[must_use]
    pub fn as_bytes(self) -> Option<u64> {
        match self {
            Self::PendingPin => None,
            Self::Bytes(n) => Some(n),
        }
    }
}

impl fmt::Display for PinnedSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PendingPin => f.write_str(PENDING_PIN),
            Self::Bytes(n) => write!(f, "{n}"),
        }
    }
}

impl Serialize for PinnedSize {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::PendingPin => serializer.serialize_str(PENDING_PIN),
            Self::Bytes(n) => serializer.serialize_u64(*n),
        }
    }
}

impl<'de> Deserialize<'de> for PinnedSize {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = PinnedSize;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("'pending_pin' or an unsigned integer byte size")
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(PinnedSize::Bytes(v))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v < 0 {
                    return Err(E::custom("file size cannot be negative"));
                }
                Ok(PinnedSize::Bytes(v as u64))
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if v == PENDING_PIN {
                    Ok(PinnedSize::PendingPin)
                } else {
                    Err(E::custom(format!(
                        "invalid file size '{v}': expected 'pending_pin' or integer bytes"
                    )))
                }
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

/// Upstream source declaration in `[[datasets.sources]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetSource {
    pub kind: SourceKind,
    pub location: String,
    pub revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

/// License metadata in `[datasets.license]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetLicense {
    pub spdx: String,
    pub terms: String,
    pub status: LicenseStatus,
}

/// One expected file entry in `[[datasets.files]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_role: Option<String>,
    pub sha256: PinnedSha256,
    pub size: PinnedSize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<ArchiveKind>,
}

/// One external dataset or scorer-model entry in `[[datasets]]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetManifestEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "ManifestEntryKind::is_dataset")]
    pub kind: ManifestEntryKind,
    pub domain: BenchmarkDomain,
    pub relative_dir: String,
    #[serde(default)]
    pub sources: Vec<DatasetSource>,
    pub license: DatasetLicense,
    pub availability: DatasetAvailability,
    pub core_applicability: CoreApplicability,
    pub lane: DatasetLane,
    pub contamination_risk: ContaminationRisk,
    pub contamination_note: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<String>,
    #[serde(default)]
    pub files: Vec<ManifestFile>,
    pub checksums_recorded: ChecksumsRecorded,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksums_recorded_date: Option<String>,
    pub converter_version: String,
    pub scorer_version: String,
}

impl DatasetManifestEntry {
    /// Collects human-readable `pending_pin` items for sources and files.
    #[must_use]
    pub fn pending_pin_items(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, src) in self.sources.iter().enumerate() {
            if src.revision == PENDING_PIN {
                out.push(format!("sources[{i}].revision"));
            }
        }
        for file in &self.files {
            if file.sha256.is_pending() {
                out.push(format!("files[{}].sha256", file.path));
            }
            if file.size.is_pending() {
                out.push(format!("files[{}].size", file.path));
            }
        }
        out
    }

    /// Returns `true` when any source or file still awaits pinning.
    #[must_use]
    pub fn has_pending_pin(&self) -> bool {
        !self.pending_pin_items().is_empty()
    }
}

/// Root TOML document for `lab/datasets.manifest.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetsManifest {
    pub schema_version: String,
    pub datasets: Vec<DatasetManifestEntry>,
}

impl DatasetsManifest {
    /// Finds a dataset entry by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&DatasetManifestEntry> {
        self.datasets.iter().find(|d| d.id == id)
    }

    /// Finds a mutable dataset entry by id.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut DatasetManifestEntry> {
        self.datasets.iter_mut().find(|d| d.id == id)
    }
}

/// Strictly parses a manifest TOML string, failing on any unknown key or invalid field value.
pub fn parse_manifest_strict(toml_str: &str) -> Result<DatasetsManifest, LabError> {
    toml::from_str::<DatasetsManifest>(toml_str)
        .map_err(|e| LabError::Serialization(format!("invalid dataset manifest TOML: {e}")))
}

/// Resolves the template manifest path, preferring an explicit path, then `lab/datasets.manifest.example.toml`,
/// then the workspace template path relative to `CARGO_MANIFEST_DIR`.
#[must_use]
pub fn resolve_template_manifest_path(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    let cwd_rel = PathBuf::from(DEFAULT_TEMPLATE_MANIFEST_PATH);
    if cwd_rel.is_file() {
        return cwd_rel;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../lab/datasets.manifest.example.toml")
}

/// Initializes a local working manifest at `target_path` by copying `template_path`.
/// Refuses without writing if `target_path` already exists.
pub fn init_local_manifest_from_template(
    template_path: &Path,
    target_path: &Path,
) -> Result<usize, LabError> {
    ensure_not_sealed_path(template_path)?;
    ensure_not_sealed_path(target_path)?;

    if fs::symlink_metadata(target_path).is_ok() {
        return Err(LabError::Protocol(format!(
            "refusing to overwrite existing local manifest '{}'",
            target_path.display()
        )));
    }

    let raw = fs::read_to_string(template_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            LabError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("manifest template '{}' not found", template_path.display()),
            ))
        } else {
            LabError::Io(e)
        }
    })?;
    let parsed = parse_manifest_strict(&raw)?;

    if let Some(parent) = target_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target_path)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                LabError::Protocol(format!(
                    "refusing to overwrite existing local manifest '{}'",
                    target_path.display()
                ))
            } else {
                LabError::Io(e)
            }
        })?;
    file.write_all(raw.as_bytes())?;
    Ok(parsed.datasets.len())
}

/// Loads and strictly parses a manifest file, refusing any `sealed` path component.
/// Returns a clear error naming `monoagens-lab datasets init` when the manifest file is absent.
pub fn load_manifest_from_path(path: &Path) -> Result<DatasetsManifest, LabError> {
    ensure_not_sealed_path(path)?;
    let raw = fs::read_to_string(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            LabError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "local dataset manifest '{}' is absent; run `monoagens-lab datasets init` to create it from `{DEFAULT_TEMPLATE_MANIFEST_PATH}`",
                    path.display()
                ),
            ))
        } else {
            LabError::Io(e)
        }
    })?;
    parse_manifest_strict(&raw)
}

/// Writes a manifest deterministically back to `path`, refusing any `sealed` path component.
pub fn save_manifest_to_path(manifest: &DatasetsManifest, path: &Path) -> Result<(), LabError> {
    ensure_not_sealed_path(path)?;
    let rendered = toml::to_string_pretty(manifest)
        .map_err(|e| LabError::Serialization(format!("failed to serialize manifest: {e}")))?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, rendered)?;
    Ok(())
}

/// Verifies that the manifest external dataset id set matches `registry`'s external dataset id set
/// (`spec.requires_external_corpus == true`), failing loudly and naming all mismatched ids.
/// Entries with `kind == ManifestEntryKind::ScorerModel` are validated separately and excluded from
/// the benchmark dataset count.
pub fn check_registry_manifest_consistency(
    manifest: &DatasetsManifest,
    registry: &[BenchmarkDatasetSpec],
) -> Result<(), LabError> {
    let mut reg_ids: BTreeSet<&str> = BTreeSet::new();
    let mut reg_map: BTreeMap<&str, &BenchmarkDatasetSpec> = BTreeMap::new();
    for spec in registry {
        if spec.requires_external_corpus {
            reg_ids.insert(spec.id.as_str());
            reg_map.insert(spec.id.as_str(), spec);
        }
    }

    let mut all_ids: BTreeSet<&str> = BTreeSet::new();
    let mut man_dataset_ids: BTreeSet<&str> = BTreeSet::new();
    let mut duplicate_ids: BTreeSet<&str> = BTreeSet::new();
    let mut scorer_model_errors: Vec<String> = Vec::new();
    for entry in &manifest.datasets {
        if !all_ids.insert(entry.id.as_str()) {
            duplicate_ids.insert(entry.id.as_str());
        }
        match entry.kind {
            ManifestEntryKind::Dataset => {
                man_dataset_ids.insert(entry.id.as_str());
            }
            ManifestEntryKind::ScorerModel => {
                if reg_ids.contains(entry.id.as_str()) {
                    scorer_model_errors.push(format!(
                        "'{}' has kind=scorer_model but collides with registry dataset id",
                        entry.id
                    ));
                }
                if entry.core_applicability != CoreApplicability::NotMappable {
                    scorer_model_errors.push(format!(
                        "'{}' has kind=scorer_model and must set core_applicability='not_mappable'",
                        entry.id
                    ));
                }
                if entry.sources.is_empty() {
                    scorer_model_errors.push(format!(
                        "'{}' has kind=scorer_model but declares zero sources",
                        entry.id
                    ));
                }
            }
        }
    }

    let missing_in_manifest: Vec<&str> = reg_ids.difference(&man_dataset_ids).copied().collect();
    let extra_in_manifest: Vec<&str> = man_dataset_ids.difference(&reg_ids).copied().collect();

    let mut domain_mismatches = Vec::new();
    let mut dir_mismatches = Vec::new();
    for entry in &manifest.datasets {
        if entry.kind == ManifestEntryKind::Dataset
            && let Some(spec) = reg_map.get(entry.id.as_str())
        {
            if entry.domain != spec.domain {
                domain_mismatches.push(format!(
                    "{} (manifest={:?}, registry={:?})",
                    entry.id, entry.domain, spec.domain
                ));
            }
            if entry.relative_dir != spec.relative_dir {
                dir_mismatches.push(format!(
                    "{} (manifest='{}', registry='{}')",
                    entry.id, entry.relative_dir, spec.relative_dir
                ));
            }
        }
    }

    if duplicate_ids.is_empty()
        && missing_in_manifest.is_empty()
        && extra_in_manifest.is_empty()
        && domain_mismatches.is_empty()
        && dir_mismatches.is_empty()
        && scorer_model_errors.is_empty()
    {
        return Ok(());
    }

    let mut parts = Vec::new();
    if !missing_in_manifest.is_empty() {
        parts.push(format!(
            "missing from manifest: {}",
            missing_in_manifest.join(", ")
        ));
    }
    if !extra_in_manifest.is_empty() {
        parts.push(format!(
            "extra in manifest (not in registry): {}",
            extra_in_manifest.join(", ")
        ));
    }
    if !duplicate_ids.is_empty() {
        let dups: Vec<&str> = duplicate_ids.into_iter().collect();
        parts.push(format!("duplicate manifest ids: {}", dups.join(", ")));
    }
    if !domain_mismatches.is_empty() {
        parts.push(format!("domain mismatch: {}", domain_mismatches.join(", ")));
    }
    if !dir_mismatches.is_empty() {
        parts.push(format!(
            "relative_dir mismatch: {}",
            dir_mismatches.join(", ")
        ));
    }
    if !scorer_model_errors.is_empty() {
        parts.push(format!(
            "scorer_model validation: {}",
            scorer_model_errors.join(", ")
        ));
    }

    Err(LabError::Protocol(format!(
        "registry-manifest consistency check failed ({})",
        parts.join("; ")
    )))
}

/// Loads a manifest from `path` and checks consistency against `registry`.
pub fn load_and_validate_manifest(
    path: &Path,
    registry: &[BenchmarkDatasetSpec],
) -> Result<DatasetsManifest, LabError> {
    let manifest = load_manifest_from_path(path)?;
    check_registry_manifest_consistency(&manifest, registry)?;
    Ok(manifest)
}

/// Summary row produced by `monoagens-lab datasets list [--json]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetListEntry {
    pub id: String,
    pub kind: ManifestEntryKind,
    pub domain: BenchmarkDomain,
    pub lane: DatasetLane,
    pub sources: Vec<DatasetSource>,
    pub license_spdx: String,
    pub license_status: LicenseStatus,
    pub availability: DatasetAvailability,
    pub core_applicability: CoreApplicability,
    pub contamination_risk: ContaminationRisk,
    pub contamination_note: String,
    pub verification_state: String,
    pub has_pending_pin: bool,
    pub pending_pin_items: Vec<String>,
    pub missing_owner_checksums: bool,
    pub checksums_recorded: ChecksumsRecorded,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksums_recorded_date: Option<String>,
    pub reason: String,
}

/// Computes per-dataset listing entries from a validated manifest and `datasets_dir`.
pub fn describe_datasets_status(
    manifest: &DatasetsManifest,
    datasets_dir: &Path,
) -> Result<Vec<DatasetListEntry>, LabError> {
    ensure_not_sealed_path(datasets_dir)?;
    let mut rows = Vec::with_capacity(manifest.datasets.len());
    for entry in &manifest.datasets {
        let pending_items = entry.pending_pin_items();
        let has_pending_pin = !pending_items.is_empty();
        let missing_owner_checksums = entry.checksums_recorded == ChecksumsRecorded::Pending
            || entry.files.iter().any(|f| f.sha256.is_pending());

        let verification_state = if entry.availability == DatasetAvailability::Unavailable {
            "unavailable".to_owned()
        } else {
            let v = crate::datasets::ops::verify_manifest_entry(entry, datasets_dir)?;
            if v.verified {
                "verified".to_owned()
            } else if has_pending_pin && v.status == "missing" {
                "missing (pending_pin)".to_owned()
            } else if has_pending_pin {
                "unverified (pending_pin)".to_owned()
            } else {
                v.status
            }
        };

        rows.push(DatasetListEntry {
            id: entry.id.clone(),
            kind: entry.kind,
            domain: entry.domain,
            lane: entry.lane,
            sources: entry.sources.clone(),
            license_spdx: entry.license.spdx.clone(),
            license_status: entry.license.status,
            availability: entry.availability,
            core_applicability: entry.core_applicability,
            contamination_risk: entry.contamination_risk,
            contamination_note: entry.contamination_note.clone(),
            verification_state,
            has_pending_pin,
            pending_pin_items: pending_items,
            missing_owner_checksums,
            checksums_recorded: entry.checksums_recorded,
            checksums_recorded_date: entry.checksums_recorded_date.clone(),
            reason: entry.reason.clone(),
        });
    }
    Ok(rows)
}

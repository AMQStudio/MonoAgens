//! Owner-run machine pinning (`datasets pin`), atomic license-gated downloading (`datasets fetch`),
//! TOFU checksum recording (`--record-checksums`), and strict verification (`datasets verify`).
//!
//! Network access for external datasets is strictly isolated to [`pin_datasets`] and
//! [`fetch_datasets`]. No other Lab command or default test opens a network connection.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::benchmark::BenchmarkDatasetSpec;
use crate::datasets::manifest::{
    ChecksumsRecorded, DatasetAvailability, DatasetManifestEntry, DatasetSource, DatasetsManifest,
    LicenseStatus, ManifestEntryKind, ManifestFile, PENDING_PIN, PinnedSha256, PinnedSize,
    SourceKind, load_and_validate_manifest, save_manifest_to_path,
};
use crate::scenario::ensure_not_sealed_path;
use crate::{LabError, sha256_hex};

/// Extraction marker file written atomically into `lab/datasets/<relative_dir>/` after fetch.
pub const EXTRACTION_MARKER_FILE: &str = ".monoagens_extracted.json";

/// Converted cache metadata file checked against `entry.converter_version` during verification.
pub const CONVERTER_CACHE_FILE: &str = ".monoagens_converted_cache.json";

/// Optional API base overrides for `datasets pin` (used by loopback mock tests).
#[derive(Debug, Clone)]
pub struct PinEndpointOverrides {
    pub github_api_base: String,
    pub hf_api_base: String,
    pub bitbucket_api_base: String,
    pub timeout_secs: u64,
}

impl Default for PinEndpointOverrides {
    fn default() -> Self {
        Self {
            github_api_base: std::env::var("MONOAGENS_GITHUB_API_BASE")
                .unwrap_or_else(|_| "https://api.github.com".to_owned()),
            hf_api_base: std::env::var("MONOAGENS_HF_API_BASE")
                .unwrap_or_else(|_| "https://huggingface.co".to_owned()),
            bitbucket_api_base: std::env::var("MONOAGENS_BITBUCKET_API_BASE")
                .unwrap_or_else(|_| "https://api.bitbucket.org".to_owned()),
            timeout_secs: 30,
        }
    }
}

/// Optional download base overrides for `datasets fetch` (used by loopback fixture tests).
#[derive(Debug, Clone)]
pub struct FetchEndpointOverrides {
    pub github_raw_base: String,
    pub hf_raw_base: String,
    pub fetch_date: Option<String>,
    pub timeout_secs: u64,
}

impl Default for FetchEndpointOverrides {
    fn default() -> Self {
        Self {
            github_raw_base: std::env::var("MONOAGENS_GITHUB_RAW_BASE")
                .unwrap_or_else(|_| "https://raw.githubusercontent.com".to_owned()),
            hf_raw_base: std::env::var("MONOAGENS_HF_RAW_BASE")
                .unwrap_or_else(|_| "https://huggingface.co".to_owned()),
            fetch_date: std::env::var("MONOAGENS_FETCH_DATE").ok(),
            timeout_secs: 120,
        }
    }
}

/// Extraction marker stored inside a fetched dataset directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionMarker {
    pub dataset_id: String,
    pub revisions: BTreeMap<String, String>,
    pub files_count: usize,
    pub extracted_ok: bool,
}

/// Converted cache metadata verified against `entry.converter_version`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvertedCacheMetadata {
    pub dataset_id: String,
    pub converter_version: String,
    pub revisions: BTreeMap<String, String>,
}

/// Result of pinning one dataset in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinDatasetResult {
    pub id: String,
    pub updated_sources: usize,
    pub updated_files: usize,
    pub license_status: LicenseStatus,
}

/// Result of fetching one dataset into `lab/datasets/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchDatasetResult {
    pub id: String,
    pub target_dir: String,
    pub files_fetched: usize,
    pub recorded_owner_checksums: bool,
}

/// Verification outcome for a single dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetVerifyResult {
    pub id: String,
    pub status: String,
    pub verified: bool,
    pub failures: Vec<String>,
}

/// Aggregate verification report across requested datasets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyDatasetsReport {
    pub all_verified: bool,
    pub results: Vec<DatasetVerifyResult>,
}

#[derive(Deserialize)]
struct GhCommitResp {
    sha: String,
}

#[derive(Deserialize)]
struct GhTreeResp {
    tree: Vec<GhTreeEntry>,
}

#[derive(Deserialize)]
struct GhTreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    size: Option<u64>,
}

#[derive(Deserialize)]
struct HfDatasetMeta {
    sha: String,
}

#[derive(Deserialize)]
struct HfTreeEntry {
    path: String,
    #[serde(rename = "type")]
    kind: String,
    size: Option<u64>,
    lfs: Option<HfLfsInfo>,
}

#[derive(Deserialize)]
struct HfLfsInfo {
    oid: Option<String>,
    size: Option<u64>,
}

/// Queries GitHub/HF APIs read-only and updates revisions, sizes, and available LFS SHA-256s
/// in `manifest_path` without downloading corpus files or altering `license.status`.
pub fn pin_datasets(
    manifest_path: &Path,
    registry: &[BenchmarkDatasetSpec],
    ids: &[String],
    overrides: &PinEndpointOverrides,
) -> Result<Vec<PinDatasetResult>, LabError> {
    let mut manifest = load_and_validate_manifest(manifest_path, registry)?;
    let target_ids = resolve_target_ids(&manifest, ids, false)?;

    let client = Client::builder()
        .timeout(Duration::from_secs(overrides.timeout_secs.max(1)))
        .user_agent("monoagens-lab/0.1")
        .build()
        .map_err(|e| LabError::Protocol(format!("failed to build HTTP client: {e}")))?;

    let mut results = Vec::new();
    for id in target_ids {
        let entry = manifest
            .get_mut(&id)
            .ok_or_else(|| LabError::Protocol(format!("unknown dataset id '{id}'")))?;
        if entry.availability == DatasetAvailability::Unavailable {
            return Err(LabError::Protocol(format!(
                "cannot pin unavailable dataset '{}'",
                entry.id
            )));
        }

        let mut updated_sources = 0usize;
        let mut updated_files = 0usize;
        let mut file_meta_by_role: BTreeMap<String, BTreeMap<String, (u64, Option<String>)>> =
            BTreeMap::new();

        for (src_idx, src) in entry.sources.iter_mut().enumerate() {
            let role_key = src
                .role
                .clone()
                .unwrap_or_else(|| format!("__idx_{src_idx}"));
            match src.kind {
                SourceKind::Git => {
                    if let Some((owner, repo)) = parse_github_owner_repo(&src.location) {
                        let (sha, files) =
                            pin_github_repo(&client, &overrides.github_api_base, &owner, &repo)?;
                        src.revision = sha;
                        updated_sources += 1;
                        file_meta_by_role.insert(role_key, files);
                    } else if let Some((owner, repo)) = parse_bitbucket_owner_repo(&src.location) {
                        let sha = pin_bitbucket_repo(
                            &client,
                            &overrides.bitbucket_api_base,
                            &owner,
                            &repo,
                        )?;
                        src.revision = sha;
                        updated_sources += 1;
                    }
                }
                SourceKind::Huggingface => {
                    let repo_id = parse_hf_repo_id(&src.location);
                    let is_model = entry.kind == ManifestEntryKind::ScorerModel;
                    let (sha, files) =
                        pin_hf_repo(&client, &overrides.hf_api_base, &repo_id, is_model)?;
                    src.revision = sha;
                    updated_sources += 1;
                    file_meta_by_role.insert(role_key, files);
                }
                SourceKind::Url | SourceKind::PaperOnly => {}
            }
        }

        let default_role_map = entry
            .sources
            .first()
            .and_then(|s| s.role.clone())
            .unwrap_or_else(|| "__idx_0".to_owned());

        for file in &mut entry.files {
            let role_key = file
                .source_role
                .as_deref()
                .unwrap_or(default_role_map.as_str());
            if let Some(tree_map) = file_meta_by_role.get(role_key)
                && let Some((size, lfs_sha)) = tree_map.get(&file.path)
            {
                file.size = PinnedSize::Bytes(*size);
                if let Some(hex) = lfs_sha
                    && is_valid_sha256_hex(hex)
                {
                    file.sha256 = PinnedSha256::Sha256(hex.to_ascii_lowercase());
                }
                updated_files += 1;
            }
        }

        if entry.kind == ManifestEntryKind::ScorerModel {
            let mut seen_paths: BTreeSet<String> =
                entry.files.iter().map(|f| f.path.clone()).collect();
            let role_opt = entry.sources.first().and_then(|s| s.role.clone());
            if let Some(tree_map) = file_meta_by_role.get(&default_role_map) {
                for (path, (size, lfs_sha)) in tree_map {
                    if path.starts_with('.') || seen_paths.contains(path) {
                        continue;
                    }
                    seen_paths.insert(path.clone());
                    let sha256 = lfs_sha
                        .as_deref()
                        .filter(|hex| is_valid_sha256_hex(hex))
                        .map_or(PinnedSha256::PendingPin, |hex| {
                            PinnedSha256::Sha256(hex.to_ascii_lowercase())
                        });
                    entry.files.push(ManifestFile {
                        path: path.clone(),
                        source_role: role_opt.clone(),
                        sha256,
                        size: PinnedSize::Bytes(*size),
                        download_url: None,
                        extract: None,
                    });
                    updated_files += 1;
                }
            }
        }

        if !entry.files.is_empty()
            && entry.files.iter().all(|f| !f.sha256.is_pending())
            && entry.checksums_recorded == ChecksumsRecorded::Pending
        {
            entry.checksums_recorded = ChecksumsRecorded::Source;
            entry.checksums_recorded_date = Some(utc_date_today(None));
        }

        results.push(PinDatasetResult {
            id: entry.id.clone(),
            updated_sources,
            updated_files,
            license_status: entry.license.status,
        });
    }

    save_manifest_to_path(&manifest, manifest_path)?;
    Ok(results)
}

/// Fetches cleared, pinned datasets atomically via `.partial/`, enforcing license clearance,
/// size + SHA-256 verification (or TOFU `--record-checksums`), and extraction markers.
pub fn fetch_datasets(
    manifest_path: &Path,
    datasets_dir: &Path,
    registry: &[BenchmarkDatasetSpec],
    ids: &[String],
    record_checksums: bool,
    overrides: &FetchEndpointOverrides,
) -> Result<Vec<FetchDatasetResult>, LabError> {
    ensure_not_sealed_path(datasets_dir)?;
    let mut manifest = load_and_validate_manifest(manifest_path, registry)?;
    let target_ids = resolve_target_ids(&manifest, ids, true)?;

    let client = Client::builder()
        .timeout(Duration::from_secs(overrides.timeout_secs.max(1)))
        .user_agent("monoagens-lab/0.1")
        .build()
        .map_err(|e| LabError::Protocol(format!("failed to build HTTP client: {e}")))?;

    let mut results = Vec::new();
    let mut manifest_dirty = false;

    for id in target_ids {
        let entry = manifest
            .get_mut(&id)
            .ok_or_else(|| LabError::Protocol(format!("unknown dataset id '{id}'")))?;

        if entry.availability == DatasetAvailability::Unavailable {
            return Err(LabError::Protocol(format!(
                "dataset '{}' has availability 'unavailable' and cannot be fetched",
                entry.id
            )));
        }

        if entry.license.status != LicenseStatus::Cleared {
            return Err(LabError::Protocol(format!(
                "refusing to fetch dataset '{}': license.status is '{}' (must be 'cleared')",
                entry.id, entry.license.status
            )));
        }

        if record_checksums {
            let already_has_checksums = entry.checksums_recorded != ChecksumsRecorded::Pending
                || (!entry.files.is_empty()
                    && entry
                        .files
                        .iter()
                        .all(|f| !f.sha256.is_pending() && !f.size.is_pending()));
            if already_has_checksums {
                return Err(LabError::Protocol(format!(
                    "refusing --record-checksums for dataset '{}': manifest already holds checksums (use `monoagens-lab datasets verify {}` instead)",
                    entry.id, entry.id
                )));
            }
        } else if entry
            .files
            .iter()
            .any(|f| f.sha256.is_pending() || f.size.is_pending())
        {
            return Err(LabError::Protocol(format!(
                "refusing to fetch dataset '{}': files still have 'pending_pin' checksums/sizes (run `datasets pin {}` or `datasets fetch --record-checksums {}` if upstream publishes none)",
                entry.id, entry.id, entry.id
            )));
        }

        if entry.files.is_empty() {
            return Err(LabError::Protocol(format!(
                "dataset '{}' declares zero files in manifest",
                entry.id
            )));
        }

        let target_dir = datasets_dir.join(&entry.relative_dir);
        let dir_existed = target_dir.exists();
        let partial_dir = target_dir.join(".partial");
        if partial_dir.exists() {
            fs::remove_dir_all(&partial_dir)?;
        }
        fs::create_dir_all(&partial_dir)?;

        let fetch_outcome = (|| -> Result<Vec<(String, String, u64)>, LabError> {
            let mut staged_records = Vec::with_capacity(entry.files.len());
            for file in &entry.files {
                let url = resolve_file_download_url(entry, file, overrides)?;
                let resp = client.get(&url).send().map_err(|e| {
                    LabError::Io(std::io::Error::other(format!(
                        "download failed for '{}' ({}): {e}",
                        entry.id, file.path
                    )))
                })?;
                if !resp.status().is_success() {
                    return Err(LabError::Io(std::io::Error::other(format!(
                        "HTTP {} downloading '{}' ({}) from {}",
                        resp.status(),
                        entry.id,
                        file.path,
                        url
                    ))));
                }
                let bytes = resp.bytes().map_err(|e| {
                    LabError::Io(std::io::Error::other(format!(
                        "interrupted download reading '{}' ({}): {e}",
                        entry.id, file.path
                    )))
                })?;

                let actual_size = bytes.len() as u64;
                let actual_sha = sha256_hex(&bytes);

                if let Some(expected_size) = file.size.as_bytes()
                    && actual_size != expected_size
                {
                    return Err(LabError::Protocol(format!(
                        "size mismatch for '{}' file '{}': expected {} bytes, got {} bytes",
                        entry.id, file.path, expected_size, actual_size
                    )));
                }
                if let Some(expected_sha) = file.sha256.as_hex()
                    && !actual_sha.eq_ignore_ascii_case(expected_sha)
                {
                    return Err(LabError::Protocol(format!(
                        "checksum mismatch for '{}' file '{}': expected sha256 {}, got {}",
                        entry.id, file.path, expected_sha, actual_sha
                    )));
                }

                let partial_file_path = safe_join_relative(&partial_dir, &file.path)?;
                if let Some(parent) = partial_file_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&partial_file_path, &bytes)?;
                staged_records.push((file.path.clone(), actual_sha, actual_size));
            }
            Ok(staged_records)
        })();

        let staged_records = match fetch_outcome {
            Ok(recs) => recs,
            Err(err) => {
                let _ = fs::remove_dir_all(&partial_dir);
                if !dir_existed {
                    let _ = fs::remove_dir_all(&target_dir);
                }
                return Err(err);
            }
        };

        // Atomically promote staged files out of `.partial/`.
        for (rel_path, _, _) in &staged_records {
            let src = safe_join_relative(&partial_dir, rel_path)?;
            let dst = safe_join_relative(&target_dir, rel_path)?;
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&src, &dst)?;
        }

        let marker = ExtractionMarker {
            dataset_id: entry.id.clone(),
            revisions: source_revisions_map(&entry.sources),
            files_count: staged_records.len(),
            extracted_ok: true,
        };
        let marker_partial = partial_dir.join(EXTRACTION_MARKER_FILE);
        fs::write(
            &marker_partial,
            serde_json::to_vec_pretty(&marker)
                .map_err(|e| LabError::Serialization(e.to_string()))?,
        )?;
        fs::rename(&marker_partial, target_dir.join(EXTRACTION_MARKER_FILE))?;
        let _ = fs::remove_dir_all(&partial_dir);

        if record_checksums {
            for (file, (_, sha, size)) in entry.files.iter_mut().zip(staged_records.iter()) {
                file.sha256 = PinnedSha256::Sha256(sha.clone());
                file.size = PinnedSize::Bytes(*size);
            }
            entry.checksums_recorded = ChecksumsRecorded::Owner;
            entry.checksums_recorded_date = Some(utc_date_today(overrides.fetch_date.as_deref()));
            manifest_dirty = true;
        }

        results.push(FetchDatasetResult {
            id: entry.id.clone(),
            target_dir: target_dir.to_string_lossy().into_owned(),
            files_fetched: staged_records.len(),
            recorded_owner_checksums: record_checksums,
        });
    }

    if manifest_dirty {
        save_manifest_to_path(&manifest, manifest_path)?;
    }

    Ok(results)
}

/// Verifies every requested dataset's file SHA-256s, byte sizes, extraction marker, and
/// converted cache version (`converter_version`). Never opens any network connection.
pub fn verify_datasets(
    manifest_path: &Path,
    datasets_dir: &Path,
    registry: &[BenchmarkDatasetSpec],
    ids: &[String],
) -> Result<VerifyDatasetsReport, LabError> {
    ensure_not_sealed_path(datasets_dir)?;
    let manifest = load_and_validate_manifest(manifest_path, registry)?;
    let target_ids = resolve_target_ids(&manifest, ids, false)?;

    let mut results = Vec::with_capacity(target_ids.len());
    let mut all_verified = true;

    for id in target_ids {
        let entry = manifest
            .get(&id)
            .ok_or_else(|| LabError::Protocol(format!("unknown dataset id '{id}'")))?;
        let res = verify_manifest_entry(entry, datasets_dir)?;
        if entry.availability != DatasetAvailability::Unavailable && !res.verified {
            all_verified = false;
        }
        results.push(res);
    }

    Ok(VerifyDatasetsReport {
        all_verified,
        results,
    })
}

/// Verifies a single [`DatasetManifestEntry`] against `datasets_dir` (offline, pure filesystem).
pub fn verify_manifest_entry(
    entry: &DatasetManifestEntry,
    datasets_dir: &Path,
) -> Result<DatasetVerifyResult, LabError> {
    ensure_not_sealed_path(datasets_dir)?;
    if entry.availability == DatasetAvailability::Unavailable {
        return Ok(DatasetVerifyResult {
            id: entry.id.clone(),
            status: "unavailable".to_owned(),
            verified: false,
            failures: vec![format!(
                "dataset '{}' is unavailable: {}",
                entry.id, entry.reason
            )],
        });
    }

    let mut failures = Vec::new();
    let pending = entry.pending_pin_items();
    if !pending.is_empty() {
        failures.push(format!(
            "manifest entry '{}' has pending_pin items: {}",
            entry.id,
            pending.join(", ")
        ));
    }

    let dataset_dir = datasets_dir.join(&entry.relative_dir);
    if !dataset_dir.is_dir() {
        failures.push(format!(
            "dataset directory missing: '{}' (run `monoagens-lab datasets fetch {}`)",
            dataset_dir.display(),
            entry.id
        ));
    } else {
        if entry.files.is_empty() {
            failures.push(format!(
                "manifest entry '{}' lists zero files to verify",
                entry.id
            ));
        }
        for file in &entry.files {
            let file_path = match safe_join_relative(&dataset_dir, &file.path) {
                Ok(p) => p,
                Err(e) => {
                    failures.push(e.to_string());
                    continue;
                }
            };
            if !file_path.is_file() {
                failures.push(format!(
                    "missing dataset file '{}' for '{}'",
                    file.path, entry.id
                ));
                continue;
            }
            let bytes = fs::read(&file_path)?;
            let actual_size = bytes.len() as u64;
            let actual_sha = sha256_hex(&bytes);
            if let Some(expected_size) = file.size.as_bytes()
                && actual_size != expected_size
            {
                failures.push(format!(
                    "size mismatch in '{}' ({}): expected {} bytes, actual {} bytes",
                    file.path, entry.id, expected_size, actual_size
                ));
            }
            if let Some(expected_sha) = file.sha256.as_hex()
                && !actual_sha.eq_ignore_ascii_case(expected_sha)
            {
                failures.push(format!(
                    "sha256 mismatch in '{}' ({}): expected {}, actual {}",
                    file.path, entry.id, expected_sha, actual_sha
                ));
            }
        }

        // Verify extraction marker.
        let marker_path = dataset_dir.join(EXTRACTION_MARKER_FILE);
        if !marker_path.is_file() {
            failures.push(format!(
                "missing extraction marker '{}' for '{}'",
                EXTRACTION_MARKER_FILE, entry.id
            ));
        } else {
            let raw = fs::read_to_string(&marker_path)?;
            match serde_json::from_str::<ExtractionMarker>(&raw) {
                Ok(marker) => {
                    if marker.dataset_id != entry.id || !marker.extracted_ok {
                        failures.push(format!(
                            "invalid extraction marker for '{}' (id={}, extracted_ok={})",
                            entry.id, marker.dataset_id, marker.extracted_ok
                        ));
                    } else if marker.revisions != source_revisions_map(&entry.sources) {
                        failures.push(format!(
                            "extraction marker revision mismatch for '{}': cached {:?} vs manifest {:?}",
                            entry.id,
                            marker.revisions,
                            source_revisions_map(&entry.sources)
                        ));
                    }
                }
                Err(e) => {
                    failures.push(format!(
                        "corrupt extraction marker '{}' for '{}': {e}",
                        EXTRACTION_MARKER_FILE, entry.id
                    ));
                }
            }
        }

        // Verify converted cache if present.
        let cache_path = dataset_dir.join(CONVERTER_CACHE_FILE);
        if cache_path.is_file() {
            let raw = fs::read_to_string(&cache_path)?;
            match serde_json::from_str::<ConvertedCacheMetadata>(&raw) {
                Ok(cache) => {
                    if cache.converter_version != entry.converter_version {
                        failures.push(format!(
                            "stale converted cache for '{}': cached converter_version '{}' != manifest '{}'",
                            entry.id, cache.converter_version, entry.converter_version
                        ));
                    }
                    if cache.revisions != source_revisions_map(&entry.sources) {
                        failures.push(format!(
                            "stale converted cache for '{}': revision mismatch",
                            entry.id
                        ));
                    }
                }
                Err(e) => {
                    failures.push(format!(
                        "corrupt converted cache metadata '{}' for '{}': {e}",
                        CONVERTER_CACHE_FILE, entry.id
                    ));
                }
            }
        }
    }

    let verified = failures.is_empty();
    let status = if verified {
        "verified".to_owned()
    } else if !dataset_dir.is_dir() {
        "missing".to_owned()
    } else {
        "unverified".to_owned()
    };

    Ok(DatasetVerifyResult {
        id: entry.id.clone(),
        status,
        verified,
        failures,
    })
}

fn resolve_target_ids(
    manifest: &DatasetsManifest,
    ids: &[String],
    skip_unavailable_when_all: bool,
) -> Result<Vec<String>, LabError> {
    if ids.is_empty() {
        return Ok(manifest
            .datasets
            .iter()
            .filter(|d| {
                !skip_unavailable_when_all || d.availability != DatasetAvailability::Unavailable
            })
            .map(|d| d.id.clone())
            .collect());
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if manifest.get(id).is_none() {
            return Err(LabError::Protocol(format!(
                "dataset id '{id}' is not registered in manifest"
            )));
        }
        out.push(id.clone());
    }
    Ok(out)
}

fn resolve_file_download_url(
    entry: &DatasetManifestEntry,
    file: &crate::datasets::manifest::ManifestFile,
    overrides: &FetchEndpointOverrides,
) -> Result<String, LabError> {
    if let Some(explicit) = &file.download_url {
        return Ok(explicit.clone());
    }
    let source = if let Some(role) = &file.source_role {
        entry
            .sources
            .iter()
            .find(|s| s.role.as_deref() == Some(role.as_str()))
            .ok_or_else(|| {
                LabError::Protocol(format!(
                    "dataset '{}' file '{}' references missing source_role '{role}'",
                    entry.id, file.path
                ))
            })?
    } else {
        entry.sources.first().ok_or_else(|| {
            LabError::Protocol(format!(
                "dataset '{}' has no sources for file '{}'",
                entry.id, file.path
            ))
        })?
    };

    if source.kind != SourceKind::Url && source.revision == PENDING_PIN {
        return Err(LabError::Protocol(format!(
            "refusing to fetch '{}' ({}): source '{}' revision is still 'pending_pin'",
            entry.id, file.path, source.location
        )));
    }

    let rel = file.path.trim_start_matches('/');
    match source.kind {
        SourceKind::Huggingface => {
            let repo_id = parse_hf_repo_id(&source.location);
            let base = overrides.hf_raw_base.trim_end_matches('/');
            if entry.kind == ManifestEntryKind::ScorerModel {
                Ok(format!(
                    "{base}/{repo_id}/resolve/{}/{}",
                    source.revision, rel
                ))
            } else {
                Ok(format!(
                    "{base}/datasets/{repo_id}/resolve/{}/{}",
                    source.revision, rel
                ))
            }
        }
        SourceKind::Git => {
            if let Some((owner, repo)) = parse_github_owner_repo(&source.location) {
                Ok(format!(
                    "{}/{}/{}/{}/{}",
                    overrides.github_raw_base.trim_end_matches('/'),
                    owner,
                    repo,
                    source.revision,
                    rel
                ))
            } else {
                Ok(format!(
                    "{}/{}/{}",
                    source.location.trim_end_matches('/'),
                    source.revision,
                    rel
                ))
            }
        }
        SourceKind::Url => {
            if source.location.ends_with(rel) {
                Ok(source.location.clone())
            } else {
                Ok(format!("{}/{}", source.location.trim_end_matches('/'), rel))
            }
        }
        SourceKind::PaperOnly => Err(LabError::Protocol(format!(
            "cannot download paper_only source '{}' for '{}'",
            source.location, entry.id
        ))),
    }
}

type PinnedAssetTree = (String, BTreeMap<String, (u64, Option<String>)>);

fn pin_github_repo(
    client: &Client,
    api_base: &str,
    owner: &str,
    repo: &str,
) -> Result<PinnedAssetTree, LabError> {
    let base = api_base.trim_end_matches('/');
    let commit_url = format!("{base}/repos/{owner}/{repo}/commits/HEAD");
    let commit: GhCommitResp = client
        .get(&commit_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            LabError::Io(std::io::Error::other(format!(
                "GitHub pin '{commit_url}': {e}"
            )))
        })?
        .json()
        .map_err(|e| LabError::Serialization(format!("GitHub commit JSON '{commit_url}': {e}")))?;

    let tree_url = format!(
        "{base}/repos/{owner}/{repo}/git/trees/{}?recursive=1",
        commit.sha
    );
    let tree: GhTreeResp = client
        .get(&tree_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| {
            LabError::Io(std::io::Error::other(format!(
                "GitHub tree '{tree_url}': {e}"
            )))
        })?
        .json()
        .map_err(|e| LabError::Serialization(format!("GitHub tree JSON '{tree_url}': {e}")))?;

    let mut files = BTreeMap::new();
    for entry in tree.tree {
        if entry.kind == "blob"
            && let Some(sz) = entry.size
        {
            files.insert(entry.path, (sz, None));
        }
    }
    Ok((commit.sha, files))
}

fn pin_bitbucket_repo(
    client: &Client,
    api_base: &str,
    owner: &str,
    repo: &str,
) -> Result<String, LabError> {
    #[derive(Deserialize)]
    struct BbCommits {
        values: Vec<BbCommit>,
    }
    #[derive(Deserialize)]
    struct BbCommit {
        hash: String,
    }
    let url = format!(
        "{}/2.0/repositories/{owner}/{repo}/commits?pagelen=1",
        api_base.trim_end_matches('/')
    );
    let resp: BbCommits = client
        .get(&url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| LabError::Io(std::io::Error::other(format!("Bitbucket pin '{url}': {e}"))))?
        .json()
        .map_err(|e| LabError::Serialization(format!("Bitbucket JSON '{url}': {e}")))?;
    resp.values
        .into_iter()
        .next()
        .map(|c| c.hash)
        .ok_or_else(|| LabError::Protocol(format!("no commits returned from '{url}'")))
}

fn pin_hf_repo(
    client: &Client,
    api_base: &str,
    repo_id: &str,
    is_model: bool,
) -> Result<PinnedAssetTree, LabError> {
    let base = api_base.trim_end_matches('/');
    let api_kind = if is_model { "models" } else { "datasets" };
    let meta_url = format!("{base}/api/{api_kind}/{repo_id}");
    let meta: HfDatasetMeta = client
        .get(&meta_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| LabError::Io(std::io::Error::other(format!("HF pin '{meta_url}': {e}"))))?
        .json()
        .map_err(|e| LabError::Serialization(format!("HF repo JSON '{meta_url}': {e}")))?;

    let tree_url = format!(
        "{base}/api/{api_kind}/{repo_id}/tree/{}?recursive=1",
        meta.sha
    );
    let tree: Vec<HfTreeEntry> = client
        .get(&tree_url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| LabError::Io(std::io::Error::other(format!("HF tree '{tree_url}': {e}"))))?
        .json()
        .map_err(|e| LabError::Serialization(format!("HF tree JSON '{tree_url}': {e}")))?;

    let mut files = BTreeMap::new();
    for entry in tree {
        if entry.kind == "file" {
            let size = entry.lfs.as_ref().and_then(|l| l.size).or(entry.size);
            let oid = entry.lfs.and_then(|l| l.oid);
            if let Some(sz) = size {
                files.insert(entry.path, (sz, oid));
            }
        }
    }
    Ok((meta.sha, files))
}

fn parse_github_owner_repo(location: &str) -> Option<(String, String)> {
    let trimmed = location.trim_end_matches('/').trim_end_matches(".git");
    let rest = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))?;
    let mut parts = rest.split('/');
    let owner = parts.next()?.to_owned();
    let repo = parts.next()?.to_owned();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

fn parse_bitbucket_owner_repo(location: &str) -> Option<(String, String)> {
    let trimmed = location.trim_end_matches('/').trim_end_matches(".git");
    let rest = trimmed
        .strip_prefix("https://bitbucket.org/")
        .or_else(|| trimmed.strip_prefix("http://bitbucket.org/"))?;
    let mut parts = rest.split('/');
    let owner = parts.next()?.to_owned();
    let repo = parts.next()?.to_owned();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

fn parse_hf_repo_id(location: &str) -> String {
    let trimmed = location.trim_end_matches('/');
    for prefix in [
        "https://huggingface.co/datasets/",
        "http://huggingface.co/datasets/",
        "https://huggingface.co/models/",
        "http://huggingface.co/models/",
        "https://huggingface.co/",
        "http://huggingface.co/",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.to_owned();
        }
    }
    trimmed.to_owned()
}

fn source_revisions_map(sources: &[DatasetSource]) -> BTreeMap<String, String> {
    sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let key = s.role.clone().unwrap_or_else(|| format!("source_{i}"));
            (key, s.revision.clone())
        })
        .collect()
}

fn safe_join_relative(base: &Path, rel: &str) -> Result<PathBuf, LabError> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(LabError::Protocol(format!(
            "manifest file path '{rel}' must be relative"
        )));
    }
    for comp in rel_path.components() {
        match comp {
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(LabError::Protocol(format!(
                    "manifest file path '{rel}' contains disallowed traversal component"
                )));
            }
            std::path::Component::CurDir | std::path::Component::Normal(_) => {}
        }
    }
    Ok(base.join(rel_path))
}

fn is_valid_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn utc_date_today(override_date: Option<&str>) -> String {
    if let Some(d) = override_date {
        return d.to_owned();
    }
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86_400) as i64;
    // Howard Hinnant civil-from-days algorithm (UTC YYYY-MM-DD).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    format!("{year:04}-{m:02}-{d:02}")
}

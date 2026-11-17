//! External benchmark dataset manifest, machine pinning, fetch/verify lifecycle,
//! behavioral `Converter`/`Scorer` framework, `reason::Embedder` probe lane, and per-dataset
//! adapters/scorers.

pub mod framework;
pub mod manifest;
pub mod ops;
pub mod parquet_utils;
pub mod report;
pub mod runner;
pub mod scoring_utils;

// Behavioral datasets (Tasks 7.1–7.4)
pub mod capa;
pub mod dyntom;
pub mod exploretom;
pub mod fantom;
pub mod group_mem_bench;
pub mod hi_tom;
pub mod locomo;
pub mod longmemeval;
pub mod memory_agent_bench;
pub mod molweni;
pub mod opentom;
pub mod pitch;
pub mod pm_bench;
pub mod proactive_bench;
pub mod stale;
pub mod tomi;
pub mod ubuntu_irc;
pub mod when2speak;

// Component `reason::EmbedderProbe` datasets (Task 7.5)
pub mod indocollex;
pub mod multilexnorm;

pub use capa::{CapaConverter, CapaScorer};
pub use dyntom::{DynTomConverter, DynTomScorer};
pub use exploretom::{ExploreTomConverter, ExploreTomScorer};
pub use fantom::{FantomConverter, FantomScorer};
pub use framework::{
    CachingEmbedder, ClusterBootstrapCi, ConvertedDataset, Converter, EmbedderProbe,
    EmbedderProbeOutcome, EmbedderReadiness, ExternalItem, ExternalItemExecution,
    ExternalPrediction, ExternalProbe, GoldBook, GoldEntry, GoldEvidenceRef, HistoryMode,
    JudgeExecutionConfig, SILENT_THINK_JSON, SamplingResult, SamplingRule, Scorer,
    ScorerDetailRecorder, ScorerItemOutcome, ScorerMetric, ScorerReport, SplitMix64,
    TranscriptRecord, assert_visiblescope_gold_evidence, cluster_bootstrap_ci,
    execute_external_item, execute_external_item_with_embedder, load_or_convert_cached,
    make_text_percept, sample_items_stratified, validate_semantic_embedder,
};
pub use group_mem_bench::{GroupMemBenchConverter, GroupMemBenchScorer};
pub use hi_tom::{HiTomConverter, HiTomScorer};
pub use indocollex::IndoCollexProbe;
pub use locomo::{LocomoConverter, LocomoScorer};
pub use longmemeval::{LongMemEvalConverter, LongMemEvalScorer};
pub use manifest::{
    ArchiveKind, ChecksumsRecorded, ContaminationRisk, CoreApplicability,
    DEFAULT_LOCAL_MANIFEST_PATH, DEFAULT_TEMPLATE_MANIFEST_PATH, DatasetAvailability, DatasetLane,
    DatasetLicense, DatasetListEntry, DatasetManifestEntry, DatasetSource, DatasetsManifest,
    LicenseStatus, ManifestEntryKind, ManifestFile, PinnedSha256, PinnedSize, SourceKind,
    check_registry_manifest_consistency, describe_datasets_status,
    init_local_manifest_from_template, load_and_validate_manifest, load_manifest_from_path,
    parse_manifest_strict, resolve_template_manifest_path, save_manifest_to_path,
};
pub use memory_agent_bench::{MemoryAgentBenchConverter, MemoryAgentBenchScorer};
pub use molweni::{MolweniConverter, MolweniScorer};
pub use multilexnorm::MultiLexNormProbe;
pub use opentom::{OpenTomConverter, OpenTomScorer};
pub use ops::{
    CONVERTER_CACHE_FILE, ConvertedCacheMetadata, DatasetVerifyResult, EXTRACTION_MARKER_FILE,
    ExtractionMarker, FetchDatasetResult, FetchEndpointOverrides, PinDatasetResult,
    PinEndpointOverrides, VerifyDatasetsReport, fetch_datasets, pin_datasets, verify_datasets,
    verify_manifest_entry,
};
pub use pitch::{PitchConverter, PitchScorer};
pub use pm_bench::{PmBenchConverter, PmBenchScorer};
pub use proactive_bench::{ProactiveBenchConverter, ProactiveBenchScorer};
pub use report::{GeneratedReportSummary, ReportOptions, generate_benchmark_report};
pub use runner::{
    BenchmarkExecutionOptions, DatasetHandlerRegistry, JudgeConfig, JudgeSubstitutionPolicy,
    PreflightDatasetEstimate, PreflightEstimate, estimate_live_preflight,
    execute_external_datasets, load_lab_provider_secrets, official_judge_model,
    resolve_judge_metric_name, resolve_manifest_path, verify_required_datasets_with_manifest,
};
pub use scoring_utils::{default_sampling_rule, run_optional_python_helper};
pub use stale::{StaleConverter, StaleScorer};
pub use tomi::{TomiConverter, TomiScorer};
pub use ubuntu_irc::{UbuntuIrcConverter, UbuntuIrcScorer};
pub use when2speak::{When2SpeakConverter, When2SpeakScorer};

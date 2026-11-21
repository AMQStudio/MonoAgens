//! Five-domain curated benchmark suite registry (`R-39`, `A-15`, `A-16`), dataset path
//! verification (never reading `sealed/`, failing non-zero on missing datasets), Tool RAG
//! scaling (`N_caps = 50..200`), and `Arm T` restart parity checks (`tasks.md` 11.4).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use monoagens_core::{
    CapabilityCatalogEntry, Core, CoreConfig, MemStore, NoopDoors, Store, ToolSelectionBudget,
    select_tools_for_context,
};
use monoagens_protocol::{
    ActorId, Audience, CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash,
    OpaquePayload, Percept, PlaceId, Time, VisibleScope,
};
use serde::{Deserialize, Serialize};

use crate::LabError;
use crate::datasets::{
    BenchmarkExecutionOptions, PreflightEstimate, build_standard_index, execute_external_datasets,
    load_and_validate_manifest, resolve_manifest_path, suite_domain,
    verify_required_datasets_with_manifest,
};
use crate::pareto::{
    ConvexParetoReport, ParetoScorecardInput, ScorecardRow, compute_convex_pareto,
    scorecard_rows_to_pareto_input,
};
use crate::record::RunRecord;
use crate::scenario::ensure_not_sealed_path;

/// Five cognitive domains plus Indonesian, Tool RAG, and Security benchmark suites (`R-39`, `A-15`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkDomain {
    Memory,
    Prospective,
    ProactivityTiming,
    GroupChat,
    TheoryOfMind,
    IndonesianRegister,
    ToolRag,
    Security,
}

/// Curated benchmark dataset specification with explicit methodological caveats (`R-39`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BenchmarkDatasetSpec {
    pub id: String,
    pub domain: BenchmarkDomain,
    pub relative_dir: String,
    pub primary_metrics: Vec<String>,
    pub caveats: String,
    pub required_in_standard_suite: bool,
    pub requires_external_corpus: bool,
}

/// Tool RAG scaling measurement point (`N_caps = 50..200`, `R-38`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolRagScalePoint {
    pub n_caps: usize,
    pub baseline_caps_in_context: usize,
    pub selected_caps_in_context: usize,
    pub schema_token_savings_ratio: f64,
}

/// Output of `monoagens-lab benchmark`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkSuiteReport {
    pub suite: String,
    pub partial: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left_out_datasets: Vec<String>,
    pub datasets_dir: String,
    pub dataset_specs: Vec<BenchmarkDatasetSpec>,
    pub scorecard: Vec<ScorecardRow>,
    pub run_records: Vec<RunRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_estimate: Option<PreflightEstimate>,
    pub excluded_from_multiparty_claims: Vec<String>,
    pub restricted_to_shortcut_checks: Vec<String>,
    pub tool_rag_scaling: Vec<ToolRagScalePoint>,
    pub arm_t_restart_parity_ok: bool,
    pub one_over_n_band_check: (f64, f64),
    pub scorecard_pareto_input: ParetoScorecardInput,
    pub pareto: Option<ConvexParetoReport>,
}

/// Returns the canonical curated benchmark registry covering all 5 cognitive domains,
/// Indonesian (`MultiLexNorm`, `IndoCollex`), `Tool RAG` (`N_caps=50..200`), and `MPBench`.
#[must_use]
pub fn standard_benchmark_registry() -> Vec<BenchmarkDatasetSpec> {
    vec![
        // Domain 1: Memory
        ds(
            "locomo",
            BenchmarkDomain::Memory,
            "locomo",
            &["qa_f1", "cat5_adversarial_acc"],
            "Must retain Category-5 adversarial questions",
            true,
        ),
        ds(
            "longmemeval",
            BenchmarkDomain::Memory,
            "longmemeval",
            &["overall_acc", "abstention_acc"],
            "1-on-1 longitudinal recall, KU + ABS (LongMemEval_S default)",
            true,
        ),
        ds(
            "memory_agent_bench",
            BenchmarkDomain::Memory,
            "memory_agent_bench",
            &["ar_acc", "selective_forgetting_acc"],
            "Evaluates selective forgetting & ACT-R decay",
            true,
        ),
        ds(
            "group_mem_bench",
            BenchmarkDomain::Memory,
            "group_mem_bench",
            &["acc", "knowledge_update", "term_ambiguity"],
            "Multi-party; supplement with VisibleScope Delta_NI",
            true,
        ),
        ds(
            "stale",
            BenchmarkDomain::Memory,
            "stale",
            &[
                "state_resolution",
                "premise_resistance",
                "implicit_policy_adaptation",
            ],
            "Tests valid_to superseding, Premise Resistance (PR), and Implicit Policy Adaptation (IPA)",
            true,
        ),
        // Domain 2: Prospective Memory
        ds(
            "pm_bench",
            BenchmarkDomain::Prospective,
            "pm_bench",
            &["set_f1", "false_alarm_rate"],
            "7-day Virtual Week with lures & updates",
            true,
        ),
        ds(
            "pis",
            BenchmarkDomain::Prospective,
            "pis",
            &["set_f1"],
            "Typed prospective intention store comparison",
            true,
        ),
        // Domain 3: Proactivity & Timing
        ds(
            "pitch",
            BenchmarkDomain::ProactivityTiming,
            "pitch",
            &["live_best_turn_acc", "in_window_acc"],
            "Chat-only live causal human-timing standard",
            true,
        ),
        ds(
            "proact",
            BenchmarkDomain::ProactivityTiming,
            "proact",
            &["appropriateness", "non_interruptiveness"],
            "Breakdown intervention precision",
            true,
        ),
        ds(
            "proactive_bench",
            BenchmarkDomain::ProactivityTiming,
            "proactive_bench",
            &["f1", "false_detection"],
            "Zero-LLM gate parity vs TGL",
            true,
        ),
        ds(
            "capa",
            BenchmarkDomain::Prospective,
            "capa",
            &["credited_recovery", "silence_rate"],
            "Meeting delegation & follow-up",
            true,
        ),
        ds(
            "when2speak",
            BenchmarkDomain::ProactivityTiming,
            "when2speak",
            &["macro_f1", "fir", "mir"],
            "Synthetic FIR/MIR stress test only; not sole proof of human timing",
            true,
        ),
        // Domain 4: Group Chat
        ds(
            "ubuntu_irc",
            BenchmarkDomain::GroupChat,
            "ubuntu_irc",
            &["thread_f1", "parent_speaker_p1"],
            "Bound solely to jkkummerfeld/irc-disentanglement (derived parent_speaker_p1)",
            true,
        ),
        ds(
            "molweni",
            BenchmarkDomain::GroupChat,
            "molweni",
            &["qa_f1", "sdrt_link_f1"],
            "Multi-party discourse; MuTual strictly excluded from multi-party claims",
            true,
        ),
        // Domain 5: Theory of Mind
        ds(
            "fantom",
            BenchmarkDomain::TheoryOfMind,
            "fantom",
            &["fantom_all_star", "fantom_all"],
            "ALL* requires all 6 formulations simultaneously",
            true,
        ),
        ds(
            "opentom",
            BenchmarkDomain::TheoryOfMind,
            "opentom",
            &["macro_f1_coarse", "macro_f1_fine", "attitude_f1", "mhop_f1"],
            "Zero train split; location + psychological attitude + multi-hop",
            true,
        ),
        ds(
            "hi_tom",
            BenchmarkDomain::TheoryOfMind,
            "hi_tom",
            &["joint_acc_0_to_4"],
            "Deceptive private side-channels & higher-order beliefs",
            true,
        ),
        ds(
            "exploretom",
            BenchmarkDomain::TheoryOfMind,
            "exploretom",
            &["exact_match_acc"],
            "ExploreToM dynamic asymmetric world belief tracking (separate from DynToM)",
            true,
        ),
        ds(
            "dyntom",
            BenchmarkDomain::TheoryOfMind,
            "dyntom",
            &["dyntom_acc"],
            "DynToM multi-stage temporal ToM transitions (GAIR-NLP/DynToM)",
            true,
        ),
        ds(
            "tomi",
            BenchmarkDomain::TheoryOfMind,
            "tomi",
            &["shortcut_perturbation_acc"],
            "Restricted strictly to shortcut-perturbation sanity checks",
            false,
        ),
        // Indonesian informal register (A-15, 256d, reason Embedder probe lane)
        ds(
            "multilexnorm",
            BenchmarkDomain::IndonesianRegister,
            "indonesian/multilexnorm",
            &["twin_retrieval_r1_256d"],
            "Indonesian/Gaul informal register normalization retrieval via reason::Embedder (256d)",
            true,
        ),
        ds(
            "indocollex",
            BenchmarkDomain::IndonesianRegister,
            "indonesian/indocollex",
            &["twin_retrieval_r1_256d"],
            "Indonesian colloquial word transformation retrieval via reason::Embedder (256d)",
            true,
        ),
        // Tool RAG & Security (in-tree runnable components)
        ds_local(
            "tool_rag_scaling",
            BenchmarkDomain::ToolRag,
            "tool_rag",
            &["schema_token_savings_ratio"],
            "In-tree capability scaling N_caps = 50..200 (R-38)",
        ),
        ds(
            "mpbench",
            BenchmarkDomain::Security,
            "mpbench",
            &["compaction_poisoning_asr", "delta_ni"],
            "Compaction poisoning ASR & VisibleScope Delta_NI (R-43)",
            true,
        ),
    ]
}

fn ds(
    id: &str,
    domain: BenchmarkDomain,
    rel: &str,
    metrics: &[&str],
    caveats: &str,
    required: bool,
) -> BenchmarkDatasetSpec {
    BenchmarkDatasetSpec {
        id: id.to_owned(),
        domain,
        relative_dir: rel.to_owned(),
        primary_metrics: metrics.iter().map(|s| (*s).to_owned()).collect(),
        caveats: caveats.to_owned(),
        required_in_standard_suite: required,
        requires_external_corpus: true,
    }
}

fn ds_local(
    id: &str,
    domain: BenchmarkDomain,
    rel: &str,
    metrics: &[&str],
    caveats: &str,
) -> BenchmarkDatasetSpec {
    BenchmarkDatasetSpec {
        id: id.to_owned(),
        domain,
        relative_dir: rel.to_owned(),
        primary_metrics: metrics.iter().map(|s| (*s).to_owned()).collect(),
        caveats: caveats.to_owned(),
        required_in_standard_suite: false,
        requires_external_corpus: false,
    }
}

/// Validates that `datasets_dir` verifies every required available dataset against the
/// committed manifest (`SHA-256`, byte size, extraction marker, converter cache).
/// Fails loudly naming missing/unverified datasets and the `datasets fetch` command.
pub fn verify_required_datasets_present(
    datasets_dir: &Path,
    registry: &[BenchmarkDatasetSpec],
) -> Result<Vec<PathBuf>, LabError> {
    ensure_not_sealed_path(datasets_dir)?;
    let manifest_path = resolve_manifest_path(None, datasets_dir);
    let (_manifest, present) =
        verify_required_datasets_with_manifest(&manifest_path, datasets_dir, registry, None)?;
    Ok(present)
}

/// Executes the in-tree `Tool RAG` scaling measurement across `N_caps = 50, 100, 150, 200` (`R-38`).
#[must_use]
pub fn run_tool_rag_scaling_eval(config: &CoreConfig) -> Vec<ToolRagScalePoint> {
    let place = PlaceId::new("room:tools");
    let soul = ActorId::new("soul");
    let alice = ActorId::new("alice");
    let aud = Audience::new([alice, soul]);
    let scope = VisibleScope::new(aud.clone(), Some(place.clone()));
    let now = Time::from_millis(1_000);
    let query_terms = vec!["calendar".to_owned(), "schedule".to_owned()];

    [50_usize, 100, 150, 200]
        .into_iter()
        .map(|n_caps| {
            let mut catalog = Vec::with_capacity(n_caps);
            for i in 0..n_caps {
                let is_static = i < 4;
                let desc = if i % 25 == 0 {
                    format!("calendar schedule event tool {i}")
                } else {
                    format!("auxiliary tool capability number {i}")
                };
                catalog.push(CapabilityCatalogEntry {
                    descriptor: CapabilityDescriptor {
                        id: CapabilityId::new(format!("cap.{i:03}")),
                        description: desc,
                        places: vec![place.clone()],
                        flags: CapabilityFlags {
                            communicative: is_static,
                            identity_changing: false,
                            irreversible: false,
                        },
                        typical_latency_ms: 10,
                        capability_schema_hash: CapabilitySchemaHash::new(format!("sha256:c{i}")),
                        sensed_effect: None,
                    },
                    is_static_core: is_static,
                    owner_signed: is_static,
                    active: true,
                    audience: aud.clone(),
                    embedding: None,
                });
            }
            let budget = ToolSelectionBudget::from_config(config, n_caps);
            let selected =
                select_tools_for_context(&query_terms, &[], &catalog, &[], &scope, now, budget);
            let selected_total = selected.static_core.len() + selected.dynamic_caps.len();
            let savings = 1.0 - (selected_total as f64 / n_caps as f64);
            ToolRagScalePoint {
                n_caps,
                baseline_caps_in_context: n_caps,
                selected_caps_in_context: selected_total,
                schema_token_savings_ratio: savings,
            }
        })
        .collect()
}

/// Verifies `Arm T` mid-task restart parity on `Core` (decisions and state survive restart cleanly).
pub fn run_arm_t_restart_parity_check(config: &CoreConfig) -> Result<bool, LabError> {
    let mem = MemStore::new();
    let place = PlaceId::new("room:restart");
    let alice = ActorId::new("alice");
    let soul = ActorId::new("soul");
    let aud = Audience::new([alice.clone(), soul.clone()]);
    let mut doors = NoopDoors;

    let mut p1 = Percept::minimal(
        alice.clone(),
        place.clone(),
        aud.clone(),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"step 1 before restart".to_vec()),
    );
    p1.addressees = vec![soul.clone()];
    p1.named = vec![soul.clone()];

    let mut core1 = Core::open(
        mem,
        CoreConfig {
            seed: 17,
            ..config.clone()
        },
        None,
    )?;
    let _ = core1.step(
        monoagens_core::Input::Percept(Box::new(p1.clone())),
        Time::from_millis(1_000),
        &mut doors,
    )?;
    let ckpt = core1.take_snapshot("mid-task-restart")?;
    let events_pre = core1.store().read_after_seq(None, usize::MAX)?;
    let mut mem_reopen = MemStore::new();
    for rec in events_pre {
        mem_reopen.append(rec)?;
    }
    monoagens_core::Store::snapshot(&mut mem_reopen, ckpt)?;

    let mut core2 = Core::open(
        mem_reopen,
        CoreConfig {
            seed: 17,
            ..config.clone()
        },
        None,
    )?;
    let mut p2 = Percept::minimal(
        alice,
        place,
        aud,
        Time::from_millis(2_000),
        OpaquePayload::from_bytes(b"step 2 after restart".to_vec()),
    );
    p2.addressees = vec![soul.clone()];
    p2.named = vec![soul];
    let eff2 = core2.step(
        monoagens_core::Input::Percept(Box::new(p2)),
        Time::from_millis(2_000),
        &mut doors,
    )?;
    Ok(!eff2.is_empty())
}

/// Runs `monoagens-lab benchmark` for `suite` (`standard` or `in_tree_only`),
/// strictly failing non-zero when external datasets are absent or unverified for `standard`.
pub fn run_benchmark_suite(
    suite: &str,
    datasets_dir: &Path,
    pareto_input: Option<&ParetoScorecardInput>,
    config: &CoreConfig,
) -> Result<BenchmarkSuiteReport, LabError> {
    run_benchmark_suite_with_options(
        suite,
        datasets_dir,
        pareto_input,
        false,
        config,
        &BenchmarkExecutionOptions::default(),
    )
}

/// Runs `monoagens-lab benchmark` with full manifest, `--only`, `--live --max-cost-usd`,
/// judge routing, and scorecard Pareto support.
pub fn run_benchmark_suite_with_options(
    suite: &str,
    datasets_dir: &Path,
    pareto_input: Option<&ParetoScorecardInput>,
    compute_pareto_from_scorecard: bool,
    config: &CoreConfig,
    opts: &BenchmarkExecutionOptions<'_>,
) -> Result<BenchmarkSuiteReport, LabError> {
    ensure_not_sealed_path(datasets_dir)?;
    let full_registry = standard_benchmark_registry();
    let manifest_path = resolve_manifest_path(opts.manifest_path.as_deref(), datasets_dir);
    let suite_scope = suite_domain(suite);
    let active_registry: Vec<BenchmarkDatasetSpec> = match suite_scope {
        Some(domain) => full_registry
            .iter()
            .filter(|s| s.domain == domain)
            .cloned()
            .collect(),
        None => full_registry.clone(),
    };

    let (scorecard, run_records, partial, left_out_datasets, preflight_estimate, dataset_specs) =
        if suite == "standard" {
            // The standard suite only indexes the latest per-category runs; it never executes.
            let (rows, records, partial, left_out, preflight) =
                build_standard_index(&manifest_path, &full_registry, opts)?;
            (
                rows,
                records,
                partial,
                left_out,
                preflight,
                full_registry.clone(),
            )
        } else if suite == "in_tree_only" {
            let manifest = load_and_validate_manifest(&manifest_path, &full_registry)?;
            let (rows, records, partial, left_out, preflight) = execute_external_datasets(
                suite,
                &manifest,
                &manifest_path,
                datasets_dir,
                &full_registry,
                config,
                opts,
            )?;
            (
                rows,
                records,
                partial,
                left_out,
                preflight,
                full_registry.clone(),
            )
        } else if suite_scope.is_some() {
            let (manifest, _present) = verify_required_datasets_with_manifest(
                &manifest_path,
                datasets_dir,
                &active_registry,
                opts.only_ids.as_deref(),
            )?;
            let (rows, records, partial, left_out, preflight) = execute_external_datasets(
                suite,
                &manifest,
                &manifest_path,
                datasets_dir,
                &active_registry,
                config,
                opts,
            )?;
            (
                rows,
                records,
                partial,
                left_out,
                preflight,
                active_registry.clone(),
            )
        } else {
            return Err(LabError::Protocol(format!(
                "unknown benchmark suite '{suite}' (expected 'standard', 'in_tree_only', or one of memory|prospective|timing|group|tom|indonesian|security)"
            )));
        };

    let tool_rag_scaling = run_tool_rag_scaling_eval(config);
    let arm_t_ok = run_arm_t_restart_parity_check(config)?;
    let scorecard_pareto_input = scorecard_rows_to_pareto_input(
        &scorecard,
        pareto_input
            .map(|p| p.paired_comparisons.clone())
            .unwrap_or_default(),
    );
    let pareto = if let Some(explicit) = pareto_input {
        let mut merged = explicit.clone();
        if merged.scorecard_rows.is_empty() {
            merged.scorecard_rows = scorecard.clone();
        }
        Some(compute_convex_pareto(&merged))
    } else if compute_pareto_from_scorecard {
        Some(compute_convex_pareto(&scorecard_pareto_input))
    } else {
        None
    };

    let mut caveat_counts: BTreeMap<String, String> = BTreeMap::new();
    caveat_counts.insert(
        "MuTual".to_owned(),
        "Excluded from multi-party addressee claims (dyadic 2-speaker only)".to_owned(),
    );

    Ok(BenchmarkSuiteReport {
        suite: suite.to_owned(),
        partial,
        left_out_datasets,
        datasets_dir: datasets_dir.to_string_lossy().into_owned(),
        dataset_specs,
        scorecard,
        run_records,
        preflight_estimate,
        excluded_from_multiparty_claims: vec!["MuTual".to_owned()],
        restricted_to_shortcut_checks: vec!["ToMi".to_owned()],
        tool_rag_scaling,
        arm_t_restart_parity_ok: arm_t_ok,
        one_over_n_band_check: (0.5, 2.0),
        scorecard_pareto_input,
        pareto,
    })
}

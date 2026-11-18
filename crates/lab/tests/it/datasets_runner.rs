use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use monoagens_core::{Prior, Switches};
use monoagens_lab::datasets::{
    BenchmarkExecutionOptions, ChecksumsRecorded, ConvertedDataset, Converter, CoreApplicability,
    DatasetAvailability, DatasetHandlerRegistry, DatasetLane, DatasetManifestEntry,
    EXTRACTION_MARKER_FILE, EmbedderProbe, EmbedderProbeOutcome, EmbedderReadiness, ExternalItem,
    ExternalPrediction, ExternalProbe, ExtractionMarker, GoldBook, GoldEntry, GoldEvidenceRef,
    HistoryMode, JudgeConfig, JudgeExecutionConfig, JudgeSubstitutionPolicy, LicenseStatus,
    ManifestEntryKind, PinnedSha256, PinnedSize, Scorer, ScorerMetric, ScorerReport,
    load_and_validate_manifest, load_lab_provider_secrets, make_text_percept,
    save_manifest_to_path, validate_semantic_embedder,
};
use monoagens_lab::{
    BenchmarkDomain, LabError, RunRecord, ScorecardStatus, compute_convex_pareto,
    run_benchmark_suite_with_options, sha256_hex, standard_benchmark_registry,
};
use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};
use monoagens_reason::embed::{Embedder, ScriptedEmbedder};
use monoagens_reason::model::{
    ModelOutput, ModelProvider, ModelRequest, OpenAiModelConfig, OpenAiModelProvider,
    ScriptedModelProvider, ScriptedReply,
};
use serde_json::Value;
use tempfile::TempDir;

fn lab_bin() -> &'static str {
    env!("CARGO_BIN_EXE_monoagens-lab")
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .to_path_buf()
}

fn seed_verified_dataset_file(
    entry: &mut DatasetManifestEntry,
    datasets_dir: &Path,
    rel_file: &str,
    content: &[u8],
) {
    let ds_dir = datasets_dir.join(&entry.relative_dir);
    let file_path = ds_dir.join(rel_file);
    fs::create_dir_all(file_path.parent().unwrap()).unwrap();
    fs::write(&file_path, content).unwrap();

    for src in &mut entry.sources {
        src.revision = "pinned-rev-1".to_owned();
    }
    entry.license.status = LicenseStatus::Cleared;
    entry.checksums_recorded = ChecksumsRecorded::Owner;
    entry.checksums_recorded_date = Some("2026-11-17".to_owned());
    entry.files[0].path = rel_file.to_owned();
    entry.files[0].sha256 = PinnedSha256::Sha256(sha256_hex(content));
    entry.files[0].size = PinnedSize::Bytes(content.len() as u64);
    if entry.files.len() > 1 {
        entry.files.truncate(1);
    }

    let revs: BTreeMap<String, String> = entry
        .sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            (
                s.role.clone().unwrap_or_else(|| format!("source_{i}")),
                s.revision.clone(),
            )
        })
        .collect();
    let marker = ExtractionMarker {
        dataset_id: entry.id.clone(),
        revisions: revs,
        files_count: 1,
        extracted_ok: true,
    };
    fs::write(
        ds_dir.join(EXTRACTION_MARKER_FILE),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();
}

struct StubBehavioralConverter {
    id: String,
    version: String,
}

impl Converter for StubBehavioralConverter {
    fn dataset_id(&self) -> &str {
        &self.id
    }
    fn converter_version(&self) -> &str {
        &self.version
    }
    fn convert(
        &self,
        _dataset_dir: &Path,
        entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let soul = ActorId::new("soul");
        let user = ActorId::new("user");
        let pa = ActorId::new("prior_assistant");
        let place = PlaceId::new(format!("place_{}", self.id));
        let hist_aud = Audience::new([user.clone(), pa, soul.clone()]);
        let probe_aud = Audience::new([user.clone(), soul]);

        let item = ExternalItem {
            item_id: format!("{}_0", self.id),
            cluster_id: format!("{}_cluster_0", self.id),
            strata_key: "default".to_owned(),
            history: vec![make_text_percept(
                "user",
                place.as_str(),
                hist_aud.members.iter().cloned(),
                1_000,
                "My favorite color is cobalt.",
            )],
            checkpoint_group: None,
            probes: vec![ExternalProbe {
                probe_id: "p0".to_owned(),
                percept: make_text_percept(
                    "user",
                    place.as_str(),
                    probe_aud.members.iter().cloned(),
                    2_000,
                    "What is my favorite color?",
                ),
                isolated_checkpoint: true,
                probe_scope: VisibleScope::new(probe_aud, Some(place.clone())),
                expected_capability: None,
            }],
            history_mode: HistoryMode::StaticSilent,
            capabilities: Vec::new(),
        };

        let gold = GoldBook {
            dataset_id: self.id.clone(),
            entries: vec![GoldEntry {
                item_id: item.item_id.clone(),
                probe_id: "p0".to_owned(),
                cluster_id: item.cluster_id.clone(),
                gold_answer: "cobalt".to_owned(),
                accepted_answers: vec!["cobalt".to_owned()],
                gold_evidence: vec![GoldEvidenceRef {
                    turn_index: 0,
                    audience: hist_aud,
                    place,
                }],
                privacy_conflict: false,
                category: None,
                metadata: BTreeMap::new(),
            }],
        };

        Ok(ConvertedDataset {
            dataset_id: self.id.clone(),
            pinned_revision: entry
                .sources
                .first()
                .map_or_else(|| "rev".to_owned(), |s| s.revision.clone()),
            converter_version: self.version.clone(),
            items: vec![item],
            gold_book: gold,
        })
    }
}

struct StubScorer {
    id: String,
    requires_judge: bool,
    metric_name: String,
}

impl Scorer for StubScorer {
    fn dataset_id(&self) -> &str {
        &self.id
    }
    fn scorer_version(&self) -> &str {
        "1.0.0"
    }
    fn score(
        &self,
        predictions: &[ExternalPrediction],
        _gold: &GoldBook,
        judge: &JudgeExecutionConfig<'_>,
        _seed: u64,
    ) -> Result<ScorerReport, LabError> {
        if self.requires_judge && !judge.live {
            return Ok(ScorerReport {
                dataset_id: self.id.clone(),
                scorer_version: "1.0.0".to_owned(),
                metrics: vec![ScorerMetric {
                    name: self.metric_name.clone(),
                    value: None,
                    ci95_low: None,
                    ci95_high: None,
                    n_clusters: None,
                    not_scored_reason: Some("requires --live judge".to_owned()),
                }],
                judge_model: None,
                judge_prompt_version: None,
            });
        }
        let mut score = if predictions.is_empty() { 0.0 } else { 1.0 };
        if let Some(jp) = judge.provider {
            let req = ModelRequest::text(
                monoagens_protocol::EventId::new(1),
                monoagens_core::ModelPurpose::Classify,
                judge.judge_model.unwrap_or("gpt-4o"),
                "judge prompt",
                "grade answer",
            );
            let res = jp
                .invoke(&req)
                .map_err(|e| LabError::Protocol(e.to_string()))?;
            if let ModelOutput::Verdict(v) = res.output {
                score = if v.label == "yes" { 1.0 } else { 0.0 };
            }
        }
        Ok(ScorerReport {
            dataset_id: self.id.clone(),
            scorer_version: "1.0.0".to_owned(),
            metrics: vec![ScorerMetric {
                name: self.metric_name.clone(),
                value: Some(score),
                ci95_low: Some(score),
                ci95_high: Some(score),
                n_clusters: Some(1),
                not_scored_reason: None,
            }],
            judge_model: judge.judge_model.map(str::to_owned),
            judge_prompt_version: judge.judge_prompt_version.map(str::to_owned),
        })
    }
}

struct StubEmbedderProbe {
    id: String,
}

impl EmbedderProbe for StubEmbedderProbe {
    fn dataset_id(&self) -> &str {
        &self.id
    }
    fn probe_version(&self) -> &str {
        "1.0.0"
    }
    fn run_probe(
        &self,
        _dataset_dir: &Path,
        embedder: Option<&dyn Embedder>,
        allow_test_embedders: bool,
        _target_n: Option<usize>,
        _seed: u64,
    ) -> Result<EmbedderProbeOutcome, LabError> {
        match validate_semantic_embedder(embedder, allow_test_embedders)? {
            EmbedderReadiness::Unconfigured(reason) => Ok(EmbedderProbeOutcome::NotScored {
                dataset_id: self.id.clone(),
                probe_version: "1.0.0".to_owned(),
                reason,
            }),
            EmbedderReadiness::Ready => {
                let emb = embedder.unwrap();
                Ok(EmbedderProbeOutcome::Scored {
                    dataset_id: self.id.clone(),
                    probe_version: "1.0.0".to_owned(),
                    embed_model_ver: emb.model_ver().unwrap_or("test").to_owned(),
                    dim_d: emb.dim_d().unwrap_or(256),
                    metrics: vec![ScorerMetric {
                        name: "twin_retrieval_r1_256d".to_owned(),
                        value: Some(0.75),
                        ci95_low: Some(0.65),
                        ci95_high: Some(0.85),
                        n_clusters: None,
                        not_scored_reason: None,
                    }],
                    item_count: 2,
                    chosen_item_ids: vec!["w0".to_owned(), "w1".to_owned()],
                })
            }
        }
    }
}

#[test]
fn standard_suite_fails_before_run_on_missing_and_checksum_mismatch() {
    let tmp = TempDir::new().unwrap();
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    fs::create_dir_all(&datasets_dir).unwrap();

    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();

    // Seed EVERY required available dataset EXCEPT tamper one byte in `locomo`.
    // ScorerModel entries are intentionally left unseeded to verify `--suite standard` does not fail when optional scorer models are absent.
    for entry in &mut manifest.datasets {
        if entry.availability == DatasetAvailability::Unavailable
            || entry.kind != ManifestEntryKind::Dataset
        {
            continue;
        }
        let rel = entry.files[0].path.clone();
        seed_verified_dataset_file(entry, &datasets_dir, &rel, b"verified-fixture-payload");
    }
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    // 1. Tamper 1 byte in `locomo` -> directory exists (old is_dir() would pass!), but manifest
    //    verification catches checksum mismatch and fails before running any dataset!
    let locomo_file = datasets_dir.join("locomo/data/locomo10.json");
    fs::write(&locomo_file, b"Xverified-fixture-payload").unwrap();

    let opts = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path.clone()),
        ..BenchmarkExecutionOptions::default()
    };
    let err_tamper = run_benchmark_suite_with_options(
        "memory",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts,
    )
    .expect_err("checksum mismatch must fail --suite memory before any run");
    let msg = err_tamper.to_string();
    assert!(msg.contains("locomo"), "msg={msg}");
    assert!(msg.contains("sha256 mismatch"), "msg={msg}");
    assert!(msg.contains("datasets fetch locomo"), "msg={msg}");

    // 2. Remove `longmemeval` file -> fails naming `longmemeval` and `datasets fetch longmemeval`.
    fs::write(&locomo_file, b"verified-fixture-payload").unwrap();
    fs::remove_dir_all(datasets_dir.join("longmemeval")).unwrap();

    let err_missing = run_benchmark_suite_with_options(
        "memory",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts,
    )
    .expect_err("missing dataset must fail --suite memory");
    let msg2 = err_missing.to_string();
    assert!(msg2.contains("longmemeval"), "msg={msg2}");
    assert!(msg2.contains("datasets fetch longmemeval"), "msg={msg2}");
}

#[test]
fn cli_benchmark_only_produces_partial_report_and_run_records() {
    let tmp = TempDir::new().unwrap();
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    let out_dir = tmp.path().join("run_records");
    fs::create_dir_all(&datasets_dir).unwrap();

    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();
    seed_verified_dataset_file(
        manifest.get_mut("locomo").unwrap(),
        &datasets_dir,
        "data/locomo10.json",
        br#"[{"sample_id":"s0","conversation":{"speaker_a":"alice","speaker_b":"bob","session_1":[{"speaker":"alice","dia_id":"D1:1","text":"hi"}]},"qa":[{"question":"Who greeted?","answer":"alice","evidence":["D1:1"],"category":1}]}]"#,
    );
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    // Run `--only locomo` inside the memory category even though the other memory datasets are
    // absent (passing --allow-no-embedder in this offline test without EmbeddingGemma 2 weights).
    let out = Command::new(lab_bin())
        .args([
            "benchmark",
            "--suite",
            "memory",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
            "--only",
            "locomo",
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--allow-no-embedder",
            "--pareto",
            "--json",
        ])
        .output()
        .expect("run benchmark --only locomo");
    assert!(
        out.status.success(),
        "benchmark --only locomo failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["partial"], true);
    let left_out = report["left_out_datasets"].as_array().unwrap();
    assert!(left_out.iter().any(|v| v == "longmemeval"));
    assert!(!left_out.iter().any(|v| v == "locomo"));

    let external_count = registry
        .iter()
        .filter(|s| s.requires_external_corpus && s.domain == BenchmarkDomain::Memory)
        .count();
    assert_eq!(
        report["scorecard"].as_array().unwrap().len(),
        external_count
    );
}

#[test]
fn scorecard_covers_all_statuses_lanes_and_pareto_gaps_without_imputation() {
    let tmp = TempDir::new().unwrap();
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    let out_dir = tmp.path().join("records");
    fs::create_dir_all(&datasets_dir).unwrap();

    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();

    // Seed verified fixtures for:
    // - `locomo` -> `run` (core_behavioral)
    // - `longmemeval` -> `not_scored` offline (requires --live judge)
    // - `multilexnorm` -> `run` (embedder_probe with ScriptedEmbedder in test mode)
    // - `indocollex` -> kept missing -> `unverified` in `in_tree_only` diagnostic mode
    // - `pitch` -> mark `core_applicability = not_mappable` in fixture to exercise `not_mappable` status
    // - `pis`, `proact`, `mpbench` -> `unavailable`
    // - `stale` -> `partial` (one deterministic metric + one `--live`-only metric)
    seed_verified_dataset_file(
        manifest.get_mut("locomo").unwrap(),
        &datasets_dir,
        "data/locomo10.json",
        b"locomo",
    );
    seed_verified_dataset_file(
        manifest.get_mut("longmemeval").unwrap(),
        &datasets_dir,
        "longmemeval_s_cleaned.json",
        b"lme",
    );
    seed_verified_dataset_file(
        manifest.get_mut("stale").unwrap(),
        &datasets_dir,
        "T1_T2_400_FULL.json",
        b"stale",
    );
    seed_verified_dataset_file(
        manifest.get_mut("multilexnorm").unwrap(),
        &datasets_dir,
        "ind/test-00000-of-00001.parquet",
        b"mln",
    );
    {
        let p = manifest.get_mut("pitch").unwrap();
        seed_verified_dataset_file(p, &datasets_dir, "data/pitch-benchmark-data.json", b"pitch");
        p.core_applicability = CoreApplicability::NotMappable;
    }
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    let mut handlers = DatasetHandlerRegistry::default();
    handlers.register_behavioral(
        Box::new(StubBehavioralConverter {
            id: "locomo".to_owned(),
            version: "1.0.0".to_owned(),
        }),
        Box::new(StubScorer {
            id: "locomo".to_owned(),
            requires_judge: false,
            metric_name: "qa_f1".to_owned(),
        }),
    );
    handlers.register_behavioral(
        Box::new(StubBehavioralConverter {
            id: "longmemeval".to_owned(),
            version: "1.0.0".to_owned(),
        }),
        Box::new(StubScorer {
            id: "longmemeval".to_owned(),
            requires_judge: true,
            metric_name: "overall_acc".to_owned(),
        }),
    );
    // Custom scorer producing 1 scored + 1 not_scored metric -> `partial` row status.
    struct PartialStubScorer;
    impl Scorer for PartialStubScorer {
        fn dataset_id(&self) -> &str {
            "stale"
        }
        fn scorer_version(&self) -> &str {
            "1.0.0"
        }
        fn score(
            &self,
            _predictions: &[ExternalPrediction],
            _gold: &GoldBook,
            _judge: &JudgeExecutionConfig<'_>,
            _seed: u64,
        ) -> Result<ScorerReport, LabError> {
            Ok(ScorerReport {
                dataset_id: "stale".to_owned(),
                scorer_version: "1.0.0".to_owned(),
                metrics: vec![
                    ScorerMetric {
                        name: "state_resolution".to_owned(),
                        value: Some(0.8),
                        ci95_low: Some(0.7),
                        ci95_high: Some(0.9),
                        n_clusters: Some(1),
                        not_scored_reason: None,
                    },
                    ScorerMetric {
                        name: "premise_resistance".to_owned(),
                        value: None,
                        ci95_low: None,
                        ci95_high: None,
                        n_clusters: None,
                        not_scored_reason: Some("requires --live judge".to_owned()),
                    },
                ],
                judge_model: None,
                judge_prompt_version: None,
            })
        }
    }
    handlers.register_behavioral(
        Box::new(StubBehavioralConverter {
            id: "stale".to_owned(),
            version: "2.0.0".to_owned(),
        }),
        Box::new(PartialStubScorer),
    );
    handlers.register_embedder_probe(Box::new(StubEmbedderProbe {
        id: "multilexnorm".to_owned(),
    }));

    let mut scripted_emb = ScriptedEmbedder::new("egemma2-256", 256);
    scripted_emb.set_fallback(Ok(vec![1.0; 256]));

    let opts = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path),
        out_dir: Some(out_dir.clone()),
        allow_test_embedders: true,
        embedder: Some(&scripted_emb),
        handlers,
        ..BenchmarkExecutionOptions::default()
    };

    let report = run_benchmark_suite_with_options(
        "in_tree_only",
        &datasets_dir,
        None,
        true,
        &Prior::default(),
        &opts,
    )
    .expect("in_tree_only scorecard run");

    let external_count = registry
        .iter()
        .filter(|s| s.requires_external_corpus)
        .count();
    assert_eq!(report.scorecard.len(), external_count);

    let find_row = |id: &str| {
        report
            .scorecard
            .iter()
            .find(|r| r.dataset_id == id)
            .unwrap_or_else(|| panic!("missing scorecard row {id}"))
    };
    assert_eq!(find_row("locomo").status, ScorecardStatus::Run);
    assert_eq!(find_row("locomo").lane, DatasetLane::CoreBehavioral);
    assert_eq!(find_row("stale").status, ScorecardStatus::Partial);
    assert_eq!(find_row("longmemeval").status, ScorecardStatus::NotScored);
    assert_eq!(find_row("pis").status, ScorecardStatus::Unavailable);
    assert_eq!(find_row("proact").status, ScorecardStatus::Unavailable);
    assert_eq!(find_row("mpbench").status, ScorecardStatus::Unavailable);
    assert_eq!(find_row("pitch").status, ScorecardStatus::NotMappable);
    assert_eq!(find_row("indocollex").status, ScorecardStatus::Unverified);
    assert_eq!(find_row("multilexnorm").status, ScorecardStatus::Run);
    assert_eq!(find_row("multilexnorm").lane, DatasetLane::EmbedderProbe);

    // Pareto uses scored rows only (`locomo`, `stale`, `multilexnorm`); gap rows stay in `gaps` with zero imputation.
    let pareto = report.pareto.as_ref().expect("pareto report present");
    assert!(
        pareto
            .undominated_frontier
            .iter()
            .chain(pareto.dominated_points.iter().filter_map(|_| None))
            .all(|s| matches!(
                s.status,
                Some(ScorecardStatus::Run | ScorecardStatus::Partial)
            ))
    );
    assert!(pareto.gaps.iter().any(|g| g.dataset_id == "pis"));
    assert!(pareto.gaps.iter().any(|g| g.dataset_id == "longmemeval"));
    assert!(pareto.gaps.iter().any(|g| g.dataset_id == "indocollex"));

    // Explicit RunRecord files written to out_dir.
    let rec_bytes = fs::read(out_dir.join("locomo__seed-1.postcard")).unwrap();
    let rec = RunRecord::decode_postcard(&rec_bytes).unwrap();
    assert_eq!(
        rec.history_reasoning_mode.as_deref(),
        Some("scripted_silent")
    );
    assert_eq!(
        rec.dataset_provenance.as_ref().unwrap().dataset_id,
        "locomo"
    );

    // Also verify `compute_convex_pareto` excludes any explicitly passed gap row on `systems`.
    let mut manual_pareto = report.scorecard_pareto_input.clone();
    if let Some(mut first) = manual_pareto.systems.first().cloned() {
        first.name = "fake_gap".to_owned();
        first.status = Some(ScorecardStatus::Unavailable);
        manual_pareto.systems.push(first);
    }
    let recomputed = compute_convex_pareto(&manual_pareto);
    assert!(
        !recomputed
            .undominated_frontier
            .iter()
            .any(|s| s.name == "fake_gap")
    );
    assert!(!recomputed.dominated_points.iter().any(|n| n == "fake_gap"));
}

#[test]
fn live_preflight_budget_cap_silent_history_and_judge_substitution_policy() {
    let tmp = TempDir::new().unwrap();
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    fs::create_dir_all(&datasets_dir).unwrap();

    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();
    seed_verified_dataset_file(
        manifest.get_mut("longmemeval").unwrap(),
        &datasets_dir,
        "longmemeval_s_cleaned.json",
        b"lme",
    );
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    let make_handlers = || {
        let mut h = DatasetHandlerRegistry::default();
        h.register_behavioral(
            Box::new(StubBehavioralConverter {
                id: "longmemeval".to_owned(),
                version: "1.0.0".to_owned(),
            }),
            Box::new(StubScorer {
                id: "longmemeval".to_owned(),
                requires_judge: true,
                metric_name: "overall_acc".to_owned(),
            }),
        );
        h
    };

    // 1. `--live` without `--max-cost-usd` fails before any call.
    let opts_no_cap = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path.clone()),
        only_ids: Some(vec!["longmemeval".to_owned()]),
        live: true,
        max_cost_usd: None,
        handlers: make_handlers(),
        ..BenchmarkExecutionOptions::default()
    };
    let err_no_cap = run_benchmark_suite_with_options(
        "memory",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts_no_cap,
    )
    .expect_err("--live without --max-cost-usd must fail");
    assert!(err_no_cap.to_string().contains("--max-cost-usd"));

    // 2. `--live` with `--max-cost-usd` smaller than preflight worst-case USD aborts before any paid call.
    let probe_provider = {
        let mut p = ScriptedModelProvider::new();
        p.set_fallback(ScriptedReply::text(
            r#"{"acts":[{"act":"act","capability_id":"a.speak","places":["place_longmemeval"],"people":["user"],"payload":[99,111,98,97,108,116],"motivation":5,"confidence_0_20":19,"used_ids":[]}]}"#,
        ));
        p
    };
    let judge_provider = {
        let mut p = ScriptedModelProvider::new();
        p.set_fallback(ScriptedReply::text(
            r#"{"label":"yes","score":1.0,"rationale":"matches gold"}"#,
        ));
        p
    };

    let opts_too_small = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path.clone()),
        only_ids: Some(vec!["longmemeval".to_owned()]),
        live: true,
        max_cost_usd: Some(0.0001),
        probe_provider: Some(&probe_provider as &dyn ModelProvider),
        judge_provider: Some(&judge_provider as &dyn ModelProvider),
        judge_config: Some(JudgeConfig {
            endpoint_url: "http://127.0.0.1:9".to_owned(),
            model: "gpt-4o".to_owned(),
            prompt_version: "lme-v1".to_owned(),
            substitution_policy: JudgeSubstitutionPolicy::StrictFail,
        }),
        handlers: make_handlers(),
        ..BenchmarkExecutionOptions::default()
    };
    let err_budget = run_benchmark_suite_with_options(
        "memory",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts_too_small,
    )
    .expect_err("over-budget preflight must abort before any paid call");
    assert!(err_budget.to_string().contains("preflight"));

    // 3. Official judge `gpt-4o` within budget emits official metric `overall_acc` + `scripted_silent`.
    let opts_official = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path.clone()),
        only_ids: Some(vec!["longmemeval".to_owned()]),
        live: true,
        max_cost_usd: Some(5.0),
        probe_model_name: "mock-live-model".to_owned(),
        probe_provider: Some(&probe_provider as &dyn ModelProvider),
        judge_provider: Some(&judge_provider as &dyn ModelProvider),
        judge_config: Some(JudgeConfig {
            endpoint_url: "http://127.0.0.1:9".to_owned(),
            model: "gpt-4o".to_owned(),
            prompt_version: "lme-v1".to_owned(),
            substitution_policy: JudgeSubstitutionPolicy::StrictFail,
        }),
        handlers: make_handlers(),
        ..BenchmarkExecutionOptions::default()
    };
    let rep_official = run_benchmark_suite_with_options(
        "memory",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts_official,
    )
    .expect("official judge live path");
    assert!(rep_official.preflight_estimate.is_some());
    assert_eq!(
        rep_official.run_records[0]
            .history_reasoning_mode
            .as_deref(),
        Some("live")
    );
    assert_eq!(
        rep_official.run_records[0].monoagens_mode.as_deref(),
        Some("invalid"),
        "scripted providers in a live run are never full MonoAgens"
    );
    let lme_row = rep_official
        .scorecard
        .iter()
        .find(|r| r.dataset_id == "longmemeval")
        .unwrap();
    assert_eq!(lme_row.status, ScorecardStatus::NotScored);
    assert_eq!(
        lme_row.reason.as_deref(),
        Some("invalid: not full MonoAgens")
    );
    assert_eq!(
        lme_row.metrics.get("overall_acc").copied().flatten(),
        Some(1.0)
    );

    // 4. Non-official judge under RenameMetric emits `overall_acc_non_official_judge`.
    let opts_rename = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path.clone()),
        only_ids: Some(vec!["longmemeval".to_owned()]),
        live: true,
        max_cost_usd: Some(5.0),
        probe_model_name: "mock-live-model".to_owned(),
        probe_provider: Some(&probe_provider as &dyn ModelProvider),
        judge_provider: Some(&judge_provider as &dyn ModelProvider),
        judge_config: Some(JudgeConfig {
            endpoint_url: "http://127.0.0.1:9".to_owned(),
            model: "llama-3.3-70b".to_owned(),
            prompt_version: "lme-v1".to_owned(),
            substitution_policy: JudgeSubstitutionPolicy::RenameMetric,
        }),
        handlers: make_handlers(),
        ..BenchmarkExecutionOptions::default()
    };
    let rep_rename = run_benchmark_suite_with_options(
        "memory",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts_rename,
    )
    .expect("renamed judge path");
    let lme_renamed = rep_rename
        .scorecard
        .iter()
        .find(|r| r.dataset_id == "longmemeval")
        .unwrap();
    assert_eq!(
        lme_renamed
            .metrics
            .get("overall_acc_non_official_judge")
            .copied()
            .flatten(),
        Some(1.0)
    );
    assert_eq!(
        rep_rename.run_records[0]
            .dataset_provenance
            .as_ref()
            .unwrap()
            .judge_model
            .as_deref(),
        Some("llama-3.3-70b")
    );

    // 5. Non-official judge under StrictFail fails cleanly.
    let opts_strict_fail = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path),
        only_ids: Some(vec!["longmemeval".to_owned()]),
        live: true,
        max_cost_usd: Some(5.0),
        probe_provider: Some(&probe_provider as &dyn ModelProvider),
        judge_provider: Some(&judge_provider as &dyn ModelProvider),
        judge_config: Some(JudgeConfig {
            endpoint_url: "http://127.0.0.1:9".to_owned(),
            model: "llama-3.3-70b".to_owned(),
            prompt_version: "lme-v1".to_owned(),
            substitution_policy: JudgeSubstitutionPolicy::StrictFail,
        }),
        handlers: make_handlers(),
        ..BenchmarkExecutionOptions::default()
    };
    let err_sub = run_benchmark_suite_with_options(
        "memory",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts_strict_fail,
    )
    .expect_err("StrictFail must refuse non-official judge");
    assert!(err_sub.to_string().contains("judge substitution refused"));
}

#[test]
fn capa_live_judge_uses_rust_openai_provider_secrets_cost_guard_and_substitution() {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use std::thread;

    let tmp = TempDir::new().unwrap();
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    let out_dir = tmp.path().join("records");
    let secrets_path = tmp.path().join("secrets.toml");
    fs::create_dir_all(&datasets_dir).unwrap();

    // Start loopback mock OpenAI-compatible judge server that asserts bearer token from secrets.toml
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let addr = listener.local_addr().unwrap();
    let endpoint_url = format!("http://{addr}/v1/chat/completions");
    let stop = Arc::new(AtomicBool::new(false));
    let hits = Arc::new(AtomicUsize::new(0));
    let auth_ok = Arc::new(AtomicBool::new(true));
    let tokens_per_call = Arc::new(AtomicUsize::new(500));
    let stop_c = Arc::clone(&stop);
    let hits_c = Arc::clone(&hits);
    let auth_c = Arc::clone(&auth_ok);
    let tok_c = Arc::clone(&tokens_per_call);

    let handle = thread::spawn(move || {
        while !stop_c.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let mut buf = [0u8; 16384];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        continue;
                    }
                    let req = String::from_utf8_lossy(&buf[..n]);
                    if !req.contains("Authorization: Bearer sk-lab-secrets-canary-01")
                        && !req.contains("authorization: Bearer sk-lab-secrets-canary-01")
                    {
                        auth_c.store(false, Ordering::SeqCst);
                    }
                    hits_c.fetch_add(1, Ordering::SeqCst);
                    let pt = tok_c.load(Ordering::SeqCst);
                    let body = serde_json::json!({
                        "model": "custom-judge-v1",
                        "choices": [{
                            "message": {
                                "content": r#"{"label":"yes","score":1.0,"rationale":"credited without hallucination"}"#
                            }
                        }],
                        "usage": {
                            "prompt_tokens": pt,
                            "completion_tokens": pt,
                            "total_tokens": pt * 2
                        }
                    })
                    .to_string();
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    let _ = stream.flush();
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });

    // Write secrets.toml and load through `load_lab_provider_secrets`.
    fs::write(
        &secrets_path,
        format!("[model]\napi_key = \"sk-lab-secrets-canary-01\"\nendpoint = \"{endpoint_url}\"\n"),
    )
    .unwrap();
    let secrets = load_lab_provider_secrets(Some(&secrets_path)).unwrap();
    let judge_ep = secrets.model_endpoint.as_deref().unwrap_or(&endpoint_url);
    let judge_prov =
        OpenAiModelProvider::new(OpenAiModelConfig::new(judge_ep, secrets.model_api_key)).unwrap();

    // Seed verified CAPA fixture (2 episodes in test.json so CostGuard can be tested cleanly).
    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();
    let capa_json = serde_json::json!([
        {
            "meeting_id": "ami_synth_01",
            "split": "test",
            "absent_role": "Industrial Designer",
            "briefing": "Track enclosure material changes.",
            "transcript": [
                {"speaker": "PM", "content": "Switch chassis to matte anodized aluminum."},
                {"speaker": "ID_delegate", "content": "Any updates?"}
            ],
            "episodes": [
                {
                    "episode_id": "ep1",
                    "cue_turn": 1,
                    "anchor_turn": 1,
                    "window_k": 3,
                    "target_proposition": "Chassis enclosure changed to matte anodized aluminum."
                },
                {
                    "episode_id": "ep2",
                    "cue_turn": 1,
                    "anchor_turn": 1,
                    "window_k": 3,
                    "target_proposition": "Finish is matte anodized aluminum."
                }
            ]
        }
    ])
    .to_string();
    {
        // The real `capa` release ships transcripts only (manifest marks it not_mappable); this
        // generic live-execution test seeds the legacy synthetic episode fixture, so it restores
        // the behavioral lane for the fixture manifest only.
        let capa = manifest.get_mut("capa").unwrap();
        capa.core_applicability = CoreApplicability::CoreBehavioral;
        seed_verified_dataset_file(
            capa,
            &datasets_dir,
            "data/ami/test.json",
            capa_json.as_bytes(),
        );
    }
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    let probe_provider = {
        let mut p = ScriptedModelProvider::new();
        let reply_json = serde_json::json!({
            "acts": [{
                "act": "act",
                "capability_id": "a.speak",
                "places": ["meeting:ami_synth_01"],
                "people": ["lab_proctor"],
                "payload": b"Matte anodized aluminum chassis.".to_vec(),
                "motivation": 5,
                "confidence_0_20": 19,
                "used_ids": []
            }]
        })
        .to_string();
        p.set_fallback(ScriptedReply::text(reply_json));
        p
    };

    struct TestAddressedCapaConverter;
    impl Converter for TestAddressedCapaConverter {
        fn dataset_id(&self) -> &str {
            "capa"
        }
        fn converter_version(&self) -> &str {
            "1.0.0"
        }
        fn convert(
            &self,
            dataset_dir: &Path,
            entry: &DatasetManifestEntry,
        ) -> Result<ConvertedDataset, LabError> {
            let mut conv = monoagens_lab::datasets::CapaConverter.convert(dataset_dir, entry)?;
            let soul = ActorId::new("soul");
            for item in &mut conv.items {
                item.history_mode = HistoryMode::StaticSilent;
                for probe in &mut item.probes {
                    probe.percept.addressees = vec![soul.clone()];
                }
            }
            Ok(conv)
        }
    }

    let make_capa_handlers = || {
        let mut h = DatasetHandlerRegistry::default();
        h.register_behavioral(
            Box::new(TestAddressedCapaConverter),
            Box::new(monoagens_lab::datasets::CapaScorer),
        );
        h
    };

    // 1. Live CAPA with non-official judge under RenameMetric succeeds, charges CostGuard, and renames metric.
    let opts = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path.clone()),
        only_ids: Some(vec!["capa".to_owned()]),
        out_dir: Some(out_dir.clone()),
        live: true,
        max_cost_usd: Some(5.0),
        probe_model_name: "mock-live-model".to_owned(),
        probe_provider: Some(&probe_provider as &dyn ModelProvider),
        judge_provider: Some(&judge_prov as &dyn ModelProvider),
        judge_config: Some(JudgeConfig {
            endpoint_url: endpoint_url.clone(),
            model: "custom-judge-v1".to_owned(),
            prompt_version: "capa-judge-v1".to_owned(),
            substitution_policy: JudgeSubstitutionPolicy::RenameMetric,
        }),
        handlers: make_capa_handlers(),
        ..BenchmarkExecutionOptions::default()
    };
    let rep = run_benchmark_suite_with_options(
        "prospective",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts,
    )
    .expect("live CAPA with Rust OpenAiModelProvider must succeed");
    let row = rep
        .scorecard
        .iter()
        .find(|r| r.dataset_id == "capa")
        .unwrap();
    assert!(auth_ok.load(Ordering::SeqCst), "row={row:?}");
    assert_eq!(hits.load(Ordering::SeqCst), 2, "row={row:?}");
    assert_eq!(
        row.metrics
            .get("credited_recovery_non_official_judge")
            .copied()
            .flatten(),
        Some(1.0)
    );
    assert!(row.cost_usd > 0.0);
    let prov = rep.run_records[0].dataset_provenance.as_ref().unwrap();
    assert_eq!(prov.judge_model.as_deref(), Some("custom-judge-v1"));
    assert_eq!(prov.judge_prompt_version.as_deref(), Some("capa-judge-v1"));
    assert!(
        rep.run_records[0]
            .cost_by_purpose
            .iter()
            .any(|c| c.purpose == monoagens_core::ModelPurpose::Classify && c.calls == 2)
    );

    // 2. CostGuard halts judge call cleanly when budget cap is reached mid-run.
    // Preflight for 2 probes + 2 judges = $0.06144; with cap = $0.070, preflight passes,
    // 1st judge call reports 8,000 tokens ($0.080 > $0.070), and CostGuard halts the 2nd judge call before HTTP!
    hits.store(0, Ordering::SeqCst);
    tokens_per_call.store(4_000, Ordering::SeqCst);
    let opts_budget_halt = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path),
        only_ids: Some(vec!["capa".to_owned()]),
        live: true,
        max_cost_usd: Some(0.070),
        probe_model_name: "mock-live-model".to_owned(),
        probe_provider: Some(&probe_provider as &dyn ModelProvider),
        judge_provider: Some(&judge_prov as &dyn ModelProvider),
        judge_config: Some(JudgeConfig {
            endpoint_url,
            model: "gpt-4o".to_owned(),
            prompt_version: "capa-judge-v1".to_owned(),
            substitution_policy: JudgeSubstitutionPolicy::StrictFail,
        }),
        handlers: make_capa_handlers(),
        ..BenchmarkExecutionOptions::default()
    };
    let err_halt = run_benchmark_suite_with_options(
        "prospective",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts_budget_halt,
    )
    .expect_err("CostGuard must halt 2nd judge call once USD cap is exceeded");
    assert!(err_halt.to_string().contains("CostGuard"), "err={err_halt}");
    assert_eq!(hits.load(Ordering::SeqCst), 1);

    stop.store(true, Ordering::Relaxed);
    let _ = std::net::TcpStream::connect(addr);
    let _ = handle.join();
}

#[test]
fn task_10_2_and_10_3_and_10_5_live_options_embedder_and_report_parity() {
    use monoagens_lab::datasets::{ReportOptions, generate_benchmark_report};
    use monoagens_reason::model::ReasoningEffort;
    use monoagens_reason::secrets::SecretString;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    let tmp = TempDir::new().unwrap();
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    let out_dir = tmp.path().join("run_out");
    fs::create_dir_all(&datasets_dir).unwrap();

    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();
    let capa_json = serde_json::json!([
        {
            "meeting_id": "ami_synth_10",
            "split": "test",
            "absent_role": "Industrial Designer",
            "briefing": "Track enclosure material changes.",
            "transcript": [
                {"speaker": "PM", "content": "Switch chassis to brushed titanium."}
            ],
            "episodes": [
                {
                    "episode_id": "ep_ok",
                    "cue_turn": 0,
                    "anchor_turn": 0,
                    "window_k": 2,
                    "target_proposition": "Chassis enclosure changed to brushed titanium."
                }
            ]
        }
    ])
    .to_string();
    {
        // The real `capa` release ships transcripts only (manifest marks it not_mappable); this
        // generic live-execution test seeds the legacy synthetic episode fixture, so it restores
        // the behavioral lane for the fixture manifest only.
        let capa = manifest.get_mut("capa").unwrap();
        capa.core_applicability = CoreApplicability::CoreBehavioral;
        seed_verified_dataset_file(
            capa,
            &datasets_dir,
            "data/ami/test.json",
            capa_json.as_bytes(),
        );
    }
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    // Task 10.5: a category run refuses without semantic embedder when require_semantic_embedder=true.
    let err_no_emb = run_benchmark_suite_with_options(
        "prospective",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &BenchmarkExecutionOptions {
            manifest_path: Some(manifest_path.clone()),
            only_ids: Some(vec!["capa".to_owned()]),
            require_semantic_embedder: true,
            ..BenchmarkExecutionOptions::default()
        },
    )
    .expect_err("standard run without semantic embedder must refuse");
    assert!(
        err_no_emb.to_string().contains("embeddinggemma-2"),
        "err={err_no_emb}"
    );

    // Mock loopback OpenAI-compatible server capturing request body + headers for soul & judge.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let endpoint_url = format!("http://{addr}/v1/chat/completions");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_srv = Arc::clone(&stop);
    let captured: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
    let cap_srv = Arc::clone(&captured);
    let secret_session = "ses_secret_test_session_value_999";

    let handle = std::thread::spawn(move || {
        while !stop_srv.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            if stop_srv.load(Ordering::Relaxed) {
                break;
            }
            let mut buf = vec![0u8; 32768];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req_text = String::from_utf8_lossy(&buf[..n]).into_owned();
            let (hdr_part, body_part) = req_text.split_once("\r\n\r\n").unwrap_or((&req_text, ""));
            if let Ok(mut g) = cap_srv.lock() {
                g.push((hdr_part.to_owned(), body_part.to_owned()));
            }
            // Soul effort is `medium`, judge effort `high` in this test; word-based detection
            // would misfire on rendered Core prompts.
            let is_judge = body_part.contains(r#""reasoning_effort":"high""#);
            let content = if is_judge {
                r#"{"label":"no","score":0.0,"rationale":"mismatch in mock test"}"#.to_owned()
            } else {
                serde_json::json!({
                    "acts": [{
                        "act": "act",
                        "capability_id": "a.speak",
                        "places": ["meeting:ami_synth_10"],
                        "people": ["lab_proctor"],
                        "payload": b"Wrong material answer.".to_vec(),
                        "motivation": 5,
                        "confidence_0_20": 19,
                        "used_ids": []
                    }]
                })
                .to_string()
            };
            let body = serde_json::json!({
                "model": "mock-ds",
                "choices": [{"message": {"content": content}}],
                "usage": {"prompt_tokens": 120, "completion_tokens": 30, "total_tokens": 150}
            })
            .to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });

    struct TestAddressedCapaConverter;
    impl Converter for TestAddressedCapaConverter {
        fn dataset_id(&self) -> &str {
            "capa"
        }
        fn converter_version(&self) -> &str {
            "1.0.0"
        }
        fn convert(
            &self,
            dataset_dir: &Path,
            entry: &DatasetManifestEntry,
        ) -> Result<ConvertedDataset, LabError> {
            let mut conv = monoagens_lab::datasets::CapaConverter.convert(dataset_dir, entry)?;
            let soul = ActorId::new("soul");
            for item in &mut conv.items {
                item.history_mode = HistoryMode::StaticSilent;
                for probe in &mut item.probes {
                    probe.percept.addressees = vec![soul.clone()];
                }
            }
            Ok(conv)
        }
    }
    let mut handlers = DatasetHandlerRegistry::default();
    handlers.register_behavioral(
        Box::new(TestAddressedCapaConverter),
        Box::new(monoagens_lab::datasets::CapaScorer),
    );

    let mut soul_cfg = OpenAiModelConfig::new(&endpoint_url, Some(SecretString::new("k-soul")));
    soul_cfg.reasoning_effort = Some(ReasoningEffort::Medium);
    soul_cfg.extra_headers = vec![(
        "x-opencode-session".to_owned(),
        SecretString::new(secret_session),
    )];
    soul_cfg.user_agent = Some("monoagens-lab/0.1.0".to_owned());
    let soul_prov = OpenAiModelProvider::new(soul_cfg).unwrap();

    let mut judge_cfg = OpenAiModelConfig::new(&endpoint_url, Some(SecretString::new("k-judge")));
    judge_cfg.reasoning_effort = Some(ReasoningEffort::High);
    judge_cfg.extra_headers = vec![(
        "x-opencode-session".to_owned(),
        SecretString::new(secret_session),
    )];
    judge_cfg.user_agent = Some("monoagens-lab/0.1.0".to_owned());
    let judge_prov = OpenAiModelProvider::new(judge_cfg).unwrap();

    let mut scripted_emb = ScriptedEmbedder::new("egemma2-256", 256);
    scripted_emb.set_fallback(Ok(vec![1.0; 256]));

    let opts = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path.clone()),
        only_ids: Some(vec!["capa".to_owned()]),
        out_dir: Some(out_dir.clone()),
        live: true,
        max_cost_usd: Some(5.0),
        allow_test_embedders: true,
        require_semantic_embedder: true,
        probe_model_name: "deepseek-v4.1-flash".to_owned(),
        probe_provider: Some(&soul_prov as &dyn ModelProvider),
        judge_provider: Some(&judge_prov as &dyn ModelProvider),
        judge_config: Some(JudgeConfig {
            endpoint_url,
            model: "deepseek-v4.1-flash".to_owned(),
            prompt_version: "capa-judge-v1".to_owned(),
            substitution_policy: JudgeSubstitutionPolicy::RenameMetric,
        }),
        embedder: Some(&scripted_emb as &dyn Embedder),
        reasoning_effort: Some("medium".to_owned()),
        judge_reasoning_effort: Some("high".to_owned()),
        extra_header_names: vec!["x-opencode-session".to_owned()],
        provider_endpoint_host: Some(format!("http://{addr}")),
        embedder_backend: Some("Scripted256d".to_owned()),
        embedder_revision: Some("test-rev".to_owned()),
        handlers,
        ..BenchmarkExecutionOptions::default()
    };

    let rep = run_benchmark_suite_with_options(
        "prospective",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts,
    )
    .expect("live run with provider options and 256d embedder must succeed");

    stop.store(true, Ordering::Relaxed);
    let _ = std::net::TcpStream::connect(addr);
    let _ = handle.join();

    // 10.2 checks: HTTP requests carried reasoning_effort + extra header; RunRecord holds names only (no secret value).
    let reqs = captured.lock().unwrap();
    assert!(
        reqs.len() >= 2,
        "expected soul + judge HTTP calls, got {}",
        reqs.len()
    );
    assert!(
        reqs.iter()
            .all(|(h, _)| h.to_ascii_lowercase().contains("x-opencode-session:")),
        "every HTTP call must carry an x-opencode-session header"
    );
    // Soul calls carry the stable per-run-and-item session value; the configured header value
    // still applies to judge calls built outside the Core loop.
    let soul_sessions: Vec<&str> = reqs
        .iter()
        .filter(|(_, b)| b.contains(r#""reasoning_effort":"medium""#))
        .filter_map(|(h, _)| {
            h.lines().find_map(|line| {
                line.strip_prefix("x-opencode-session: ")
                    .or_else(|| line.strip_prefix("X-OpenCode-Session: "))
            })
        })
        .collect();
    assert!(!soul_sessions.is_empty(), "no soul calls captured");
    let first_session = soul_sessions[0].trim();
    assert!(
        first_session.starts_with("ses_"),
        "soul session must be generated per item, got {first_session}"
    );
    assert!(
        soul_sessions.iter().all(|s| s.trim() == first_session),
        "session value must be stable per run and item: {soul_sessions:?}"
    );
    assert!(
        reqs.iter().any(|(h, _)| h
            .to_ascii_lowercase()
            .contains(&format!("x-opencode-session: {secret_session}"))),
        "judge calls keep the configured extra header"
    );
    assert!(
        reqs.iter()
            .any(|(_, b)| b.contains(r#""reasoning_effort":"medium""#)),
        "soul call must carry reasoning_effort medium"
    );
    assert!(
        reqs.iter()
            .any(|(_, b)| b.contains(r#""reasoning_effort":"high""#)),
        "judge call must carry reasoning_effort high"
    );

    let rr_json = fs::read_to_string(out_dir.join("capa__seed-1.json")).unwrap();
    assert!(
        !rr_json.contains(secret_session),
        "RunRecord must never leak extra header value"
    );
    let rr: RunRecord = serde_json::from_str(&rr_json).unwrap();
    assert_eq!(rr.embedder_id, "egemma2-256");
    let prov = rr.dataset_provenance.as_ref().unwrap();
    assert_eq!(prov.reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(prov.judge_reasoning_effort.as_deref(), Some("high"));
    assert_eq!(
        prov.extra_header_names,
        vec!["x-opencode-session".to_owned()]
    );

    // 10.5 check: transcript JSONL records 256d vectors on percepts and probes.
    let tx_path = out_dir.join("transcripts/capa/ami_synth_10_ep_ok.jsonl");
    assert!(
        tx_path.is_file(),
        "transcript missing at {}",
        tx_path.display()
    );
    let tx_raw = fs::read_to_string(&tx_path).unwrap();
    assert!(tx_raw.contains(r#""embedding_dim":256"#), "tx={tx_raw}");
    assert!(tx_raw.contains("egemma2-256:256:t_d"), "tx={tx_raw}");

    // 10.3 check: report generator produces Markdown, HTML, CSVs, charts, and failure .md matching RunRecord.
    let rep_summary = generate_benchmark_report(&ReportOptions {
        run_dir: out_dir.clone(),
        out_dir: None,
        manifest_path: Some(manifest_path),
        extra_notes: vec!["unit test note".to_owned()],
        skip_python_charts: true,
        suite: None,
        label: None,
        appendix_markdown: None,
    })
    .expect("generate report");

    let report_dir = PathBuf::from(&rep_summary.report_dir);
    assert!(report_dir.join("REPORT.md").is_file());
    assert!(report_dir.join("index.html").is_file());
    assert!(report_dir.join("tables/overall.csv").is_file());
    assert!(report_dir.join("tables/status.csv").is_file());
    assert!(report_dir.join("tables/cost.csv").is_file());
    assert!(report_dir.join("tables/prospective.csv").is_file());
    assert!(!rep_summary.chart_files.is_empty());
    assert!(!rep_summary.failure_markdown_files.is_empty());

    let overall_csv = fs::read_to_string(report_dir.join("tables/overall.csv")).unwrap();
    let rec_metric_val = prov.metric_scores[0].value.unwrap();
    assert!(
        overall_csv.contains(&format!("{rec_metric_val:.6}")),
        "overall.csv must match RunRecord metric value {rec_metric_val:.6}, got:\n{overall_csv}"
    );
    let md = fs::read_to_string(report_dir.join("REPORT.md")).unwrap();
    assert!(md.contains("deepseek-v4.1-flash"));
    assert!(md.contains("x-opencode-session"));
    assert!(!md.contains(secret_session));
    let _ = rep;
}

/// Task 11.2: a live run that deviates from full MonoAgens (scripted provider/switch off) is
/// recorded `invalid` and its scores are excluded from the regenerated report.
#[test]
fn live_deviation_is_recorded_invalid_and_excluded_from_reports() {
    use monoagens_lab::datasets::{
        ReportOptions, derive_monoagens_mode, generate_benchmark_report,
    };
    use monoagens_lab::record::{MONOAGENS_MODE_INVALID, MONOAGENS_MODE_OFFLINE};

    let tmp = TempDir::new().unwrap();
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    let out_dir = tmp.path().join("memory");
    fs::create_dir_all(&datasets_dir).unwrap();

    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();
    seed_verified_dataset_file(
        manifest.get_mut("longmemeval").unwrap(),
        &datasets_dir,
        "longmemeval_s_cleaned.json",
        b"lme",
    );
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    let mut handlers = DatasetHandlerRegistry::default();
    handlers.register_behavioral(
        Box::new(StubBehavioralConverter {
            id: "longmemeval".to_owned(),
            version: "1.0.0".to_owned(),
        }),
        Box::new(StubScorer {
            id: "longmemeval".to_owned(),
            requires_judge: false,
            metric_name: "overall_acc".to_owned(),
        }),
    );

    // No provider is supplied, so the runner falls back to `ScriptedModelProvider` even though
    // `live` is requested: that deviation must invalidate the record, never produce a score.
    let opts = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path.clone()),
        only_ids: Some(vec!["longmemeval".to_owned()]),
        out_dir: Some(out_dir.clone()),
        live: true,
        max_cost_usd: Some(5.0),
        handlers,
        ..BenchmarkExecutionOptions::default()
    };
    let rep = run_benchmark_suite_with_options(
        "memory",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &opts,
    )
    .expect("deviating live run still records an invalid record");

    let rec = &rep.run_records[0];
    assert_eq!(rec.monoagens_mode.as_deref(), Some(MONOAGENS_MODE_INVALID));
    assert_eq!(rec.history_reasoning_mode.as_deref(), Some("live"));
    assert!(!rec.is_full_monoagens());
    let row = rep
        .scorecard
        .iter()
        .find(|r| r.dataset_id == "longmemeval")
        .unwrap();
    assert_eq!(row.status, ScorecardStatus::NotScored);
    assert_eq!(row.reason.as_deref(), Some("invalid: not full MonoAgens"));

    // A switch off is always invalid; a non-live run is explicitly offline, never invalid.
    let mut switches_off = Switches::all_enabled();
    switches_off.credence = false;
    assert_eq!(
        derive_monoagens_mode(true, true, &switches_off, Some("live"), Some("core_render")),
        MONOAGENS_MODE_INVALID
    );
    assert_eq!(
        derive_monoagens_mode(
            true,
            false,
            &Switches::all_enabled(),
            Some("live"),
            Some("core_render")
        ),
        MONOAGENS_MODE_INVALID
    );
    assert_eq!(
        derive_monoagens_mode(
            false,
            false,
            &Switches::all_enabled(),
            Some("scripted_silent"),
            Some("core_render")
        ),
        MONOAGENS_MODE_OFFLINE
    );

    let summary = generate_benchmark_report(&ReportOptions {
        run_dir: out_dir.clone(),
        out_dir: None,
        manifest_path: Some(manifest_path),
        extra_notes: Vec::new(),
        skip_python_charts: true,
        suite: Some("memory".to_owned()),
        label: None,
        appendix_markdown: None,
    })
    .expect("regenerate report");
    let report_dir = PathBuf::from(&summary.report_dir);
    let md = fs::read_to_string(report_dir.join("REPORT.md")).unwrap();
    assert!(
        md.contains("invalid: not full MonoAgens"),
        "invalid banner missing from report:\n{md}"
    );
    assert!(
        !md.contains("| overall_acc | 1.0000"),
        "invalid score leaked"
    );
    let status_csv = fs::read_to_string(report_dir.join("tables/status.csv")).unwrap();
    assert!(
        status_csv.contains("invalid: not full MonoAgens"),
        "status table must name the invalid record: {status_csv}"
    );
}

/// Task 11.3: `--suite memory` runs only memory datasets and its own report holds no other
/// category.
#[test]
fn cli_memory_suite_runs_only_memory_and_report_has_no_other_category() {
    let tmp = TempDir::new().unwrap();
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    let out_dir = tmp.path().join("memory");
    fs::create_dir_all(&datasets_dir).unwrap();

    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();
    seed_verified_dataset_file(
        manifest.get_mut("locomo").unwrap(),
        &datasets_dir,
        "data/locomo10.json",
        br#"[{"sample_id":"s0","conversation":{"speaker_a":"alice","speaker_b":"bob","session_1":[{"speaker":"alice","dia_id":"D1:1","text":"hi"}]},"qa":[{"question":"Who greeted?","answer":"alice","evidence":["D1:1"],"category":1}]}]"#,
    );
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    let out = Command::new(lab_bin())
        .args([
            "benchmark",
            "--suite",
            "memory",
            "--manifest",
            manifest_path.to_str().unwrap(),
            "--datasets-dir",
            datasets_dir.to_str().unwrap(),
            "--only",
            "locomo",
            "--out-dir",
            out_dir.to_str().unwrap(),
            "--allow-no-embedder",
            "--json",
        ])
        .output()
        .expect("run benchmark --suite memory");
    assert!(
        out.status.success(),
        "benchmark --suite memory failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["suite"], "memory");
    let specs = report["dataset_specs"].as_array().unwrap();
    assert!(!specs.is_empty());
    assert!(
        specs.iter().all(|s| s["domain"].as_str() == Some("memory")),
        "memory suite leaked another category: {specs:?}"
    );
    let rows = report["scorecard"].as_array().unwrap();
    assert!(
        rows.iter().all(|r| r["domain"].as_str() == Some("memory")),
        "scorecard leaked another category: {rows:?}"
    );
    assert!(rows.len() <= 5, "memory suite rows: {rows:?}");

    let md = fs::read_to_string(out_dir.join("report/REPORT.md")).unwrap();
    assert!(md.contains("### Memory"), "memory section missing:\n{md}");
    for other in [
        "### Prospective Memory & Delegation",
        "### Proactivity & Timing",
        "### Group Chat",
        "### Theory of Mind",
        "### Indonesian Probes",
        "### Security",
    ] {
        assert!(
            !md.contains(other),
            "category report contains {other}:\n{md}"
        );
    }
}

/// Task 11.5: a regenerated report marks records without `monoagens_mode` (the 2026-11-18 pilot
/// shape) invalid and excludes their scores.
#[test]
fn report_marks_records_without_monoagens_mode_invalid() {
    use monoagens_lab::datasets::{ReportOptions, generate_benchmark_report};

    let tmp = TempDir::new().unwrap();
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let datasets_dir = tmp.path().join("datasets");
    let out_dir = tmp.path().join("memory");
    fs::create_dir_all(&datasets_dir).unwrap();

    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();
    seed_verified_dataset_file(
        manifest.get_mut("longmemeval").unwrap(),
        &datasets_dir,
        "longmemeval_s_cleaned.json",
        b"lme",
    );
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    let mut handlers = DatasetHandlerRegistry::default();
    handlers.register_behavioral(
        Box::new(StubBehavioralConverter {
            id: "longmemeval".to_owned(),
            version: "1.0.0".to_owned(),
        }),
        Box::new(StubScorer {
            id: "longmemeval".to_owned(),
            requires_judge: false,
            metric_name: "overall_acc".to_owned(),
        }),
    );
    run_benchmark_suite_with_options(
        "memory",
        &datasets_dir,
        None,
        false,
        &Prior::default(),
        &BenchmarkExecutionOptions {
            manifest_path: Some(manifest_path.clone()),
            only_ids: Some(vec!["longmemeval".to_owned()]),
            out_dir: Some(out_dir.clone()),
            handlers,
            ..BenchmarkExecutionOptions::default()
        },
    )
    .expect("offline memory run");

    // Recreate the legacy pilot record shape: `monoagens_mode` absent.
    let rec_path = out_dir.join("longmemeval__seed-1.json");
    let raw = fs::read_to_string(&rec_path).unwrap();
    let mut value: Value = serde_json::from_str(&raw).unwrap();
    assert!(value.get("monoagens_mode").is_some());
    value.as_object_mut().unwrap().remove("monoagens_mode");
    fs::write(&rec_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

    let summary = generate_benchmark_report(&ReportOptions {
        run_dir: out_dir.clone(),
        out_dir: None,
        manifest_path: Some(manifest_path),
        extra_notes: Vec::new(),
        skip_python_charts: true,
        suite: Some("memory".to_owned()),
        label: Some("PILOT".to_owned()),
        appendix_markdown: None,
    })
    .expect("regenerate legacy report");
    let report_dir = PathBuf::from(&summary.report_dir);
    let md = fs::read_to_string(report_dir.join("REPORT.md")).unwrap();
    assert!(
        md.contains("invalid: not full MonoAgens"),
        "invalid banner missing:\n{md}"
    );
    assert!(
        md.contains("PILOT; invalid: not full MonoAgens"),
        "banner must preserve the requested label:\n{md}"
    );
    let status_csv = fs::read_to_string(report_dir.join("tables/status.csv")).unwrap();
    assert!(
        status_csv.contains("invalid: not full MonoAgens"),
        "status.csv must mark the legacy record invalid: {status_csv}"
    );
}

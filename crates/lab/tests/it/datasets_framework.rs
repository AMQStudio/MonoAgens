use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_core::{Prior, Switches};
use monoagens_lab::datasets::{
    ChecksumsRecorded, ContaminationRisk, ConvertedDataset, Converter, CoreApplicability,
    DatasetAvailability, DatasetLane, DatasetLicense, DatasetManifestEntry, EmbedderProbe,
    EmbedderProbeOutcome, EmbedderReadiness, ExternalItem, ExternalPrediction, ExternalProbe,
    GoldBook, GoldEntry, GoldEvidenceRef, HistoryMode, JudgeExecutionConfig, LicenseStatus,
    ManifestEntryKind, SamplingRule, Scorer, ScorerMetric, ScorerReport,
    assert_visiblescope_gold_evidence, cluster_bootstrap_ci, execute_external_item,
    load_or_convert_cached, make_text_percept, sample_items_stratified, validate_semantic_embedder,
};
use monoagens_lab::{
    BenchmarkDomain, CostGuard, LabError, RunRecord, Scenario, ScenarioAnnotations, ScenarioSet,
    ScriptedEvent, ScriptedEventKind, calibrate_from_run_records, calibrate_from_scenarios,
    ensure_calibration_set_allowed, run_scenario,
};
use monoagens_protocol::{ActorId, Audience, PlaceId, VisibleScope};
use monoagens_reason::embed::{
    Embedder, EmbeddingGemma2Config, EmbeddingGemma2Embedder, HashEmbedder, NoneEmbedder,
    ScriptedEmbedder,
};
use monoagens_reason::model::{ScriptedModelProvider, ScriptedReply};
use serde::Deserialize;
use tempfile::TempDir;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/datasets")
}

fn synthetic_manifest_entry(converter_version: &str) -> DatasetManifestEntry {
    DatasetManifestEntry {
        id: "reference_synthetic".to_owned(),
        kind: ManifestEntryKind::Dataset,
        domain: BenchmarkDomain::Memory,
        relative_dir: "reference_synthetic".to_owned(),
        sources: Vec::new(),
        license: DatasetLicense {
            spdx: "MIT".to_owned(),
            terms: "Synthetic test fixture".to_owned(),
            status: LicenseStatus::Cleared,
        },
        availability: DatasetAvailability::Available,
        core_applicability: CoreApplicability::CoreBehavioral,
        lane: DatasetLane::CoreBehavioral,
        contamination_risk: ContaminationRisk::VeryLow,
        contamination_note: "Synthetic test fixture".to_owned(),
        reason: "Group 3 reference converter test".to_owned(),
        split: None,
        files: Vec::new(),
        checksums_recorded: ChecksumsRecorded::Na,
        checksums_recorded_date: None,
        converter_version: converter_version.to_owned(),
        scorer_version: "1.0.0".to_owned(),
    }
}

#[derive(Deserialize)]
struct RawRefFixture {
    dataset_id: String,
    conversations: Vec<RawRefConv>,
}

#[derive(Deserialize)]
struct RawRefConv {
    conv_id: String,
    turns: Vec<RawRefTurn>,
    probes: Vec<RawRefProbe>,
}

#[derive(Deserialize)]
struct RawRefTurn {
    actor: String,
    audience: Vec<String>,
    text: String,
}

#[derive(Deserialize)]
struct RawRefProbe {
    probe_id: String,
    asker: String,
    question: String,
    answer: String,
    evidence_turns: Vec<usize>,
    #[serde(default)]
    privacy_conflict: bool,
    #[serde(default)]
    gold_secret_tag: String,
    #[serde(default)]
    trace: String,
}

struct ReferenceSyntheticConverter {
    version: String,
}

impl Converter for ReferenceSyntheticConverter {
    fn dataset_id(&self) -> &str {
        "reference_synthetic"
    }

    fn converter_version(&self) -> &str {
        &self.version
    }

    fn convert(
        &self,
        dataset_dir: &Path,
        manifest_entry: &DatasetManifestEntry,
    ) -> Result<ConvertedDataset, LabError> {
        let raw = fs::read_to_string(dataset_dir.join("reference_synthetic.json"))?;
        let parsed: RawRefFixture =
            serde_json::from_str(&raw).map_err(|e| LabError::Serialization(e.to_string()))?;
        let soul = ActorId::new("soul");

        let mut items = Vec::new();
        let mut gold_entries = Vec::new();

        for conv in parsed.conversations {
            let place = PlaceId::new(format!("ref_{}", conv.conv_id));
            let mut history = Vec::new();
            let mut turn_audiences = Vec::new();
            for (t_idx, turn) in conv.turns.iter().enumerate() {
                let aud_actors: Vec<ActorId> = turn
                    .audience
                    .iter()
                    .map(|s| ActorId::new(s.clone()))
                    .collect();
                let aud = Audience::new(aud_actors);
                turn_audiences.push(aud.clone());
                history.push(make_text_percept(
                    &turn.actor,
                    place.as_str(),
                    aud.members.iter().cloned(),
                    1_000 + (t_idx as u64) * 1_000,
                    &turn.text,
                ));
            }

            let mut probes = Vec::new();
            for (p_idx, pr) in conv.probes.iter().enumerate() {
                let asker = ActorId::new(&pr.asker);
                let probe_aud = Audience::new([asker.clone(), soul.clone()]);
                let ts = 10_000 + (p_idx as u64) * 1_000;
                let percept = make_text_percept(
                    &pr.asker,
                    place.as_str(),
                    probe_aud.members.iter().cloned(),
                    ts,
                    &pr.question,
                );
                probes.push(ExternalProbe {
                    probe_id: pr.probe_id.clone(),
                    percept,
                    isolated_checkpoint: true,
                    probe_scope: VisibleScope::new(probe_aud, Some(place.clone())),
                    expected_capability: None,
                });

                let gold_evidence = pr
                    .evidence_turns
                    .iter()
                    .map(|&ti| GoldEvidenceRef {
                        turn_index: ti,
                        audience: turn_audiences[ti].clone(),
                        place: place.clone(),
                    })
                    .collect();

                let mut metadata = BTreeMap::new();
                metadata.insert("gold_secret_tag".to_owned(), pr.gold_secret_tag.clone());
                metadata.insert("trace".to_owned(), pr.trace.clone());

                gold_entries.push(GoldEntry {
                    item_id: conv.conv_id.clone(),
                    probe_id: pr.probe_id.clone(),
                    cluster_id: conv.conv_id.clone(),
                    gold_answer: pr.answer.clone(),
                    accepted_answers: vec![pr.answer.clone()],
                    gold_evidence,
                    privacy_conflict: pr.privacy_conflict,
                    category: None,
                    metadata,
                });
            }

            items.push(ExternalItem {
                item_id: conv.conv_id.clone(),
                cluster_id: conv.conv_id.clone(),
                strata_key: "default".to_owned(),
                history,
                checkpoint_group: Some(conv.conv_id.clone()),
                probes,
                history_mode: HistoryMode::StaticSilent,
                capabilities: Vec::new(),
            });
        }

        Ok(ConvertedDataset {
            dataset_id: parsed.dataset_id,
            pinned_revision: manifest_entry
                .sources
                .first()
                .map_or_else(|| "fixture".to_owned(), |s| s.revision.clone()),
            converter_version: self.version.clone(),
            items,
            gold_book: GoldBook {
                dataset_id: "reference_synthetic".to_owned(),
                entries: gold_entries,
            },
        })
    }
}

struct ReferenceSyntheticScorer;

impl Scorer for ReferenceSyntheticScorer {
    fn dataset_id(&self) -> &str {
        "reference_synthetic"
    }

    fn scorer_version(&self) -> &str {
        "1.0.0"
    }

    fn score(
        &self,
        predictions: &[ExternalPrediction],
        gold: &GoldBook,
        _judge: &JudgeExecutionConfig<'_>,
        seed: u64,
    ) -> Result<ScorerReport, LabError> {
        let mut non_conflict_obs = Vec::new();
        let mut conflict_refusals = Vec::new();

        for pred in predictions {
            let entry = gold.find(&pred.item_id, &pred.probe_id).ok_or_else(|| {
                LabError::Protocol(format!(
                    "missing gold entry for {}:{}",
                    pred.item_id, pred.probe_id
                ))
            })?;
            let resp = pred.response_text.as_deref().unwrap_or("");
            if entry.privacy_conflict {
                let refused = !pred.spoke || !resp.contains(&entry.gold_answer);
                conflict_refusals.push((pred.cluster_id.clone(), if refused { 1.0 } else { 0.0 }));
            } else {
                let hit = resp
                    .to_ascii_lowercase()
                    .contains(&entry.gold_answer.to_ascii_lowercase());
                non_conflict_obs.push((pred.cluster_id.clone(), if hit { 1.0 } else { 0.0 }));
            }
        }

        let acc_ci = cluster_bootstrap_ci(&non_conflict_obs, seed, 200);
        let ref_ci = cluster_bootstrap_ci(&conflict_refusals, seed, 200);

        let mut metrics = Vec::new();
        if let Some(ci) = acc_ci {
            metrics.push(ScorerMetric {
                name: "non_conflict_acc".to_owned(),
                value: Some(ci.point_estimate),
                ci95_low: Some(ci.ci95_low),
                ci95_high: Some(ci.ci95_high),
                n_clusters: Some(ci.n_clusters),
                not_scored_reason: None,
            });
        }
        if let Some(ci) = ref_ci {
            metrics.push(ScorerMetric {
                name: "privacy_conflict_refusal_rate".to_owned(),
                value: Some(ci.point_estimate),
                ci95_low: Some(ci.ci95_low),
                ci95_high: Some(ci.ci95_high),
                n_clusters: Some(ci.n_clusters),
                not_scored_reason: None,
            });
        }

        Ok(ScorerReport {
            dataset_id: self.dataset_id().to_owned(),
            scorer_version: self.scorer_version().to_owned(),
            metrics,
            judge_model: None,
            judge_prompt_version: None,
        })
    }
}

#[test]
fn calibration_rejects_external_inputs_with_error() {
    let err_set = ensure_calibration_set_allowed(ScenarioSet::External)
        .expect_err("calibration must reject ScenarioSet::External");
    assert!(err_set.to_string().contains("External"));

    // RunRecord tagged ScenarioSet::External is rejected by calibrate_from_run_records.
    let sc = Scenario {
        id: "ext_probe".to_owned(),
        name: "External probe".to_owned(),
        set: ScenarioSet::External,
        start_time_ms: 0,
        model_latency_ms: 1,
        model_id: "scripted".to_owned(),
        embedder_id: "none".to_owned(),
        bodies: Vec::new(),
        events: Vec::new(),
        scripted_model: Vec::new(),
        checks: Vec::new(),
        switches: None,
        cost_guard: None,
        prior: None,
        annotations: ScenarioAnnotations::default(),
    };
    let out = run_scenario(monoagens_core::MemStore::new(), Prior::default(), &sc, 1).unwrap();
    let recs: Vec<RunRecord> = vec![out.run_record];
    let err_rec =
        calibrate_from_run_records(&recs).expect_err("calibrate_from_run_records must fail");
    assert!(err_rec.to_string().contains("External"));

    let err_sc = calibrate_from_scenarios(&[sc], &Prior::default(), 1)
        .expect_err("calibrate_from_scenarios must fail on External");
    assert!(err_sc.to_string().contains("External"));

    // RecordCalibration inside an External scenario is also rejected.
    let mut bad_sc = Scenario {
        id: "ext_cal".to_owned(),
        name: "External calibration attempt".to_owned(),
        set: ScenarioSet::External,
        start_time_ms: 0,
        model_latency_ms: 1,
        model_id: "scripted".to_owned(),
        embedder_id: "none".to_owned(),
        bodies: Vec::new(),
        events: vec![ScriptedEvent {
            at_ms: 100,
            body_id: None,
            event: ScriptedEventKind::RecordCalibration {
                conf_0_20: 15,
                correct: true,
            },
        }],
        scripted_model: Vec::new(),
        checks: Vec::new(),
        switches: None,
        cost_guard: None,
        prior: None,
        annotations: ScenarioAnnotations::default(),
    };
    let err_op = run_scenario(
        monoagens_core::MemStore::new(),
        Prior::default(),
        &bad_sc,
        1,
    )
    .expect_err("RecordCalibration in External scenario must error");
    assert!(err_op.to_string().contains("External"));

    // Positive control: Development set succeeds.
    bad_sc.set = ScenarioSet::Development;
    let cal_ok = calibrate_from_scenarios(&[bad_sc], &Prior::default(), 1)
        .expect("development calibration must succeed");
    assert_eq!(cal_ok.scenario_count, 1);
}

#[test]
fn reference_converter_cache_isolation_visiblescope_and_scorer_parity() {
    let tmp = TempDir::new().expect("tempdir");
    let ds_dir = tmp.path().join("reference_synthetic");
    fs::create_dir_all(&ds_dir).unwrap();
    fs::copy(
        fixtures_dir().join("reference_synthetic.json"),
        ds_dir.join("reference_synthetic.json"),
    )
    .unwrap();

    // 1. Byte-identical conversion on two calls & cache rebuild on converter_version change.
    let c1 = ReferenceSyntheticConverter {
        version: "1.0.0".to_owned(),
    };
    let m1 = synthetic_manifest_entry("1.0.0");
    let (conv_a, hit_a) = load_or_convert_cached(&c1, &ds_dir, &m1).unwrap();
    assert!(!hit_a);
    let (conv_b, hit_b) = load_or_convert_cached(&c1, &ds_dir, &m1).unwrap();
    assert!(hit_b);
    let bytes_a = serde_json::to_vec(&conv_a).unwrap();
    let bytes_b = serde_json::to_vec(&conv_b).unwrap();
    assert_eq!(bytes_a, bytes_b, "conversion must be byte-identical");

    let c2 = ReferenceSyntheticConverter {
        version: "2.0.0".to_owned(),
    };
    let m2 = synthetic_manifest_entry("2.0.0");
    let (conv_c, hit_c) = load_or_convert_cached(&c2, &ds_dir, &m2).unwrap();
    assert!(!hit_c, "cache must rebuild when converter_version changes");
    assert_eq!(conv_c.converter_version, "2.0.0");

    // 2. Zero gold leakage in ExternalItems.
    let items_json = serde_json::to_string(&conv_a.items).unwrap();
    for forbidden in [
        "4821_gold",
        "gold_secret_tag",
        "_abs_gold_only",
        "_abs_unanswerable",
        "TRACE_HIDDEN",
        "9090",
        "blue drawer",
        "no information available",
    ] {
        assert!(
            !items_json.contains(forbidden),
            "ExternalItem leaked gold string '{forbidden}': {items_json}"
        );
    }

    // 3. Mandatory VisibleScope gold-evidence visibility check passes for non-conflict AND
    //    verifies cross-channel privacy_conflict items are blocked.
    assert_visiblescope_gold_evidence(&conv_a.items, &conv_a.gold_book)
        .expect("VisibleScope check must pass on synthetic fixture");

    // Negative check: mutating a non-conflict probe audience to exclude evidence fails loudly.
    let mut broken_items = conv_a.items.clone();
    broken_items[0].probes[0].probe_scope = VisibleScope::new(
        Audience::new([ActorId::new("stranger"), ActorId::new("soul")]),
        None,
    );
    broken_items[0].probes[0].percept.audience =
        broken_items[0].probes[0].probe_scope.audience.clone();
    assert_visiblescope_gold_evidence(&broken_items, &conv_a.gold_book)
        .expect_err("VisibleScope helper must reject non-conflict probe missing from evidence");

    // 4. Execute through Core's unmodified loop with Lab-side ScriptedModelProvider silent-think
    //    history loading, and score with cluster-bootstrap CI.
    let probe_provider = {
        let mut p = ScriptedModelProvider::new();
        p.set_fallback(ScriptedReply::text(
            r#"{"acts":[{"act":"think","thought":"answering probe","motivation":4,"confidence_0_20":18,"used_ids":[]},{"act":"act","capability_id":"a.speak","places":["ref_conv_alpha"],"people":["alice"],"payload":"4821","motivation":4,"confidence_0_20":18,"used_ids":[]}]}"#,
        ));
        p
    };
    let mut guard = CostGuard::default();
    let mut all_preds = Vec::new();
    for item in &conv_a.items {
        let exec = execute_external_item(
            item,
            &Prior::default(),
            &Switches::all_enabled(),
            7,
            &probe_provider,
            "scripted",
            false,
            &mut guard,
        )
        .expect("execute_external_item");
        assert_eq!(exec.history_reasoning_mode, "scripted_silent");
        all_preds.extend(exec.predictions);
    }
    assert_eq!(all_preds.len(), 4);

    let scorer = ReferenceSyntheticScorer;
    let report = scorer
        .score(
            &all_preds,
            &conv_a.gold_book,
            &JudgeExecutionConfig::default(),
            7,
        )
        .unwrap();
    let acc = report
        .metrics
        .iter()
        .find(|m| m.name == "non_conflict_acc")
        .unwrap();
    assert!(acc.value.is_some());
    assert!(acc.ci95_low.is_some() && acc.ci95_high.is_some());
    assert_eq!(acc.n_clusters, Some(2));
}

#[test]
fn embedder_probe_enforces_semantic_backend_and_not_scored_when_unconfigured() {
    // 1. Non-semantic backends MUST be rejected when allow_test_embedders == false.
    let hash_emb = HashEmbedder::new(256);
    let err_hash = validate_semantic_embedder(Some(&hash_emb), false)
        .expect_err("HashEmbedder must be rejected outside tests");
    assert!(err_hash.to_string().contains("hash"));

    let none_emb = NoneEmbedder;
    let err_none = validate_semantic_embedder(Some(&none_emb), false)
        .expect_err("NoneEmbedder must be rejected outside tests");
    assert!(err_none.to_string().contains("none"));

    let mut scripted_emb = ScriptedEmbedder::new("test-v1", 256);
    scripted_emb.set_fallback(Ok(vec![1.0; 256]));
    let err_scripted = validate_semantic_embedder(Some(&scripted_emb), false)
        .expect_err("ScriptedEmbedder must be rejected outside tests");
    assert!(err_scripted.to_string().contains("scripted"));

    // 2. Unconfigured EmbeddingGemma2 (no model_path) fails construction with MissingModelPath;
    //    passing None to validate_semantic_embedder yields Unconfigured("embedder_unconfigured").
    let gemma_res = EmbeddingGemma2Embedder::new(EmbeddingGemma2Config::local_http(
        "http://127.0.0.1:9",
        None,
        100,
    ));
    assert!(gemma_res.is_err());
    let unconfigured = gemma_res.ok();
    let readiness =
        validate_semantic_embedder(unconfigured.as_ref().map(|e| e as &dyn Embedder), false)
            .expect("missing local weights should report unconfigured rather than panic");
    assert_eq!(
        readiness,
        EmbedderReadiness::Unconfigured("embedder_unconfigured".to_owned())
    );

    // 3. Test probe implementing EmbedderProbe works with ScriptedEmbedder in fixture mode,
    //    and reports NotScored when local weights are absent.
    struct DummyRegisterProbe;
    impl EmbedderProbe for DummyRegisterProbe {
        fn dataset_id(&self) -> &str {
            "dummy_register"
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
                    dataset_id: self.dataset_id().to_owned(),
                    probe_version: self.probe_version().to_owned(),
                    reason,
                }),
                EmbedderReadiness::Ready => {
                    let emb = embedder.expect("ready implies Some");
                    Ok(EmbedderProbeOutcome::Scored {
                        dataset_id: self.dataset_id().to_owned(),
                        probe_version: self.probe_version().to_owned(),
                        embed_model_ver: emb.model_ver().unwrap_or("unknown").to_owned(),
                        dim_d: emb.dim_d().unwrap_or(0),
                        metrics: vec![ScorerMetric {
                            name: "twin_retrieval_r1_256d".to_owned(),
                            value: Some(1.0),
                            ci95_low: Some(1.0),
                            ci95_high: Some(1.0),
                            n_clusters: None,
                            not_scored_reason: None,
                        }],
                        item_count: 1,
                        chosen_item_ids: vec!["item_0".to_owned()],
                    })
                }
            }
        }
    }

    let probe = DummyRegisterProbe;
    let out_unconf = probe
        .run_probe(Path::new("."), None, false, None, 1)
        .unwrap();
    assert!(matches!(
        out_unconf,
        EmbedderProbeOutcome::NotScored { ref reason, .. } if reason == "embedder_unconfigured"
    ));

    let out_test = probe
        .run_probe(Path::new("."), Some(&scripted_emb), true, None, 1)
        .unwrap();
    assert!(matches!(
        out_test,
        EmbedderProbeOutcome::Scored { dim_d: 256, .. }
    ));
}

#[test]
fn seeded_stratified_sampling_is_deterministic_across_arms_and_clusters() {
    let mut items = Vec::new();
    for i in 0..20_usize {
        let stratum = if i < 12 { "stratum_a" } else { "stratum_b" };
        let cluster = format!("cluster_{}", i / 2);
        items.push(ExternalItem {
            item_id: format!("item_{i:02}"),
            cluster_id: cluster,
            strata_key: stratum.to_owned(),
            history: Vec::new(),
            checkpoint_group: None,
            probes: Vec::new(),
            history_mode: HistoryMode::StaticSilent,
            capabilities: Vec::new(),
        });
    }

    let rule_arm1 = SamplingRule {
        rule_name: "proportional_s10".to_owned(),
        seed: 2026,
        target_n: Some(10),
        sample_by_cluster: false,
    };
    let rule_arm2 = rule_arm1.clone();
    let r1 = sample_items_stratified(&items, &rule_arm1);
    let r2 = sample_items_stratified(&items, &rule_arm2);
    assert_eq!(r1.chosen_item_ids, r2.chosen_item_ids);
    assert_eq!(r1.chosen_item_ids.len(), 10);

    // Check proportional representation: 12/20 * 10 = 6 in stratum_a, 4 in stratum_b.
    let count_a = r1
        .chosen_item_ids
        .iter()
        .filter(|id| {
            let idx: usize = id.trim_start_matches("item_").parse().unwrap();
            idx < 12
        })
        .count();
    assert_eq!(count_a, 6);

    // Cluster sampling keeps clusters intact.
    let rule_cluster = SamplingRule {
        rule_name: "cluster_s4".to_owned(),
        seed: 99,
        target_n: Some(4),
        sample_by_cluster: true,
    };
    let rc = sample_items_stratified(&items, &rule_cluster);
    assert_eq!(rc.chosen_item_ids.len() % 2, 0);

    let obs: Vec<(String, f64)> = (0..10)
        .map(|i| (format!("c_{}", i / 2), (i as f64) / 10.0))
        .collect();
    let ci_1 = cluster_bootstrap_ci(&obs, 42, 500).unwrap();
    let ci_2 = cluster_bootstrap_ci(&obs, 42, 500).unwrap();
    assert_eq!(ci_1, ci_2);
    assert_eq!(ci_1.n_clusters, 5);
    assert!(ci_1.ci95_low <= ci_1.point_estimate && ci_1.point_estimate <= ci_1.ci95_high);
}

/// Regression: live probes must see the question text and turn a plain-text completion into a
/// dispatched reply action (Core's reasoning plan alone carries no payload text).
#[test]
fn live_probe_prompt_carries_question_and_plain_text_becomes_action() {
    use std::sync::Mutex;

    use monoagens_lab::reason_seam::model::{
        CallOutcome, ModelCallRecord, ModelCallSuccess, ModelOutput, ModelProvider, ModelRequest,
        TokenUsage,
    };

    struct RecordingProvider {
        seen: Mutex<Vec<ModelRequest>>,
        answer: String,
    }
    impl ModelProvider for RecordingProvider {
        fn provider_name(&self) -> &str {
            "mock-live-model"
        }
        fn invoke(
            &self,
            request: &ModelRequest,
        ) -> Result<ModelCallSuccess, monoagens_lab::reason_seam::model::ModelCallFailure> {
            self.seen
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(request.clone());
            let output = ModelOutput::ThoughtActs(
                monoagens_lab::reason_seam::parse_thought_output(&self.answer)
                    .expect("plain text falls back to a think act"),
            );
            Ok(ModelCallSuccess {
                record: ModelCallRecord {
                    request_id: request.request_id,
                    purpose: request.purpose,
                    provider: "mock-live-model".to_owned(),
                    model: request.model.clone(),
                    usage: Some(TokenUsage {
                        input_tokens: 12,
                        output_tokens: 3,
                        cached_tokens: None,
                        reasoning_tokens: Some(2),
                        total_tokens: 15,
                    }),
                    latency_ms: 7,
                    outcome: CallOutcome::Success,
                },
                output,
                raw_text: self.answer.clone(),
            })
        }
    }

    let soul = ActorId::new("soul");
    let user = ActorId::new("user");
    let proctor = ActorId::new("lab_proctor");
    let place = PlaceId::new("vault_room");
    let aud = Audience::new([user.clone(), soul.clone(), proctor.clone()]);
    let item = ExternalItem {
        item_id: "probe_item".to_owned(),
        cluster_id: "probe_item".to_owned(),
        strata_key: "test".to_owned(),
        history: vec![make_text_percept(
            "user",
            place.as_str(),
            aud.members.iter().cloned(),
            1_000,
            "The vault code is 4417.",
        )],
        checkpoint_group: None,
        probes: vec![ExternalProbe {
            probe_id: "q".to_owned(),
            percept: make_text_percept(
                "lab_proctor",
                place.as_str(),
                [proctor.clone(), soul.clone()],
                2_000,
                "What is the vault code?",
            ),
            isolated_checkpoint: true,
            probe_scope: VisibleScope::new(Audience::new([proctor, soul]), Some(place.clone())),
            expected_capability: None,
        }],
        history_mode: HistoryMode::StaticSilent,
        capabilities: Vec::new(),
    };
    let provider = RecordingProvider {
        seen: Mutex::new(Vec::new()),
        answer: "4417".to_owned(),
    };
    let mut guard = CostGuard::default();
    let exec = execute_external_item(
        &item,
        &Prior::default(),
        &Switches::all_enabled(),
        1,
        &provider,
        "mock-live-model",
        false,
        &mut guard,
    )
    .expect("execute");
    let seen = provider.seen.lock().unwrap();
    assert!(
        seen.iter().any(|r| {
            r.messages.iter().any(|m| {
                m.parts.iter().any(|p| match p {
                    monoagens_lab::reason_seam::model::MessagePart::Text(t) => {
                        t.contains("What is the vault code?")
                            && t.contains("The vault code is 4417.")
                    }
                    _ => false,
                })
            })
        }),
        "probe prompt must carry visible history and the question"
    );
    assert_eq!(exec.predictions.len(), 1);
    assert_eq!(exec.predictions[0].response_text.as_deref(), Some("4417"));
    assert!(exec.predictions[0].spoke);
}

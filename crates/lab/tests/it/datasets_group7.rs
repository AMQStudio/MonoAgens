//! Comprehensive Group 7 verification tests for all 20 active external dataset adapters,
//! `reason::EmbedderProbe` modules, `R-54` sampling/cluster-bootstrap, and `#[ignore]`d Python helpers.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_core::CoreConfig;
use monoagens_lab::benchmark::{BenchmarkSuiteReport, standard_benchmark_registry};
use monoagens_lab::datasets::{
    BenchmarkExecutionOptions, CapaConverter, CapaScorer, ChecksumsRecorded, ConvertedDataset,
    Converter, DatasetAvailability, DatasetLane, DatasetManifestEntry, DynTomConverter,
    DynTomScorer, EXTRACTION_MARKER_FILE, EmbedderProbe, EmbedderProbeOutcome, ExploreTomConverter,
    ExploreTomScorer, ExternalItem, ExternalPrediction, ExtractionMarker, FantomConverter,
    FantomScorer, GoldBook, GroupMemBenchConverter, GroupMemBenchScorer, HiTomConverter,
    HiTomScorer, IndoCollexProbe, JudgeExecutionConfig, LicenseStatus, LocomoConverter,
    LocomoScorer, LongMemEvalConverter, LongMemEvalScorer, ManifestEntryKind, ManifestFile,
    MemoryAgentBenchConverter, MemoryAgentBenchScorer, MolweniConverter, MolweniScorer,
    MultiLexNormProbe, OpenTomConverter, OpenTomScorer, PinnedSha256, PinnedSize, PitchConverter,
    PitchScorer, PmBenchConverter, PmBenchScorer, ProactiveBenchConverter, ProactiveBenchScorer,
    Scorer, ScorerReport, StaleConverter, StaleScorer, TomiConverter, TomiScorer,
    UbuntuIrcConverter, UbuntuIrcScorer, When2SpeakConverter, When2SpeakScorer,
    assert_visiblescope_gold_evidence, cluster_bootstrap_ci, default_sampling_rule,
    load_and_validate_manifest, run_optional_python_helper, sample_items_stratified,
    save_manifest_to_path,
};
use monoagens_lab::pareto::ScorecardStatus;
use monoagens_lab::reason_seam::embed::{
    EmbeddingGemma2Config, EmbeddingGemma2Embedder, HashEmbedder, NoneEmbedder, ScriptedEmbedder,
    TaskPrefix,
};
use monoagens_lab::reason_seam::model::{ModelProvider, ScriptedModelProvider, ScriptedReply};
use monoagens_lab::{run_benchmark_suite_with_options, sha256_hex};
use monoagens_protocol::{ActorId, VisibleScope};
use tempfile::TempDir;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture_dir(dataset_id: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/datasets")
        .join(dataset_id)
}

fn manifest_entry_for(id: &str) -> DatasetManifestEntry {
    let reg = standard_benchmark_registry();
    let m = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &reg,
    )
    .unwrap();
    m.get(id).unwrap().clone()
}

fn convert_twice_byte_identical(conv: &dyn Converter) -> ConvertedDataset {
    let dir = fixture_dir(conv.dataset_id());
    let entry = manifest_entry_for(conv.dataset_id());
    let first = conv
        .convert(&dir, &entry)
        .unwrap_or_else(|e| panic!("{} first convert failed: {e}", conv.dataset_id()));
    let second = conv
        .convert(&dir, &entry)
        .unwrap_or_else(|e| panic!("{} second convert failed: {e}", conv.dataset_id()));
    let b1 = serde_json::to_vec(&first).unwrap();
    let b2 = serde_json::to_vec(&second).unwrap();
    assert_eq!(
        b1,
        b2,
        "{} conversion must be byte-identical across runs",
        conv.dataset_id()
    );
    assert!(
        !first.items.is_empty(),
        "{} converted zero items",
        conv.dataset_id()
    );
    first
}

fn serialize_items_json(items: &[ExternalItem]) -> String {
    serde_json::to_string(items).unwrap()
}

fn gold_predictions_exact(conv: &ConvertedDataset) -> Vec<ExternalPrediction> {
    let mut out = Vec::new();
    for item in &conv.items {
        for probe in &item.probes {
            let entry = conv
                .gold_book
                .find(&item.item_id, &probe.probe_id)
                .expect("gold entry matches probe");
            let w2s_silent = entry.metadata.get("gold_label").map(String::as_str) == Some("SILENT")
                || entry.gold_answer == ">";
            let pa_silent = entry.metadata.get("need").map(String::as_str) == Some("false");
            let mrc_impossible =
                entry.metadata.get("is_impossible").map(String::as_str) == Some("true");
            let pitch_silent = entry.metadata.get("do_not_surface").map(String::as_str)
                == Some("true")
                || (entry.metadata.contains_key("ideal_turn")
                    && entry.metadata.get("turn_index") != entry.metadata.get("ideal_turn"));
            let is_silent = w2s_silent
                || pa_silent
                || mrc_impossible
                || pitch_silent
                || entry.gold_answer.eq_ignore_ascii_case("silent");

            let resp = if entry.privacy_conflict {
                Some("I cannot share private details from another channel.".to_owned())
            } else if is_silent {
                None
            } else {
                Some(entry.gold_answer.clone())
            };
            let action_turn_ms = item.history.last().map(|p| p.timestamp_ms.as_millis());
            out.push(ExternalPrediction {
                item_id: item.item_id.clone(),
                probe_id: probe.probe_id.clone(),
                cluster_id: item.cluster_id.clone(),
                spoke: !is_silent || entry.privacy_conflict,
                response_text: resp,
                action_turn_ms,
                probe_gate_wakes: 1,
                history_gate_wakes: 0,
                tokens_in: 10,
                tokens_out: 5,
                cost_usd: 0.0,
            });
        }
    }
    out
}

fn yes_judge_provider() -> ScriptedModelProvider {
    let mut jp = ScriptedModelProvider::new();
    jp.set_fallback(ScriptedReply::text(
        r#"{"label":"yes","score":1.0,"rationale":"matches gold"}"#,
    ));
    jp
}

fn assert_items_contain_no_literal_gold_answers(conv: &ConvertedDataset, extra_forbidden: &[&str]) {
    let json = serialize_items_json(&conv.items);
    for entry in &conv.gold_book.entries {
        if entry.gold_answer.len() >= 12 {
            assert!(
                !json.contains(&entry.gold_answer),
                "{}: item JSON leaked gold_answer {:?}",
                conv.dataset_id,
                entry.gold_answer
            );
        }
    }
    for bad in extra_forbidden {
        assert!(
            !json.contains(bad),
            "{}: item JSON leaked forbidden string {:?}",
            conv.dataset_id,
            bad
        );
    }
}

fn find_metric<'a>(rep: &'a ScorerReport, name: &str) -> &'a monoagens_lab::datasets::ScorerMetric {
    rep.metrics
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("missing metric '{name}' in {} report", rep.dataset_id))
}

// ---------------------------------------------------------------------------
// Task 7.1: Memory (5)
// ---------------------------------------------------------------------------

#[test]
fn visiblescope_locomo() {
    let conv = convert_twice_byte_identical(&LocomoConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
    // Design §4: locomo probes use `[speaker_a, soul]` and retain Category 5 adversarial QA.
    let first_probe = &conv.items[0].probes[0];
    let members: Vec<&str> = first_probe
        .probe_scope
        .audience
        .members
        .iter()
        .map(ActorId::as_str)
        .collect();
    assert_eq!(members, vec!["maya", "soul"]);
    assert!(
        conv.gold_book
            .entries
            .iter()
            .any(|g| g.category.as_deref() == Some("cat_5")),
        "Category 5 adversarial QA must be retained"
    );
}

#[test]
fn gold_leak_and_scorer_locomo() {
    let conv = convert_twice_byte_identical(&LocomoConverter);
    assert_items_contain_no_literal_gold_answers(
        &conv,
        &["No information available.", "ceramic kiln"],
    );
    let preds = gold_predictions_exact(&conv);
    let rep = LocomoScorer
        .score(
            &preds,
            &conv.gold_book,
            &JudgeExecutionConfig::default(),
            42,
        )
        .unwrap();
    assert!((find_metric(&rep, "qa_f1").value.unwrap() - 1.0).abs() < 1e-6);
    assert!((find_metric(&rep, "cat5_adversarial_acc").value.unwrap() - 1.0).abs() < 1e-6);
}

#[test]
fn visiblescope_longmemeval() {
    let conv = convert_twice_byte_identical(&LongMemEvalConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
    for item in &conv.items {
        for turn in &item.history {
            let mems: BTreeSet<&str> = turn.audience.members.iter().map(ActorId::as_str).collect();
            assert_eq!(mems, BTreeSet::from(["prior_assistant", "soul", "user"]));
        }
        let pmems: BTreeSet<&str> = item.probes[0]
            .probe_scope
            .audience
            .members
            .iter()
            .map(ActorId::as_str)
            .collect();
        assert_eq!(pmems, BTreeSet::from(["soul", "user"]));
    }
}

#[test]
fn gold_leak_and_scorer_longmemeval() {
    let conv = convert_twice_byte_identical(&LongMemEvalConverter);
    assert_items_contain_no_literal_gold_answers(
        &conv,
        &["_abs", "I do not have that information."],
    );
    for item in &conv.items {
        assert!(!item.item_id.contains("_abs"));
        assert!(!item.probes[0].probe_id.contains("_abs"));
    }
    let preds = gold_predictions_exact(&conv);
    let offline = LongMemEvalScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 7)
        .unwrap();
    assert_eq!(find_metric(&offline, "overall_acc").value, None);
    assert!(
        find_metric(&offline, "overall_acc")
            .not_scored_reason
            .is_some()
    );
    assert_eq!(find_metric(&offline, "abstention_acc").value, None);

    let jp = yes_judge_provider();
    let live = LongMemEvalScorer
        .score(
            &preds,
            &conv.gold_book,
            &JudgeExecutionConfig {
                live: true,
                judge_model: Some("gpt-4o"),
                judge_prompt_version: Some("lme-v1"),
                provider: Some(&jp as &dyn ModelProvider),
                ..JudgeExecutionConfig::default()
            },
            7,
        )
        .unwrap();
    assert_eq!(find_metric(&live, "overall_acc").value, Some(1.0));
    assert_eq!(find_metric(&live, "abstention_acc").value, Some(1.0));
}

#[test]
fn visiblescope_memory_agent_bench() {
    let conv = convert_twice_byte_identical(&MemoryAgentBenchConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
    // LME(S*) row -> [user, soul]; non-LME row -> [lab_proctor, soul].
    let lme_item = conv
        .items
        .iter()
        .find(|i| i.strata_key.contains("LongMemEval"))
        .expect("has LME(S*) item");
    let lme_mems: BTreeSet<&str> = lme_item.probes[0]
        .probe_scope
        .audience
        .members
        .iter()
        .map(ActorId::as_str)
        .collect();
    assert_eq!(lme_mems, BTreeSet::from(["user", "soul"]));
    let doc_item = conv
        .items
        .iter()
        .find(|i| !i.strata_key.contains("LongMemEval"))
        .expect("has non-LME item");
    let doc_mems: BTreeSet<&str> = doc_item.probes[0]
        .probe_scope
        .audience
        .members
        .iter()
        .map(ActorId::as_str)
        .collect();
    assert_eq!(doc_mems, BTreeSet::from(["lab_proctor", "soul"]));
    // Synthetic chunk clock T_0 + i*60_000 ms and wrapper stripped.
    assert_eq!(
        doc_item.history[1].timestamp_ms.as_millis() - doc_item.history[0].timestamp_ms.as_millis(),
        60_000
    );
}

#[test]
fn gold_leak_and_scorer_memory_agent_bench() {
    let conv = convert_twice_byte_identical(&MemoryAgentBenchConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &["Please memorize it"]);
    let preds = gold_predictions_exact(&conv);
    let off = MemoryAgentBenchScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 9)
        .unwrap();
    assert_eq!(
        find_metric(&off, "selective_forgetting_acc").value,
        Some(1.0)
    );
    assert_eq!(find_metric(&off, "ar_acc").value, None);

    let jp = yes_judge_provider();
    let live = MemoryAgentBenchScorer
        .score(
            &preds,
            &conv.gold_book,
            &JudgeExecutionConfig {
                live: true,
                judge_model: Some("gpt-4o"),
                judge_prompt_version: None,
                provider: Some(&jp as &dyn ModelProvider),
                ..JudgeExecutionConfig::default()
            },
            9,
        )
        .unwrap();
    assert_eq!(find_metric(&live, "ar_acc").value, Some(1.0));
}

#[test]
fn visiblescope_group_mem_bench() {
    let conv = convert_twice_byte_identical(&GroupMemBenchConverter);
    // Built-in `assert_visiblescope_gold_evidence` asserts allows==true on non-conflict AND allows==false on conflict!
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();

    let conflict_gold = conv
        .gold_book
        .entries
        .iter()
        .find(|e| e.privacy_conflict)
        .expect("fixture includes cross-channel privacy_conflict item");
    let conflict_item = conv
        .items
        .iter()
        .find(|i| i.item_id == conflict_gold.item_id)
        .unwrap();
    let ev = &conflict_gold.gold_evidence[0];
    let t = conflict_item.history[ev.turn_index].timestamp_ms;
    assert!(
        !conflict_item.probes[0]
            .probe_scope
            .allows(&ev.audience, None, t),
        "VisibleScope must block cross-channel evidence for privacy_conflict probe"
    );
    // Reply_to edge reconstructed cleanly.
    assert!(conv.items[0].history.iter().any(|p| p.reply_to.is_some()));
}

#[test]
fn gold_leak_and_scorer_group_mem_bench() {
    let conv = convert_twice_byte_identical(&GroupMemBenchConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &["REFUSE_CROSS_CHANNEL"]);
    let preds = gold_predictions_exact(&conv);
    let off = GroupMemBenchScorer
        .score(
            &preds,
            &conv.gold_book,
            &JudgeExecutionConfig::default(),
            11,
        )
        .unwrap();
    assert_eq!(
        find_metric(&off, "privacy_conflict_refusal_rate").value,
        Some(1.0)
    );
    assert_eq!(find_metric(&off, "acc").value, None);

    let jp = yes_judge_provider();
    let live = GroupMemBenchScorer
        .score(
            &preds,
            &conv.gold_book,
            &JudgeExecutionConfig {
                live: true,
                judge_model: Some("gpt-5"),
                judge_prompt_version: Some("gmb-v1"),
                provider: Some(&jp as &dyn ModelProvider),
                ..JudgeExecutionConfig::default()
            },
            11,
        )
        .unwrap();
    assert_eq!(find_metric(&live, "acc").value, Some(1.0));
    assert_eq!(find_metric(&live, "non_conflict_acc").value, Some(1.0));
    assert_eq!(find_metric(&live, "knowledge_update").value, Some(1.0));
    assert_eq!(find_metric(&live, "term_ambiguity").value, Some(1.0));
}

#[test]
fn visiblescope_stale() {
    let conv = convert_twice_byte_identical(&StaleConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
    assert_eq!(conv.items[0].probes.len(), 3);
    assert!(conv.items[0].probes.iter().all(|p| p.isolated_checkpoint));
}

#[test]
fn gold_leak_and_scorer_stale() {
    let conv = convert_twice_byte_identical(&StaleConverter);
    assert_items_contain_no_literal_gold_answers(
        &conv,
        &[
            "Correct the false premise:",
            "Use the current rotated credentials",
        ],
    );
    let preds = gold_predictions_exact(&conv);
    let off = StaleScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 3)
        .unwrap();
    assert_eq!(find_metric(&off, "state_resolution").value, None);
    assert_eq!(find_metric(&off, "premise_resistance").value, None);
    assert_eq!(find_metric(&off, "implicit_policy_adaptation").value, None);

    let jp = yes_judge_provider();
    let live = StaleScorer
        .score(
            &preds,
            &conv.gold_book,
            &JudgeExecutionConfig {
                live: true,
                judge_model: Some("Gemini-3.1-flash-lite"),
                judge_prompt_version: None,
                provider: Some(&jp as &dyn ModelProvider),
                ..JudgeExecutionConfig::default()
            },
            3,
        )
        .unwrap();
    assert_eq!(find_metric(&live, "state_resolution").value, Some(1.0));
    assert_eq!(find_metric(&live, "premise_resistance").value, Some(1.0));
    assert_eq!(
        find_metric(&live, "implicit_policy_adaptation").value,
        Some(1.0)
    );
}

// ---------------------------------------------------------------------------
// Task 7.2: Prospective & Timing (5 active)
// ---------------------------------------------------------------------------

#[test]
fn visiblescope_pm_bench() {
    let conv = convert_twice_byte_identical(&PmBenchConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
    assert_eq!(conv.items.len(), 2);
}

#[test]
fn gold_leak_and_scorer_pm_bench() {
    let conv = convert_twice_byte_identical(&PmBenchConverter);
    let items_json = serialize_items_json(&conv.items);
    assert!(!items_json.contains("due_task_ids"));
    assert!(!items_json.contains("groundtruth"));
    for item in &conv.items {
        for probe in &item.probes {
            let text = serde_json::to_string(&probe.percept).unwrap();
            assert!(!text.contains("\"status\":\"due\""));
        }
    }
    let preds = gold_predictions_exact(&conv);
    let rep = PmBenchScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 1)
        .unwrap();
    assert_eq!(find_metric(&rep, "set_f1").value, Some(1.0));
    assert_eq!(find_metric(&rep, "false_alarm_rate").value, Some(0.0));
}

#[test]
fn visiblescope_capa() {
    let conv = convert_twice_byte_identical(&CapaConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
}

#[test]
fn gold_leak_and_scorer_capa() {
    let conv = convert_twice_byte_identical(&CapaConverter);
    assert_items_contain_no_literal_gold_answers(
        &conv,
        &["Chassis enclosure changed to matte anodized aluminum."],
    );
    let preds = gold_predictions_exact(&conv);
    let off = CapaScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&off, "credited_recovery").value, None);
    assert!(
        find_metric(&off, "credited_recovery")
            .not_scored_reason
            .is_some()
    );
    assert_eq!(find_metric(&off, "silence_rate").value, Some(0.0));

    let jp = yes_judge_provider();
    let live = CapaScorer
        .score(
            &preds,
            &conv.gold_book,
            &JudgeExecutionConfig {
                live: true,
                judge_model: Some("gpt-4o"),
                judge_prompt_version: None,
                provider: Some(&jp as &dyn ModelProvider),
                ..JudgeExecutionConfig::default()
            },
            5,
        )
        .unwrap();
    assert_eq!(find_metric(&live, "credited_recovery").value, Some(1.0));
    assert_eq!(live.judge_model.as_deref(), Some("gpt-4o"));
    assert_eq!(live.judge_prompt_version.as_deref(), Some("capa-judge-v1"));
}

#[test]
fn visiblescope_proactive_bench() {
    let conv = convert_twice_byte_identical(&ProactiveBenchConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
}

#[test]
fn gold_leak_and_scorer_proactive_bench() {
    let conv = convert_twice_byte_identical(&ProactiveBenchConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &["\"need\":true", "\"need\":false"]);
    let preds = gold_predictions_exact(&conv);
    let rep = ProactiveBenchScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&rep, "f1").value, Some(1.0));
    assert_eq!(find_metric(&rep, "false_detection").value, Some(0.0));
    // RM metric offline (or without fetched local scorer_model) is `not_scored` (`scorer_model_unavailable`).
    let rm = find_metric(&rep, "rm_f1");
    assert_eq!(rm.value, None);
    assert_eq!(
        rm.not_scored_reason.as_deref(),
        Some("scorer_model_unavailable")
    );
}

#[test]
fn visiblescope_pitch() {
    let conv = convert_twice_byte_identical(&PitchConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
}

#[test]
fn gold_leak_and_scorer_pitch() {
    let conv = convert_twice_byte_identical(&PitchConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &["do_not_surface"]);
    let preds = gold_predictions_exact(&conv);
    let rep = PitchScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&rep, "live_best_turn_acc").value, Some(1.0));
    assert_eq!(find_metric(&rep, "in_window_acc").value, Some(1.0));
}

#[test]
fn visiblescope_when2speak() {
    let conv = convert_twice_byte_identical(&When2SpeakConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
}

#[test]
fn gold_leak_and_scorer_when2speak() {
    let conv = convert_twice_byte_identical(&When2SpeakConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &["<speak>"]);
    let preds = gold_predictions_exact(&conv);
    let rep = When2SpeakScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&rep, "macro_f1").value, Some(1.0));
    assert_eq!(find_metric(&rep, "fir").value, Some(0.0));
    assert_eq!(find_metric(&rep, "mir").value, Some(0.0));
}

// ---------------------------------------------------------------------------
// Task 7.3: Group chat (2)
// ---------------------------------------------------------------------------

#[test]
fn visiblescope_ubuntu_irc() {
    let conv = convert_twice_byte_identical(&UbuntuIrcConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
    let probe_mems: BTreeSet<&str> = conv.items[0].probes[0]
        .probe_scope
        .audience
        .members
        .iter()
        .map(ActorId::as_str)
        .collect();
    assert_eq!(probe_mems, BTreeSet::from(["lab_proctor", "soul"]));
}

#[test]
fn gold_leak_and_scorer_ubuntu_irc() {
    let conv = convert_twice_byte_identical(&UbuntuIrcConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &["msg_0 dev_a"]);
    let preds = gold_predictions_exact(&conv);
    let rep = UbuntuIrcScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&rep, "thread_f1").value, Some(1.0));
    assert_eq!(find_metric(&rep, "parent_speaker_p1").value, Some(1.0));
}

#[test]
fn visiblescope_molweni() {
    let conv = convert_twice_byte_identical(&MolweniConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
    assert!(conv.items.iter().any(|i| i.strata_key.starts_with("mrc")));
    assert!(conv.items.iter().any(|i| i.strata_key.starts_with("dp")));
}

#[test]
fn gold_leak_and_scorer_molweni() {
    let conv = convert_twice_byte_identical(&MolweniConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &["\"x\":0,\"y\":1"]);
    let preds = gold_predictions_exact(&conv);
    let rep = MolweniScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&rep, "qa_f1").value, Some(1.0));
    assert_eq!(find_metric(&rep, "sdrt_link_f1").value, Some(1.0));
}

// ---------------------------------------------------------------------------
// Task 7.4: Theory of Mind (6 incl. dyntom + optional tomi)
// ---------------------------------------------------------------------------

#[test]
fn visiblescope_fantom() {
    let conv = convert_twice_byte_identical(&FantomConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
    // Real FANToM info set: fact + belief(dist+mc) + 2 lists + shipped binaries (one `no:long` skipped).
    assert_eq!(conv.items[0].probes.len(), 8);
}

#[test]
fn gold_leak_and_scorer_fantom() {
    let conv = convert_twice_byte_identical(&FantomConverter);
    let items_json = serialize_items_json(&conv.items);
    assert!(!items_json.contains("wrong_belief_answer"));
    assert!(!items_json.contains("\"correct_answer\""));
    let preds = gold_predictions_exact(&conv);
    let rep = FantomScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&rep, "fantom_all").value, Some(1.0));
    // `BeliefQ[Dist.]` present and local SentenceTransformer scorer_model absent -> `fantom_all_star` is `not_scored` (`scorer_model_unavailable`).
    let star = find_metric(&rep, "fantom_all_star");
    assert_eq!(star.value, None);
    assert_eq!(
        star.not_scored_reason.as_deref(),
        Some("scorer_model_unavailable")
    );
}

#[test]
fn visiblescope_opentom() {
    let conv = convert_twice_byte_identical(&OpenTomConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
}

#[test]
fn gold_leak_and_scorer_opentom() {
    let conv = convert_twice_byte_identical(&OpenTomConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &["top of the wooden desk"]);
    let preds = gold_predictions_exact(&conv);
    let rep = OpenTomScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    for m in ["macro_f1_coarse", "macro_f1_fine", "attitude_f1", "mhop_f1"] {
        assert_eq!(find_metric(&rep, m).value, Some(1.0), "metric={m}");
    }
}

#[test]
fn visiblescope_hi_tom() {
    let conv = convert_twice_byte_identical(&HiTomConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
    // Private `Tell` restricts room utterance audience to {Ava, Ben, lab_proctor, soul} (excluding Chloe).
    let tell_turn = &conv.items[0].history[3];
    let tell_mems: BTreeSet<&str> = tell_turn
        .audience
        .members
        .iter()
        .map(ActorId::as_str)
        .collect();
    assert!(!tell_mems.contains("Chloe"));
    assert!(
        VisibleScope::new(conv.items[0].probes[0].probe_scope.audience.clone(), None).allows(
            &tell_turn.audience,
            None,
            tell_turn.timestamp_ms
        )
    );
}

#[test]
fn gold_leak_and_scorer_hi_tom() {
    let conv = convert_twice_byte_identical(&HiTomConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &[]);
    let preds = gold_predictions_exact(&conv);
    let rep = HiTomScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&rep, "standard_acc").value, Some(1.0));
    assert_eq!(find_metric(&rep, "joint_acc_0_to_4").value, Some(1.0));
}

#[test]
fn visiblescope_exploretom() {
    let conv = convert_twice_byte_identical(&ExploreTomConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
}

#[test]
fn gold_leak_and_scorer_exploretom() {
    let conv = convert_twice_byte_identical(&ExploreTomConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &["Human_State"]);
    let preds = gold_predictions_exact(&conv);
    let rep = ExploreTomScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&rep, "exact_match_acc").value, Some(1.0));
}

#[test]
fn visiblescope_dyntom() {
    let conv = convert_twice_byte_identical(&DynTomConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
}

#[test]
fn gold_leak_and_scorer_dyntom() {
    let conv = convert_twice_byte_identical(&DynTomConverter);
    assert_items_contain_no_literal_gold_answers(&conv, &[]);
    let preds = gold_predictions_exact(&conv);
    let rep = DynTomScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(find_metric(&rep, "dyntom_acc").value, Some(1.0));
}

#[test]
fn visiblescope_tomi() {
    let conv = convert_twice_byte_identical(&TomiConverter);
    assert_visiblescope_gold_evidence(&conv.items, &conv.gold_book).unwrap();
}

#[test]
fn gold_leak_and_scorer_tomi() {
    let conv = convert_twice_byte_identical(&TomiConverter);
    assert_items_contain_no_literal_gold_answers(
        &conv,
        &["false_belief_first_order:secret_gold_trace_line_do_not_expose_to_items"],
    );
    let preds = gold_predictions_exact(&conv);
    let rep = TomiScorer
        .score(&preds, &conv.gold_book, &JudgeExecutionConfig::default(), 5)
        .unwrap();
    assert_eq!(
        find_metric(&rep, "shortcut_perturbation_acc").value,
        Some(1.0)
    );
}

// ---------------------------------------------------------------------------
// Task 7.5: Indonesian `reason::EmbedderProbe` (`multilexnorm`, `indocollex`)
// ---------------------------------------------------------------------------

#[test]
fn embedder_probe_multilexnorm_and_indocollex_validation_and_scoring() {
    // 1. Real EmbeddingGemma2Embedder refuses construction when weights path is absent (`ModelPathNotFound`).
    assert!(
        EmbeddingGemma2Embedder::new(EmbeddingGemma2Config::local_http(
            "http://127.0.0.1:9",
            Some(PathBuf::from("/nonexistent/embeddinggemma2.gguf")),
            5_000,
        ))
        .is_err()
    );

    // 2. Both probes report `not_scored` (`embedder_unconfigured`) when embedder is `None`.
    let mln_dir = fixture_dir("multilexnorm");
    let ic_dir = fixture_dir("indocollex");
    let mln_none = MultiLexNormProbe
        .run_probe(&mln_dir, None, false, None, 1)
        .unwrap();
    assert!(matches!(
        mln_none,
        EmbedderProbeOutcome::NotScored { ref reason, .. } if reason == "embedder_unconfigured"
    ));
    let ic_none = IndoCollexProbe
        .run_probe(&ic_dir, None, false, None, 1)
        .unwrap();
    assert!(matches!(
        ic_none,
        EmbedderProbeOutcome::NotScored { ref reason, .. } if reason == "embedder_unconfigured"
    ));

    // 3. Reject HashEmbedder, NoneEmbedder, ScriptedEmbedder when allow_test_embedders == false.
    let hash_emb = HashEmbedder::new(256);
    let none_emb = NoneEmbedder;
    let scripted_emb = ScriptedEmbedder::new("test-scripted-v1", 256);
    for emb in [
        &hash_emb as &dyn monoagens_lab::reason_seam::embed::Embedder,
        &none_emb,
        &scripted_emb,
    ] {
        assert!(
            MultiLexNormProbe
                .run_probe(&mln_dir, Some(emb), false, None, 1)
                .is_err()
        );
        assert!(
            IndoCollexProbe
                .run_probe(&ic_dir, Some(emb), false, None, 1)
                .is_err()
        );
    }

    // 4. With ScriptedEmbedder under explicit test flag (`allow_test_embedders = true`),
    //    verify exact 256-d twin_retrieval_r1_256d == 1.0 on both synthetic fixtures.
    let mut matched = ScriptedEmbedder::new("test-gemma2-mock-v1", 256);
    let unit = |idx: usize| {
        let mut v = vec![0.0_f32; 256];
        v[idx] = 1.0;
        v
    };
    matched.set_fallback(Ok(unit(255)));
    matched.insert_text(TaskPrefix::Query, "gw mau mkn", unit(0));
    matched.insert_text(TaskPrefix::Document, "saya ingin makan", unit(0));
    matched.insert_text(TaskPrefix::Query, "km uda blom", unit(1));
    matched.insert_text(TaskPrefix::Document, "kamu sudah belum", unit(1));
    matched.insert_text(TaskPrefix::Query, "bgt btw oty", unit(2));
    matched.insert_text(
        TaskPrefix::Document,
        "banget by the way on the way",
        unit(2),
    );
    matched.insert_text(TaskPrefix::Query, "kalo ga tau", unit(3));
    matched.insert_text(TaskPrefix::Document, "kalau tidak tahu", unit(3));

    matched.insert_text(TaskPrefix::Query, "gw", unit(10));
    matched.insert_text(TaskPrefix::Document, "aku", unit(10));
    matched.insert_text(TaskPrefix::Query, "yg", unit(11));
    matched.insert_text(TaskPrefix::Document, "yang", unit(11));
    matched.insert_text(TaskPrefix::Query, "dgn", unit(12));
    matched.insert_text(TaskPrefix::Document, "dengan", unit(12));
    matched.insert_text(TaskPrefix::Query, "kmn", unit(13));
    matched.insert_text(TaskPrefix::Document, "ke mana", unit(13));

    let mln_scored = MultiLexNormProbe
        .run_probe(&mln_dir, Some(&matched), true, None, 1)
        .unwrap();
    match mln_scored {
        EmbedderProbeOutcome::Scored {
            dim_d,
            metrics,
            item_count,
            ..
        } => {
            assert_eq!(dim_d, 256);
            assert_eq!(item_count, 4);
            assert_eq!(metrics[0].name, "twin_retrieval_r1_256d");
            assert_eq!(metrics[0].value, Some(1.0));
        }
        other => panic!("expected Scored multilexnorm, got {other:?}"),
    }

    let ic_scored = IndoCollexProbe
        .run_probe(&ic_dir, Some(&matched), true, None, 1)
        .unwrap();
    match ic_scored {
        EmbedderProbeOutcome::Scored {
            dim_d,
            metrics,
            item_count,
            ..
        } => {
            assert_eq!(dim_d, 256);
            assert_eq!(item_count, 4);
            assert_eq!(metrics[0].name, "twin_retrieval_r1_256d");
            assert_eq!(metrics[0].value, Some(1.0));
        }
        other => panic!("expected Scored indocollex, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Task 7.6: Run 3 sampling defaults, cluster bootstrap, full end-to-end suite
// ---------------------------------------------------------------------------

#[test]
fn sampling_defaults_and_cluster_bootstrap_across_benchmarks() {
    assert_eq!(default_sampling_rule("opentom", 7).target_n, Some(471));
    assert!(default_sampling_rule("opentom", 7).sample_by_cluster);
    assert_eq!(default_sampling_rule("exploretom", 7).target_n, Some(471));
    assert_eq!(default_sampling_rule("dyntom", 7).target_n, Some(471));
    assert_eq!(default_sampling_rule("tomi", 7).target_n, Some(471));
    assert_eq!(default_sampling_rule("indocollex", 7).target_n, Some(471));
    assert_eq!(default_sampling_rule("when2speak", 7).target_n, Some(1_000));
    assert_eq!(default_sampling_rule("molweni", 7).target_n, Some(500));

    let obs = vec![
        ("c1".to_owned(), 1.0),
        ("c1".to_owned(), 1.0),
        ("c2".to_owned(), 0.0),
        ("c3".to_owned(), 1.0),
    ];
    let ci = cluster_bootstrap_ci(&obs, 200, 42).unwrap();
    assert_eq!(ci.n_clusters, 3);
    assert!(ci.ci95_low <= ci.point_estimate && ci.point_estimate <= ci.ci95_high);

    // Seed temporary datasets directory with all 20 synthetic fixtures and verify full standard suite execution!
    let tmp = TempDir::new().unwrap();
    let datasets_dir = tmp.path().join("datasets");
    let out_dir = tmp.path().join("records");
    let manifest_path = tmp.path().join("datasets.manifest.toml");
    let registry = standard_benchmark_registry();
    let mut manifest = load_and_validate_manifest(
        &workspace_root().join("lab/datasets.manifest.example.toml"),
        &registry,
    )
    .unwrap();

    for entry in &mut manifest.datasets {
        if entry.availability == DatasetAvailability::Unavailable
            || entry.kind != ManifestEntryKind::Dataset
        {
            continue;
        }
        let src_dir = fixture_dir(&entry.id);
        let dst_dir = datasets_dir.join(&entry.relative_dir);
        for src in &mut entry.sources {
            src.revision = "pinned-rev-1".to_owned();
        }
        entry.license.status = LicenseStatus::Cleared;
        entry.checksums_recorded = ChecksumsRecorded::Owner;
        entry.checksums_recorded_date = Some("2026-11-17".to_owned());
        entry.files.clear();
        copy_and_pin_tree(&src_dir, &src_dir, &dst_dir, &mut entry.files);
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
            files_count: entry.files.len(),
            extracted_ok: true,
        };
        fs::write(
            dst_dir.join(EXTRACTION_MARKER_FILE),
            serde_json::to_vec(&marker).unwrap(),
        )
        .unwrap();
    }
    save_manifest_to_path(&manifest, &manifest_path).unwrap();

    // Verify stratified sampling determinism on converted locomo items.
    let loc_conv = convert_twice_byte_identical(&LocomoConverter);
    let s1 = sample_items_stratified(&loc_conv.items, &default_sampling_rule("locomo", 42));
    let s2 = sample_items_stratified(&loc_conv.items, &default_sampling_rule("locomo", 42));
    assert_eq!(s1.chosen_item_ids, s2.chosen_item_ids);

    // Run each category suite into its own run directory, then let `--suite standard` index the
    // latest per-category records (`Design 8`, Task 11.3).
    for category in [
        "memory",
        "prospective",
        "timing",
        "group",
        "tom",
        "indonesian",
        "security",
    ] {
        let opts = BenchmarkExecutionOptions {
            manifest_path: Some(manifest_path.clone()),
            out_dir: Some(out_dir.join(category)),
            ..BenchmarkExecutionOptions::default()
        };
        run_benchmark_suite_with_options(
            category,
            &datasets_dir,
            None,
            false,
            &CoreConfig::default(),
            &opts,
        )
        .unwrap_or_else(|e| panic!("category suite {category} failed: {e}"));
    }

    let opts = BenchmarkExecutionOptions {
        manifest_path: Some(manifest_path),
        runs_root: Some(out_dir.clone()),
        ..BenchmarkExecutionOptions::default()
    };
    let report: BenchmarkSuiteReport = run_benchmark_suite_with_options(
        "standard",
        &datasets_dir,
        None,
        true,
        &CoreConfig::default(),
        &opts,
    )
    .expect("standard index over all category runs succeeds");

    assert_eq!(report.scorecard.len(), 23);
    // Spot-check lanes and offline statuses:
    let find_row = |id: &str| {
        report
            .scorecard
            .iter()
            .find(|r| r.dataset_id == id)
            .unwrap()
    };
    assert_eq!(find_row("pis").status, ScorecardStatus::Unavailable);
    assert_eq!(find_row("proact").status, ScorecardStatus::Unavailable);
    assert_eq!(find_row("mpbench").status, ScorecardStatus::Unavailable);
    assert_eq!(find_row("multilexnorm").lane, DatasetLane::EmbedderProbe);
    assert_eq!(find_row("multilexnorm").status, ScorecardStatus::NotScored);
    assert_eq!(find_row("indocollex").status, ScorecardStatus::NotScored);
    assert_eq!(find_row("longmemeval").status, ScorecardStatus::NotScored);
    assert_eq!(find_row("stale").status, ScorecardStatus::NotScored);
    assert_eq!(find_row("locomo").status, ScorecardStatus::Run);
    assert_eq!(find_row("pm_bench").status, ScorecardStatus::Run);
    assert_eq!(find_row("dyntom").status, ScorecardStatus::Run);
    assert_eq!(find_row("ubuntu_irc").status, ScorecardStatus::Run);
    assert_eq!(find_row("molweni").status, ScorecardStatus::Run);
    assert!(report.pareto.is_some());
    assert!(out_dir.join("memory/locomo__seed-1.postcard").is_file());
    assert!(
        GoldBook {
            dataset_id: "x".into(),
            entries: vec![]
        }
        .find("a", "b")
        .is_none()
    );
}

fn copy_and_pin_tree(root: &Path, cur: &Path, dst_root: &Path, out_files: &mut Vec<ManifestFile>) {
    let mut entries: Vec<_> = fs::read_dir(cur).unwrap().filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for ent in entries {
        let p = ent.path();
        if p.is_dir() {
            copy_and_pin_tree(root, &p, dst_root, out_files);
        } else {
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let dst = dst_root.join(&rel);
            if let Some(par) = dst.parent() {
                fs::create_dir_all(par).unwrap();
            }
            let bytes = fs::read(&p).unwrap();
            let sha = sha256_hex(&bytes);
            fs::write(&dst, &bytes).unwrap();
            out_files.push(ManifestFile {
                path: rel,
                source_role: None,
                sha256: PinnedSha256::Sha256(sha),
                size: PinnedSize::Bytes(bytes.len() as u64),
                download_url: None,
                extract: None,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Offline helper contracts + `#[ignore]`d tests needing fetched local weights
// ---------------------------------------------------------------------------

#[test]
fn python_helper_capa_request_builder_runs_offline_without_openai_or_api_keys() {
    let tmp = TempDir::new().unwrap();
    fs::create_dir_all(tmp.path().join("prompts")).unwrap();
    fs::write(
        tmp.path().join("prompts/delegate_judge.txt"),
        "Official CAPA pinned prompt: role={absent_role} target={target_proposition} resp={response_text}",
    )
    .unwrap();

    let payload = serde_json::json!({
        "dataset_dir": tmp.path(),
        "items": [
            {
                "item_id": "ep1",
                "absent_role": "Industrial Designer",
                "target_proposition": "Chassis enclosure changed to matte anodized aluminum.",
                "response_text": "We switched the enclosure material to matte anodized aluminum."
            },
            {
                "item_id": "ep2",
                "absent_role": "Industrial Designer",
                "target_proposition": "Battery moved to bottom tray.",
                "response_text": "   "
            }
        ]
    });
    if let Some(out) = run_optional_python_helper(
        &workspace_root(),
        "lab/adapters/eval_helpers/capa_judge.py",
        &payload,
    )
    .expect("capa_judge.py must run offline without openai or API keys")
    {
        let pv = out["prompt_version"].as_str().unwrap();
        assert!(pv.starts_with("capa-pinned-"), "prompt_version={pv}");
        let reqs = out["requests"].as_array().unwrap();
        assert_eq!(reqs.len(), 2);
        assert_eq!(reqs[0]["skip"], serde_json::json!(false));
        assert!(
            reqs[0]["user_prompt"]
                .as_str()
                .unwrap()
                .contains("Official CAPA pinned prompt:")
        );
        assert_eq!(reqs[1]["skip"], serde_json::json!(true));
    }

    // Also verify missing local scorer_model fails clearly in Python helpers (never downloading)
    // and reports `scorer_model_unavailable` from Rust when manifest/datasets_dir are wired.
    let tmp_missing = TempDir::new().unwrap();
    let missing_dir = tmp_missing.path().join("absent_model");
    for script in [
        "lab/adapters/eval_helpers/fantom_eval.py",
        "lab/adapters/eval_helpers/proactive_rm.py",
    ] {
        let res = run_optional_python_helper(
            &workspace_root(),
            script,
            &serde_json::json!({
                "model_path": missing_dir,
                "items": [{"item_id": "x", "response": "hi", "gold": "hi", "context": "c"}]
            }),
        );
        match res {
            Ok(None) => {} // sentence_transformers / transformers not installed in env
            Ok(Some(v)) => panic!("{script} unexpectedly succeeded without model files: {v}"),
            Err(e) => assert!(
                e.to_string().contains("scorer_model_unavailable"),
                "{script} must fail with scorer_model_unavailable, got: {e}"
            ),
        }
    }
}

#[test]
#[ignore = "requires Python environment with fetched local sentence_transformers weights under lab/datasets/scorer_models/all-roberta-large-v1"]
fn python_helper_fantom_sentence_transformer_eval() {
    let model_dir = workspace_root().join("lab/datasets/scorer_models/all-roberta-large-v1");
    let payload = serde_json::json!({
        "model_path": model_dir,
        "items": [{
            "item_id": "set_a",
            "response": "Owen heard Vera mention the blue drawer.",
            "gold": "Owen heard Vera say the spare battery is in the blue drawer.",
            "wrong_belief": "Owen thinks the drawer is empty."
        }]
    });
    let out = run_optional_python_helper(
        &workspace_root(),
        "lab/adapters/eval_helpers/fantom_eval.py",
        &payload,
    )
    .unwrap()
    .expect("sentence_transformers + local weights must be installed");
    assert_eq!(out["results"][0]["ok"], serde_json::json!(true));
}

#[test]
#[ignore = "requires Python environment with fetched local ProactiveAgent-RM weights under lab/datasets/scorer_models/proactive-agent-rm"]
fn python_helper_proactive_rm_eval() {
    let model_dir = workspace_root().join("lab/datasets/scorer_models/proactive-agent-rm");
    let payload = serde_json::json!({
        "model_path": model_dir,
        "items": [{
            "item_id": "pa_1",
            "context": "User left port unquoted in config.toml.",
            "response": "Quote the port number as a string on line 4.",
            "reference": "Point out the missing quotes around the port string."
        }]
    });
    let out = run_optional_python_helper(
        &workspace_root(),
        "lab/adapters/eval_helpers/proactive_rm.py",
        &payload,
    )
    .unwrap()
    .expect("transformers + torch + local ProactiveAgent-RM weights required");
    assert!(out["results"][0]["score"].as_f64().is_some());
}

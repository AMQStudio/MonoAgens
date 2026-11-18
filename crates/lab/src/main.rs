//! `monoagens-lab` CLI binary for running scenarios, suites, ablations, replay, and mechanisms.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, BufWriter};
use std::path::PathBuf;
use std::process::{Child, Command, ExitCode, Stdio};
use std::time::Duration;

use monoagens_core::{MechanismId, MemStore, ReplayMode, Switches, brain_map_registry};
use monoagens_lab::reason_seam::embed::{
    EmbedInput, Embedder, EmbeddingGemma2Config, EmbeddingGemma2Embedder,
};
use monoagens_lab::reason_seam::model::{OpenAiModelConfig, OpenAiModelProvider, ReasoningEffort};
use monoagens_lab::reason_seam::secrets::SecretString;
use monoagens_lab::{
    BaselineKind, ParetoScorecardInput, Scenario, ScenarioSet, ScriptedActionRule, TestBody,
    TestBodySpec, aggregate_metrics_across_seeds, compute_convex_pareto, datasets,
    ensure_not_sealed_path, format_metrics_table, load_scenarios_from_set, replay_events_offline,
    run_ablation_compare_scenarios, run_baseline_adapter, run_benchmark_suite_with_options,
    run_scenario, standard_benchmark_registry,
};
use monoagens_protocol::{
    BodyId, CapabilityDescriptor, CapabilityFlags, CapabilityId, CapabilitySchemaHash, PlaceId,
    ReceiptOutcome, TerminationReason,
};
use monoagens_store::load_prior_or_default;

fn main() -> ExitCode {
    match run_cli(env::args().skip(1).collect()) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::from(2)
        }
    }
}

fn run_cli(args: Vec<String>) -> Result<ExitCode, String> {
    let Some(cmd) = args.first().map(String::as_str) else {
        return Err(usage());
    };

    match cmd {
        "-h" | "--help" => {
            println!("{}", usage());
            Ok(ExitCode::SUCCESS)
        }
        "mechanisms" => {
            let json = args[1..].iter().any(|a| a == "--json");
            let unknown: Vec<&String> = args[1..].iter().filter(|a| *a != "--json").collect();
            if !unknown.is_empty() {
                return Err(format!(
                    "unknown argument(s) for mechanisms: {}",
                    unknown[0]
                ));
            }
            let entries = brain_map_registry().map_err(|e| e.to_string())?;
            if json {
                let out = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
                println!("{out}");
            } else {
                for entry in &entries {
                    println!(
                        "{:02}. {} ({}) — neutral: {}",
                        entry.row, entry.stable_id, entry.function_name, entry.neutral_description
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        "run" => run_single_cmd(&args[1..]),
        "suite" => run_suite_cmd(&args[1..]),
        "compare" => run_compare_cmd(&args[1..]),
        "replay" => run_replay_cmd(&args[1..]),
        "baseline" => run_baseline_cmd(&args[1..]),
        "benchmark" => run_benchmark_cmd(&args[1..]),
        "report" => run_report_cmd(&args[1..]),
        "datasets" => run_datasets_cmd(&args[1..]),
        "pareto" => run_pareto_cmd(&args[1..]),
        "test-body" => run_test_body_stdio(&args[1..]),
        other => Err(format!("unknown subcommand '{other}'\n\n{}", usage())),
    }
}

fn run_single_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut scenario_path: Option<PathBuf> = None;
    let mut prior_path: Option<PathBuf> = None;
    let mut seed: u64 = 1;
    let mut json = false;
    let mut record_out: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--scenario" => {
                i += 1;
                let val = args.get(i).ok_or("--scenario requires a path")?;
                scenario_path = Some(PathBuf::from(val));
            }
            "--prior" => {
                i += 1;
                let val = args.get(i).ok_or("--prior requires a path")?;
                prior_path = Some(PathBuf::from(val));
            }
            "--seed" => {
                i += 1;
                let val = args.get(i).ok_or("--seed requires an integer")?;
                seed = val
                    .parse::<u64>()
                    .map_err(|e| format!("invalid --seed '{val}': {e}"))?;
            }
            "--json" => {
                json = true;
            }
            "--record-out" => {
                i += 1;
                let val = args.get(i).ok_or("--record-out requires a path")?;
                record_out = Some(PathBuf::from(val));
            }
            other => {
                return Err(format!("unknown argument '{other}'"));
            }
        }
        i += 1;
    }

    let loaded_prior = load_prior_or_default(prior_path.as_deref()).map_err(|e| e.to_string())?;
    let path = scenario_path.ok_or("run requires --scenario <path>")?;
    ensure_not_sealed_path(&path).map_err(|e| e.to_string())?;
    let raw = fs::read_to_string(&path)
        .map_err(|e| format!("failed to read scenario '{}': {e}", path.display()))?;
    let scenario: Scenario = serde_json::from_str(&raw)
        .map_err(|e| format!("failed to parse scenario '{}': {e}", path.display()))?;

    let output = run_scenario(MemStore::new(), loaded_prior.prior, &scenario, seed)
        .map_err(|e| format!("scenario run failed: {e}"))?;

    if let Some(out_path) = record_out {
        let bytes = output
            .run_record
            .encode_postcard()
            .map_err(|e| e.to_string())?;
        if let Some(parent) = out_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| format!("create output dir failed: {e}"))?;
        }
        fs::write(&out_path, bytes)
            .map_err(|e| format!("write record '{}' failed: {e}", out_path.display()))?;
    }

    if json {
        let payload = output.run_record.to_machine_json();
        let rendered = serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?;
        println!("{rendered}");
    } else {
        println!(
            "scenario={} set={:?} passed={} events={}",
            output.run_record.scenario_id,
            output.run_record.scenario_set,
            output.run_record.passed,
            output.run_record.metrics.total_events
        );
        if !output.run_record.sealed {
            for check in &output.run_record.checks {
                let status = if check.passed { "PASS" } else { "FAIL" };
                println!("  [{status}] {}: {}", check.name, check.detail);
            }
            println!("\n{}", format_metrics_table(&output.run_record.metrics));
        }
    }

    if output.run_record.passed {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn run_suite_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut root = PathBuf::from("lab/scenarios");
    let mut set = ScenarioSet::Development;
    let mut seeds: Vec<u64> = vec![1];
    let mut prior_path: Option<PathBuf> = None;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                root = PathBuf::from(args.get(i).ok_or("--root requires a path")?);
            }
            "--set" => {
                i += 1;
                let val = args.get(i).ok_or("--set requires development|validation")?;
                set = parse_non_sealed_set(val)?;
            }
            "--prior" => {
                i += 1;
                prior_path = Some(PathBuf::from(args.get(i).ok_or("--prior requires a path")?));
            }
            "--seeds" => {
                i += 1;
                let val = args.get(i).ok_or("--seeds requires comma-separated u64s")?;
                seeds = val
                    .split(',')
                    .map(|s| {
                        s.trim()
                            .parse::<u64>()
                            .map_err(|e| format!("invalid seed '{s}': {e}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "--json" => json = true,
            other => return Err(format!("unknown argument '{other}' for suite")),
        }
        i += 1;
    }

    let loaded_prior = load_prior_or_default(prior_path.as_deref()).map_err(|e| e.to_string())?;
    let scenarios = load_scenarios_from_set(&root, set).map_err(|e| e.to_string())?;
    let mut all_passed = true;
    let mut records = Vec::new();
    let mut metric_runs = Vec::new();

    for sc in &scenarios {
        for &seed in &seeds {
            let out = run_scenario(MemStore::new(), loaded_prior.prior.clone(), sc, seed)
                .map_err(|e| e.to_string())?;
            all_passed &= out.run_record.passed;
            metric_runs.push(out.run_record.metrics.clone());
            records.push(out.run_record.to_machine_json());
        }
    }
    let aggregates = aggregate_metrics_across_seeds(&metric_runs);

    if json {
        let payload = serde_json::json!({
            "set": set,
            "scenario_count": scenarios.len(),
            "seed_count": seeds.len(),
            "passed": all_passed,
            "aggregates": aggregates,
            "runs": records,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "suite={:?} scenarios={} seeds={} passed={}",
            set,
            scenarios.len(),
            seeds.len(),
            all_passed
        );
        for agg in &aggregates {
            if let Some(m) = agg.mean {
                let hw = agg.ci95_half_width.unwrap_or(0.0);
                println!(
                    "  {} = {:.4} +/- {:.4} (n={})",
                    agg.name, m, hw, agg.n_seeds
                );
            }
        }
    }

    if all_passed {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn parse_non_sealed_set(val: &str) -> Result<ScenarioSet, String> {
    match val {
        "development" | "dev" => Ok(ScenarioSet::Development),
        "validation" | "val" => Ok(ScenarioSet::Validation),
        "sealed" => Err("refusing to enumerate or run lab/scenarios/sealed/".to_owned()),
        other => Err(format!("unknown scenario set '{other}'")),
    }
}

fn run_compare_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut scenario_path: Option<PathBuf> = None;
    let mut set_opt: Option<ScenarioSet> = None;
    let mut root = PathBuf::from("lab/scenarios");
    let mut out_dir: Option<PathBuf> = None;
    let mut prior_path: Option<PathBuf> = None;
    let mut mech_name: Option<String> = None;
    let mut seeds: Vec<u64> = vec![1, 2, 3, 4, 5];
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--scenario" => {
                i += 1;
                scenario_path = Some(PathBuf::from(
                    args.get(i).ok_or("--scenario requires a path")?,
                ));
            }
            "--set" => {
                i += 1;
                let val = args.get(i).ok_or("--set requires development|validation")?;
                set_opt = Some(parse_non_sealed_set(val)?);
            }
            "--root" => {
                i += 1;
                root = PathBuf::from(args.get(i).ok_or("--root requires a path")?);
            }
            "--out-dir" | "--output-dir" => {
                i += 1;
                out_dir = Some(PathBuf::from(
                    args.get(i).ok_or("--out-dir requires a directory path")?,
                ));
            }
            "--prior" => {
                i += 1;
                prior_path = Some(PathBuf::from(args.get(i).ok_or("--prior requires a path")?));
            }
            "--mechanism" => {
                i += 1;
                mech_name = Some(
                    args.get(i)
                        .ok_or("--mechanism requires a mechanism id or 'self_mechanics_guide'")?
                        .clone(),
                );
            }
            "--seeds" => {
                i += 1;
                let val = args.get(i).ok_or("--seeds requires comma-separated u64s")?;
                seeds = val
                    .split(',')
                    .map(|s| {
                        s.trim()
                            .parse::<u64>()
                            .map_err(|e| format!("invalid seed '{s}': {e}"))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "--json" => json = true,
            other => return Err(format!("unknown argument '{other}' for compare")),
        }
        i += 1;
    }

    if seeds.len() < 5 {
        return Err("compare requires at least 5 seeds per side (specs/lab/spec.md)".to_owned());
    }
    let loaded_prior = load_prior_or_default(prior_path.as_deref()).map_err(|e| e.to_string())?;
    let scenarios: Vec<Scenario> = match (scenario_path, set_opt) {
        (Some(_), Some(_)) => {
            return Err("compare accepts either --scenario or --set, not both".to_owned());
        }
        (None, None) => {
            return Err(
                "compare requires --scenario <path> or --set <development|validation>".to_owned(),
            );
        }
        (Some(path), None) => {
            ensure_not_sealed_path(&path).map_err(|e| e.to_string())?;
            let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let scenario: Scenario = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            vec![scenario]
        }
        (None, Some(set)) => load_scenarios_from_set(&root, set).map_err(|e| e.to_string())?,
    };
    let mech = mech_name.ok_or("compare requires --mechanism <id>")?;

    let mut enabled = Switches::all_enabled();
    let mut neutral = Switches::all_enabled();
    if mech == "self_mechanics_guide" {
        enabled.self_mechanics_guide = true;
        neutral.self_mechanics_guide = false;
    } else {
        let id = MechanismId::ALL
            .into_iter()
            .find(|m| m.stable_id() == mech || format!("{m:?}").eq_ignore_ascii_case(&mech))
            .ok_or_else(|| format!("unknown mechanism '{mech}'"))?;
        neutral.set_neutral(id);
    }

    let deltas = run_ablation_compare_scenarios(
        &scenarios,
        &loaded_prior.prior,
        enabled,
        neutral,
        &seeds,
        out_dir.as_deref(),
    )
    .map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&deltas).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "ablation mechanism={mech} scenarios={} seeds={}",
            scenarios.len(),
            seeds.len()
        );
        if let Some(dir) = &out_dir {
            println!("records_dir={}", dir.display());
        }
        for d in &deltas {
            if let Some(md) = d.mean_delta {
                let hw = d.ci95_half_width.unwrap_or(0.0);
                let flag = if d.within_seed_noise {
                    "no_measured_effect"
                } else {
                    "significant"
                };
                println!("  {} delta={md:+.4} +/- {hw:.4} [{flag}]", d.metric);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_replay_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut scenario_path: Option<PathBuf> = None;
    let mut prior_path: Option<PathBuf> = None;
    let mut seed: u64 = 1;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--scenario" => {
                i += 1;
                scenario_path = Some(PathBuf::from(
                    args.get(i).ok_or("--scenario requires a path")?,
                ));
            }
            "--prior" => {
                i += 1;
                prior_path = Some(PathBuf::from(args.get(i).ok_or("--prior requires a path")?));
            }
            "--seed" => {
                i += 1;
                let val = args.get(i).ok_or("--seed requires an integer")?;
                seed = val
                    .parse::<u64>()
                    .map_err(|e| format!("invalid --seed '{val}': {e}"))?;
            }
            "--json" => json = true,
            other => return Err(format!("unknown argument '{other}' for replay")),
        }
        i += 1;
    }

    let loaded_prior = load_prior_or_default(prior_path.as_deref()).map_err(|e| e.to_string())?;
    let path = scenario_path.ok_or("replay requires --scenario <path>")?;
    ensure_not_sealed_path(&path).map_err(|e| e.to_string())?;
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let scenario: Scenario = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    let out = run_scenario(MemStore::new(), loaded_prior.prior, &scenario, seed)
        .map_err(|e| e.to_string())?;
    let switches = scenario.switches.clone().unwrap_or_default();
    let report = replay_events_offline(&switches, seed, &out.events, ReplayMode::FullReplay)
        .map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "replay scenario={} events={} matched_decisions={} bit_identical={}",
            scenario.id,
            report.replayed_events,
            report.matched_decisions,
            report.is_bit_identical()
        );
    }
    if report.is_bit_identical() {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn run_test_body_stdio(args: &[String]) -> Result<ExitCode, String> {
    let mut state_dir: Option<PathBuf> = env::var_os("MONOAGENS_BODY_STATE_DIR").map(PathBuf::from);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--state-dir" => {
                i += 1;
                let val = args.get(i).ok_or("--state-dir requires a path")?;
                state_dir = Some(PathBuf::from(val));
            }
            other => return Err(format!("unknown argument '{other}' for test-body")),
        }
        i += 1;
    }

    let room = PlaceId::new("lab:room-1");
    let device = PlaceId::new("lab:device-1");
    let mut body = TestBody::from_spec(TestBodySpec {
        body_id: BodyId::new("lab-test-body"),
        places: vec![room.clone(), device.clone()],
        capabilities: vec![
            CapabilityDescriptor {
                id: CapabilityId::new("a.speak"),
                description: "Lab speak capability".to_owned(),
                places: vec![room],
                flags: CapabilityFlags {
                    communicative: true,
                    identity_changing: false,
                    irreversible: false,
                },
                typical_latency_ms: 5,
                capability_schema_hash: CapabilitySchemaHash::new("sha256:cap-a.speak"),
                sensed_effect: None,
            },
            CapabilityDescriptor {
                id: CapabilityId::new("z.push"),
                description: "Lab push capability".to_owned(),
                places: vec![device],
                flags: CapabilityFlags {
                    communicative: false,
                    identity_changing: false,
                    irreversible: true,
                },
                typical_latency_ms: 5,
                capability_schema_hash: CapabilitySchemaHash::new("sha256:cap-z.push"),
                sensed_effect: None,
            },
        ],
        action_scripts: vec![
            ScriptedActionRule {
                capability_id: CapabilityId::new("a.speak"),
                place: None,
                payload_contains: Some("hold".to_owned()),
                outcome: ReceiptOutcome::Held,
                receipt_delay_ms: 0,
                termination_reason: Some(TerminationReason::Normal),
                progress: Some(0.0),
                delivered_units: Some(0),
                reason_code: Some("PendingApproval".to_owned()),
                detail: Some("held for approval".to_owned()),
                follow_up_percepts: Vec::new(),
            },
            ScriptedActionRule {
                capability_id: CapabilityId::new("z.push"),
                place: None,
                payload_contains: Some("hold".to_owned()),
                outcome: ReceiptOutcome::Held,
                receipt_delay_ms: 0,
                termination_reason: Some(TerminationReason::Normal),
                progress: Some(0.0),
                delivered_units: Some(0),
                reason_code: Some("PendingApproval".to_owned()),
                detail: Some("held for approval".to_owned()),
                follow_up_percepts: Vec::new(),
            },
        ],
    });

    let stdin = io::stdin();
    let stdout = io::stdout();
    let writer = BufWriter::new(stdout.lock());
    body.run_stdio_with_state_dir(state_dir.as_deref(), stdin.lock(), writer)
        .map_err(|e| format!("test-body stdio failed: {e}"))?;
    Ok(ExitCode::SUCCESS)
}

fn run_baseline_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut adapter: Option<BaselineKind> = None;
    let mut scenario_path: Option<PathBuf> = None;
    let mut prior_path: Option<PathBuf> = None;
    let mut seed: u64 = 1;
    let mut sealed = false;
    let mut timeout_s: u64 = 5;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--adapter" => {
                i += 1;
                let val = args.get(i).ok_or("--adapter requires a slug")?;
                adapter = Some(BaselineKind::parse(val)?);
            }
            "--scenario" => {
                i += 1;
                scenario_path = Some(PathBuf::from(
                    args.get(i).ok_or("--scenario requires a path")?,
                ));
            }
            "--prior" => {
                i += 1;
                prior_path = Some(PathBuf::from(args.get(i).ok_or("--prior requires a path")?));
            }
            "--seed" => {
                i += 1;
                let val = args.get(i).ok_or("--seed requires an integer")?;
                seed = val
                    .parse::<u64>()
                    .map_err(|e| format!("invalid --seed '{val}': {e}"))?;
            }
            "--sealed" => sealed = true,
            "--timeout-s" => {
                i += 1;
                let val = args.get(i).ok_or("--timeout-s requires an integer")?;
                timeout_s = val
                    .parse::<u64>()
                    .map_err(|e| format!("invalid --timeout-s '{val}': {e}"))?;
            }
            "--json" => json = true,
            other => return Err(format!("unknown argument '{other}' for baseline")),
        }
        i += 1;
    }

    let loaded_prior = load_prior_or_default(prior_path.as_deref()).map_err(|e| e.to_string())?;
    let kind = adapter.ok_or("baseline requires --adapter <slug>")?;
    let path = scenario_path.ok_or("baseline requires --scenario <path>")?;
    if !sealed {
        ensure_not_sealed_path(&path).map_err(|e| e.to_string())?;
    }
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let scenario: Scenario = serde_json::from_str(&raw).map_err(|e| e.to_string())?;

    let root = env::current_dir().map_err(|e| e.to_string())?;
    let script = kind.adapter_script_path(&root);
    let report = run_baseline_adapter(
        kind,
        &script,
        &scenario,
        &loaded_prior.prior,
        seed,
        sealed,
        None,
        Duration::from_secs(timeout_s),
        &[],
    )
    .map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "baseline={} sdk_verified={} steps={} noop_llm_wakes={} mono_gate_suppressions={} reason={:?}",
            report.adapter,
            report.sdk_verified,
            report.steps.len(),
            report.noop_llm_wakes,
            report.monoagens_zero_llm_gate_suppressions,
            report.unavailable_reason,
        );
    }
    if report.sdk_verified {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

fn run_benchmark_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut suite = "standard".to_owned();
    let mut datasets_dir = PathBuf::from("lab/datasets");
    let mut manifest_path: Option<PathBuf> = None;
    let mut prior_path: Option<PathBuf> = None;
    let mut pareto_flag = false;
    let mut pareto_input_path: Option<PathBuf> = None;
    let mut only_ids: Option<Vec<String>> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut runs_root: Option<PathBuf> = None;
    let mut seed: u64 = 1;
    let mut live = false;
    let mut max_cost_usd: Option<f64> = None;
    let mut model_endpoint: Option<String> = None;
    let mut model_name = "scripted".to_owned();
    let mut model_api_key_env: Option<String> = None;
    let mut judge_endpoint: Option<String> = None;
    let mut judge_model: Option<String> = None;
    let mut judge_api_key_env: Option<String> = None;
    let mut judge_prompt_version = "v1".to_owned();
    let mut judge_sub_policy = datasets::JudgeSubstitutionPolicy::RenameMetric;
    let mut reasoning_effort: Option<ReasoningEffort> = None;
    let mut judge_reasoning_effort: Option<ReasoningEffort> = None;
    let mut extra_headers_spec: Vec<String> = Vec::new();
    let mut embed_endpoint: Option<String> = None;
    let mut embed_model_path: Option<PathBuf> = None;
    let mut allow_no_embedder = false;
    let mut preflight_only = false;
    let mut pilot_items: Option<usize> = None;
    let mut memory_phase = "0".to_owned();
    let mut memory_pilot = false;
    let mut memory_reuse_from: Vec<PathBuf> = Vec::new();
    let mut report_flag = false;
    let mut report_notes: Vec<String> = Vec::new();
    let mut secrets_path: Option<PathBuf> = None;
    let mut profile_path: Option<PathBuf> = None;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--suite" => {
                i += 1;
                suite = args
                    .get(i)
                    .ok_or("--suite requires standard, in_tree_only, or a category")?
                    .clone();
            }
            "--datasets-dir" => {
                i += 1;
                datasets_dir = PathBuf::from(args.get(i).ok_or("--datasets-dir requires a path")?);
            }
            "--manifest" => {
                i += 1;
                manifest_path = Some(PathBuf::from(
                    args.get(i).ok_or("--manifest requires a path")?,
                ));
            }
            "--prior" => {
                i += 1;
                prior_path = Some(PathBuf::from(args.get(i).ok_or("--prior requires a path")?));
            }
            "--profile" => {
                i += 1;
                profile_path = Some(PathBuf::from(
                    args.get(i).ok_or("--profile requires a path")?,
                ));
            }
            "--only" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or("--only requires comma-separated dataset ids")?;
                let mut list = only_ids.take().unwrap_or_default();
                push_dataset_ids(&mut list, val);
                only_ids = Some(list);
            }
            "--out-dir" => {
                i += 1;
                out_dir = Some(PathBuf::from(
                    args.get(i).ok_or("--out-dir requires a directory path")?,
                ));
            }
            "--seed" => {
                i += 1;
                let val = args.get(i).ok_or("--seed requires an integer")?;
                seed = val
                    .parse::<u64>()
                    .map_err(|e| format!("invalid --seed '{val}': {e}"))?;
            }
            "--live" => live = true,
            "--runs-root" => {
                i += 1;
                runs_root = Some(PathBuf::from(
                    args.get(i).ok_or("--runs-root requires a directory path")?,
                ));
            }
            "--max-cost-usd" => {
                i += 1;
                let val = args.get(i).ok_or("--max-cost-usd requires a float")?;
                max_cost_usd = Some(
                    val.parse::<f64>()
                        .map_err(|e| format!("invalid --max-cost-usd '{val}': {e}"))?,
                );
            }
            "--model-endpoint" => {
                i += 1;
                model_endpoint = Some(
                    args.get(i)
                        .ok_or("--model-endpoint requires a URL")?
                        .clone(),
                );
            }
            "--model" => {
                i += 1;
                model_name = args.get(i).ok_or("--model requires a model id")?.clone();
            }
            "--model-api-key-env" => {
                i += 1;
                model_api_key_env = Some(
                    args.get(i)
                        .ok_or("--model-api-key-env requires an env var")?
                        .clone(),
                );
            }
            "--judge-endpoint" => {
                i += 1;
                judge_endpoint = Some(
                    args.get(i)
                        .ok_or("--judge-endpoint requires a URL")?
                        .clone(),
                );
            }
            "--judge-model" => {
                i += 1;
                judge_model = Some(
                    args.get(i)
                        .ok_or("--judge-model requires a model id")?
                        .clone(),
                );
            }
            "--judge-api-key-env" => {
                i += 1;
                judge_api_key_env = Some(
                    args.get(i)
                        .ok_or("--judge-api-key-env requires an env var")?
                        .clone(),
                );
            }
            "--judge-prompt-version" => {
                i += 1;
                judge_prompt_version = args
                    .get(i)
                    .ok_or("--judge-prompt-version requires a version string")?
                    .clone();
            }
            "--judge-substitution" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or("--judge-substitution requires 'rename' or 'fail'")?;
                judge_sub_policy = match val.as_str() {
                    "rename" => datasets::JudgeSubstitutionPolicy::RenameMetric,
                    "fail" => datasets::JudgeSubstitutionPolicy::StrictFail,
                    other => return Err(format!("unknown --judge-substitution '{other}'")),
                };
            }
            "--pareto" => {
                pareto_flag = true;
            }
            "--pareto-input" => {
                i += 1;
                pareto_input_path = Some(PathBuf::from(
                    args.get(i).ok_or("--pareto-input requires a path")?,
                ));
            }
            "--secrets" => {
                i += 1;
                secrets_path = Some(PathBuf::from(
                    args.get(i).ok_or("--secrets requires a path")?,
                ));
            }
            "--reasoning-effort" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or("--reasoning-effort requires low|medium|high|max")?;
                reasoning_effort = Some(val.parse::<ReasoningEffort>()?);
            }
            "--judge-reasoning-effort" => {
                i += 1;
                let val = args
                    .get(i)
                    .ok_or("--judge-reasoning-effort requires low|medium|high|max")?;
                judge_reasoning_effort = Some(val.parse::<ReasoningEffort>()?);
            }
            "--extra-header" => {
                i += 1;
                let val = args.get(i).ok_or("--extra-header requires NAME=ENV_VAR")?;
                extra_headers_spec.push(val.clone());
            }
            "--embed-endpoint" => {
                i += 1;
                embed_endpoint = Some(
                    args.get(i)
                        .ok_or("--embed-endpoint requires a URL")?
                        .clone(),
                );
            }
            "--embed-model-path" => {
                i += 1;
                embed_model_path = Some(PathBuf::from(
                    args.get(i).ok_or("--embed-model-path requires a path")?,
                ));
            }
            "--allow-no-embedder" => allow_no_embedder = true,
            "--preflight-only" => preflight_only = true,
            "--pilot-items" => {
                i += 1;
                let val = args.get(i).ok_or("--pilot-items requires an integer")?;
                pilot_items = Some(
                    val.parse::<usize>()
                        .map_err(|e| format!("invalid --pilot-items '{val}': {e}"))?,
                );
            }
            "--phase" => {
                i += 1;
                memory_phase = args.get(i).ok_or("--phase requires 0")?.clone();
            }
            "--pilot" => memory_pilot = true,
            "--reuse-items-from" => {
                i += 1;
                memory_reuse_from.push(PathBuf::from(
                    args.get(i)
                        .ok_or("--reuse-items-from requires a run directory")?,
                ));
            }
            "--report" => report_flag = true,
            "--report-note" => {
                i += 1;
                report_notes.push(
                    args.get(i)
                        .ok_or("--report-note requires a string")?
                        .clone(),
                );
            }
            "--json" => json = true,
            other => return Err(format!("unknown argument '{other}' for benchmark")),
        }
        i += 1;
    }

    let category_suite = datasets::suite_domain(&suite).is_some();
    let memory_suite = suite == "memory-comparison";
    if !category_suite && !memory_suite && suite != "standard" && suite != "in_tree_only" {
        return Err(format!(
            "unknown benchmark suite '{suite}' (expected standard, in_tree_only, memory-comparison, or one of {})",
            datasets::CATEGORY_SUITES.join("|")
        ));
    }
    if category_suite && out_dir.is_none() {
        out_dir = Some(PathBuf::from(datasets::DEFAULT_RUNS_ROOT).join(&suite));
    }
    if memory_suite && out_dir.is_none() {
        let base = PathBuf::from(datasets::DEFAULT_RUNS_ROOT).join("memory-comparison");
        out_dir = Some(if memory_pilot {
            base.join("pilot")
        } else {
            base
        });
    }

    if live && profile_path.is_none() {
        return Err(
            "--live run requires --profile <path> (e.g. --profile lab/profiles/<name>.toml)"
                .to_owned(),
        );
    }
    let loaded_profile = match &profile_path {
        Some(p) => {
            Some(monoagens_lab::memory::RunProfile::load_from_path(p).map_err(|e| e.to_string())?)
        }
        None => None,
    };

    let loaded_prior = load_prior_or_default(prior_path.as_deref()).map_err(|e| e.to_string())?;
    let pareto_input: Option<ParetoScorecardInput> = match pareto_input_path {
        Some(p) => {
            ensure_not_sealed_path(&p).map_err(|e| e.to_string())?;
            let raw = fs::read_to_string(&p).map_err(|e| e.to_string())?;
            Some(serde_json::from_str(&raw).map_err(|e| e.to_string())?)
        }
        None => None,
    };

    let provider_secrets = if live || secrets_path.is_some() {
        datasets::load_lab_provider_secrets(secrets_path.as_deref()).map_err(|e| e.to_string())?
    } else {
        monoagens_lab::reason_seam::secrets::ProviderSecrets::default()
    };

    let resolved_model_endpoint = model_endpoint
        .clone()
        .or_else(|| provider_secrets.model_endpoint.clone());
    let resolved_judge_endpoint = judge_endpoint
        .clone()
        .or_else(|| provider_secrets.model_endpoint.clone());

    if memory_suite && only_ids.is_none() {
        only_ids = Some(vec!["locomo".to_owned(), "longmemeval".to_owned()]);
    }

    let mut extra_headers: Vec<(String, monoagens_lab::reason_seam::model::ExtraHeaderValue)> =
        Vec::new();
    let mut extra_header_names: Vec<String> = Vec::new();
    for spec in &extra_headers_spec {
        let Some((hdr_name, env_var)) = spec.split_once('=') else {
            return Err(format!(
                "invalid --extra-header '{spec}': expected NAME=ENV_VAR"
            ));
        };
        let hdr_name = hdr_name.trim();
        let env_var = env_var.trim();
        if hdr_name.is_empty() || env_var.is_empty() {
            return Err(format!(
                "invalid --extra-header '{spec}': NAME and ENV_VAR must be non-empty"
            ));
        }
        let hdr_val = env::var(env_var).map_err(|_| {
            format!("--extra-header '{hdr_name}': environment variable '{env_var}' is not set")
        })?;
        if hdr_val.trim().is_empty() {
            return Err(format!(
                "--extra-header '{hdr_name}': environment variable '{env_var}' is empty"
            ));
        }
        extra_header_names.push(hdr_name.to_owned());
        extra_headers.push((
            hdr_name.to_owned(),
            monoagens_lab::reason_seam::model::ExtraHeaderValue::Literal(SecretString::new(
                hdr_val,
            )),
        ));
    }

    let lab_ua = format!("monoagens-lab/{}", env!("CARGO_PKG_VERSION"));
    let provider_endpoint_host = resolved_model_endpoint
        .as_deref()
        .and_then(extract_endpoint_host);

    let live_probe_provider: Option<OpenAiModelProvider> = if live {
        if let Some(ep) = &resolved_model_endpoint {
            let key = model_api_key_env
                .as_deref()
                .and_then(|v| env::var(v).ok())
                .map(SecretString::new)
                .or_else(|| provider_secrets.model_api_key.clone());
            let mut cfg = OpenAiModelConfig::new(ep, key);
            cfg.reasoning_effort = reasoning_effort;
            cfg.extra_headers = extra_headers.clone();
            cfg.user_agent = Some(lab_ua.clone());
            cfg.max_retries = 5;
            cfg.default_timeout_ms = 300_000;
            Some(OpenAiModelProvider::new(cfg)?)
        } else {
            None
        }
    } else {
        None
    };
    let live_judge_provider: Option<OpenAiModelProvider> = if live {
        if let Some(ep) = &resolved_judge_endpoint {
            let key = judge_api_key_env
                .as_deref()
                .and_then(|v| env::var(v).ok())
                .map(SecretString::new)
                .or_else(|| provider_secrets.model_api_key.clone());
            let mut cfg = OpenAiModelConfig::new(ep, key);
            cfg.reasoning_effort = judge_reasoning_effort;
            cfg.extra_headers = extra_headers;
            cfg.user_agent = Some(lab_ua);
            cfg.max_retries = 5;
            cfg.default_timeout_ms = 300_000;
            Some(OpenAiModelProvider::new(cfg)?)
        } else {
            None
        }
    } else {
        None
    };
    let judge_config = judge_model.map(|jm| datasets::JudgeConfig {
        endpoint_url: resolved_judge_endpoint.unwrap_or_default(),
        model: jm,
        prompt_version: judge_prompt_version,
        substitution_policy: judge_sub_policy,
    });

    // Configure EmbeddingGemma 2 semantic embedder for standard runs (`task 10.5`).
    let mut sidecar_guard: Option<SidecarProcessGuard> = None;
    let mut embedder_backend: Option<String> = None;
    let mut embedder_revision: Option<String> = None;
    let configured_embedder: Option<EmbeddingGemma2Embedder> = if (category_suite || memory_suite)
        && !allow_no_embedder
        && !preflight_only
    {
        if let Some(ep) = &embed_endpoint {
            let mpath = embed_model_path
                .clone()
                .unwrap_or_else(|| datasets_dir.join("embedder_models/embeddinggemma-2"));
            embedder_backend = Some(format!("SidecarHttp ({ep})"));
            let emb = EmbeddingGemma2Embedder::new(EmbeddingGemma2Config::local_http(
                ep,
                Some(mpath),
                60_000,
            ))
            .map_err(|e| {
                format!(
                    "failed to configure EmbeddingGemma 2 embedder at '{ep}': {e}. Run `monoagens-lab datasets pin embeddinggemma-2 && monoagens-lab datasets fetch --record-checksums embeddinggemma-2`"
                )
            })?;
            Some(emb)
        } else {
            let resolved_manifest =
                datasets::resolve_manifest_path(manifest_path.as_deref(), &datasets_dir);
            let registry = standard_benchmark_registry();
            let (manifest, _) = datasets::verify_required_datasets_with_manifest(
                &resolved_manifest,
                &datasets_dir,
                &registry,
                only_ids.as_deref(),
            )
            .map_err(|e| e.to_string())?;
            let emb_entry = manifest.get("embeddinggemma-2").ok_or_else(|| {
                "manifest is missing 'embeddinggemma-2' embedder_model entry; run `monoagens-lab datasets init` or update manifest".to_owned()
            })?;
            let verify = datasets::verify_manifest_entry(emb_entry, &datasets_dir)
                .map_err(|e| e.to_string())?;
            if !verify.verified {
                return Err(format!(
                    "standard benchmark requires verified EmbeddingGemma 2 weights under '{}' ({}). Run `monoagens-lab datasets pin embeddinggemma-2 && monoagens-lab datasets fetch --record-checksums embeddinggemma-2`",
                    datasets_dir.join(&emb_entry.relative_dir).display(),
                    verify.failures.join("; ")
                ));
            }
            embedder_revision = emb_entry.sources.first().map(|s| s.revision.clone());
            let model_dir = embed_model_path
                .clone()
                .unwrap_or_else(|| datasets_dir.join(&emb_entry.relative_dir));
            let (guard, sidecar_url, device_info) = start_embeddinggemma2_sidecar(&model_dir)?;
            sidecar_guard = Some(guard);
            embedder_backend = Some(format!("SidecarHttp (uv offline, {device_info})"));
            let emb = EmbeddingGemma2Embedder::new(EmbeddingGemma2Config::local_http(
                sidecar_url,
                Some(model_dir),
                60_000,
            ))
            .map_err(|e| format!("failed to connect to EmbeddingGemma 2 sidecar: {e}"))?;
            emb.embed(&EmbedInput::query("semantic embedder readiness check"))
                .map_err(|e| format!("EmbeddingGemma 2 readiness check failed: {e}"))?;
            Some(emb)
        }
    } else {
        None
    };

    if memory_suite {
        if !live {
            return Err("memory-comparison requires --live".to_owned());
        }
        let phase = if memory_pilot {
            monoagens_lab::memory::MemoryPhase::Pilot
        } else {
            monoagens_lab::memory::MemoryPhase::parse(&memory_phase).map_err(|e| e.to_string())?
        };
        let run_dir = out_dir
            .clone()
            .ok_or_else(|| "memory-comparison requires an output directory".to_owned())?;
        let proxy_api_key = model_api_key_env
            .as_deref()
            .and_then(|name| env::var(name).ok())
            .filter(|value| !value.trim().is_empty())
            .map(SecretString::new)
            .or_else(|| provider_secrets.model_api_key.clone());
        if proxy_api_key.is_none() {
            return Err(
                "memory-comparison needs an API key (--model-api-key-env or MONOAGENS_MODEL_API_KEY)"
                    .to_owned(),
            );
        }
        let profile = loaded_profile
            .as_ref()
            .ok_or_else(|| "--live memory-comparison requires --profile <path>".to_owned())?;
        let proxy_cfg = monoagens_lab::memory::MeteringProxyConfig::from_profile(
            profile,
            proxy_api_key,
            run_dir.join("ledger.jsonl"),
        )
        .map_err(|e| e.to_string())?;
        let proxy =
            monoagens_lab::memory::MeteringProxy::start(proxy_cfg).map_err(|e| e.to_string())?;
        let embedder: Option<std::sync::Arc<dyn Embedder>> =
            configured_embedder.map(|e| std::sync::Arc::new(e) as std::sync::Arc<dyn Embedder>);
        let memory_opts = monoagens_lab::memory::MemorySuiteOptions {
            datasets_dir: datasets_dir.clone(),
            manifest_path: manifest_path.clone(),
            run_dir: run_dir.clone(),
            prior: loaded_prior.prior.clone(),
            seed,
            phase,
            live,
            profile: Some(profile.clone()),
            max_cost_usd,
            provider_endpoint: Some(proxy.chat_completions_url()),
            judge_endpoint: Some(proxy.chat_completions_url()),
            probe_provider: None,
            judge: None,
            embedder,
            write_report: true,
            only_datasets: None,
            max_items_per_dataset: None,
            reuse_from: memory_reuse_from.clone(),
        };
        let outcome =
            monoagens_lab::memory::run_memory_suite(&memory_opts).map_err(|e| e.to_string())?;
        if let Some(pilot) = &outcome.pilot {
            println!(
                "memory pilot run_dir={} calls={} cost=${:.4} projected_rest=${:.2} wall_rest={:.0}s",
                outcome.run_dir.display(),
                pilot.calls,
                pilot.list_cost_usd,
                pilot.projected_rest_list_cost_usd,
                pilot.projected_rest_wall_clock_s
            );
        } else {
            println!(
                "memory phase 0 run_dir={} datasets={} status={} report={}",
                outcome.run_dir.display(),
                outcome.datasets.len(),
                outcome
                    .stopped_reason
                    .clone()
                    .unwrap_or_else(|| "complete".to_owned()),
                outcome
                    .report_md
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    let opts = datasets::BenchmarkExecutionOptions {
        manifest_path: manifest_path.clone(),
        only_ids,
        seed,
        live,
        max_cost_usd,
        out_dir: out_dir.clone(),
        runs_root: runs_root.clone(),
        pilot_prefix_n: pilot_items,
        preflight_only,
        require_semantic_embedder: category_suite && !allow_no_embedder && !preflight_only,
        probe_model_name: model_name,
        probe_provider: live_probe_provider
            .as_ref()
            .map(|p| p as &dyn monoagens_lab::reason_seam::model::ModelProvider),
        judge_config,
        judge_provider: live_judge_provider
            .as_ref()
            .map(|p| p as &dyn monoagens_lab::reason_seam::model::ModelProvider),
        embedder: configured_embedder
            .as_ref()
            .map(|e| e as &dyn monoagens_lab::reason_seam::embed::Embedder),
        reasoning_effort: reasoning_effort.map(|e| e.to_string()),
        judge_reasoning_effort: judge_reasoning_effort.map(|e| e.to_string()),
        extra_header_names,
        provider_endpoint_host,
        embedder_backend,
        embedder_revision,
        ..datasets::BenchmarkExecutionOptions::default()
    };

    let report = run_benchmark_suite_with_options(
        &suite,
        &datasets_dir,
        pareto_input.as_ref(),
        pareto_flag,
        &loaded_prior.prior,
        &opts,
    )
    .map_err(|e| e.to_string())?;

    if live && let Some(dir) = out_dir.as_deref() {
        let telem = serde_json::json!({
            "soul": live_probe_provider.as_ref().map(|p| p.telemetry_snapshot()),
            "judge": live_judge_provider.as_ref().map(|p| p.telemetry_snapshot()),
        });
        if let Ok(bytes) = serde_json::to_vec_pretty(&telem) {
            let _ = fs::write(dir.join("provider_telemetry.json"), bytes);
        }
    }
    drop(sidecar_guard);

    // Each category suite owns its run directory and report; `standard` only indexes.
    let auto_report = category_suite && !preflight_only;
    if (report_flag || auto_report)
        && let Some(dir) = out_dir.as_deref()
    {
        datasets::generate_benchmark_report(&datasets::ReportOptions {
            run_dir: dir.to_path_buf(),
            out_dir: None,
            manifest_path,
            extra_notes: report_notes,
            skip_python_charts: false,
            suite: category_suite.then(|| suite.clone()),
            label: None,
            appendix_markdown: None,
        })
        .map_err(|e| e.to_string())?;
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "benchmark suite={} partial={} scorecard_rows={} tool_rag_points={} arm_t_restart_ok={}",
            report.suite,
            report.partial,
            report.scorecard.len(),
            report.tool_rag_scaling.len(),
            report.arm_t_restart_parity_ok
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn run_datasets_cmd(args: &[String]) -> Result<ExitCode, String> {
    let Some(sub) = args.first().map(String::as_str) else {
        return Err(
            "datasets requires a subcommand: init | list | pin | fetch | verify".to_owned(),
        );
    };
    match sub {
        "init" => run_datasets_init_cmd(&args[1..]),
        "list" => run_datasets_list_cmd(&args[1..]),
        "pin" => run_datasets_pin_cmd(&args[1..]),
        "fetch" => run_datasets_fetch_cmd(&args[1..]),
        "verify" => run_datasets_verify_cmd(&args[1..]),
        other => Err(format!("unknown datasets subcommand '{other}'")),
    }
}

fn run_datasets_init_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut manifest_path = PathBuf::from(datasets::DEFAULT_LOCAL_MANIFEST_PATH);
    let mut template_path: Option<PathBuf> = None;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                i += 1;
                manifest_path = PathBuf::from(args.get(i).ok_or("--manifest requires a path")?);
            }
            "--template" => {
                i += 1;
                template_path = Some(PathBuf::from(
                    args.get(i).ok_or("--template requires a path")?,
                ));
            }
            "--json" => json = true,
            "--force" | "-f" => {
                return Err(
                    "--force is not allowed for datasets init; refusing to overwrite an existing local manifest"
                        .to_owned(),
                );
            }
            other => return Err(format!("unknown argument '{other}' for datasets init")),
        }
        i += 1;
    }

    let resolved_template = datasets::resolve_template_manifest_path(template_path.as_deref());
    let entry_count =
        datasets::init_local_manifest_from_template(&resolved_template, &manifest_path)
            .map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "initialized": true,
                "manifest_path": manifest_path.to_string_lossy(),
                "template_path": resolved_template.to_string_lossy(),
                "entries": entry_count,
            }))
            .map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "initialized {} from {} ({} entries); edit license.status locally when cleared (never commit)",
            manifest_path.display(),
            resolved_template.display(),
            entry_count
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn push_dataset_ids(out: &mut Vec<String>, arg: &str) {
    for part in arg.split(',') {
        let trimmed = part.trim();
        if !trimmed.is_empty() {
            out.push(trimmed.to_owned());
        }
    }
}

fn run_datasets_pin_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut manifest_path = PathBuf::from("lab/datasets.manifest.toml");
    let mut json = false;
    let mut ids = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                i += 1;
                manifest_path = PathBuf::from(args.get(i).ok_or("--manifest requires a path")?);
            }
            "--json" => json = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown argument '{other}' for datasets pin"));
            }
            id => push_dataset_ids(&mut ids, id),
        }
        i += 1;
    }

    let registry = standard_benchmark_registry();
    let results = datasets::pin_datasets(
        &manifest_path,
        &registry,
        &ids,
        &datasets::PinEndpointOverrides::default(),
    )
    .map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "pinned": results }))
                .map_err(|e| e.to_string())?
        );
    } else {
        for r in &results {
            println!(
                "pinned {} | sources_updated={} | files_updated={} | license_status={}",
                r.id, r.updated_sources, r.updated_files, r.license_status
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_datasets_fetch_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut manifest_path = PathBuf::from("lab/datasets.manifest.toml");
    let mut datasets_dir = PathBuf::from("lab/datasets");
    let mut record_checksums = false;
    let mut json = false;
    let mut ids = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                i += 1;
                manifest_path = PathBuf::from(args.get(i).ok_or("--manifest requires a path")?);
            }
            "--datasets-dir" => {
                i += 1;
                datasets_dir = PathBuf::from(args.get(i).ok_or("--datasets-dir requires a path")?);
            }
            "--record-checksums" => record_checksums = true,
            "--json" => json = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown argument '{other}' for datasets fetch"));
            }
            id => push_dataset_ids(&mut ids, id),
        }
        i += 1;
    }

    let registry = standard_benchmark_registry();
    let results = datasets::fetch_datasets(
        &manifest_path,
        &datasets_dir,
        &registry,
        &ids,
        record_checksums,
        &datasets::FetchEndpointOverrides::default(),
    )
    .map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "fetched": results }))
                .map_err(|e| e.to_string())?
        );
    } else {
        for r in &results {
            println!(
                "fetched {} -> {} ({} files, owner_checksums_recorded={})",
                r.id, r.target_dir, r.files_fetched, r.recorded_owner_checksums
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_datasets_verify_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut manifest_path = PathBuf::from("lab/datasets.manifest.toml");
    let mut datasets_dir = PathBuf::from("lab/datasets");
    let mut json = false;
    let mut ids = Vec::new();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                i += 1;
                manifest_path = PathBuf::from(args.get(i).ok_or("--manifest requires a path")?);
            }
            "--datasets-dir" => {
                i += 1;
                datasets_dir = PathBuf::from(args.get(i).ok_or("--datasets-dir requires a path")?);
            }
            "--json" => json = true,
            other if other.starts_with('-') => {
                return Err(format!("unknown argument '{other}' for datasets verify"));
            }
            id => push_dataset_ids(&mut ids, id),
        }
        i += 1;
    }

    let registry = standard_benchmark_registry();
    let report = datasets::verify_datasets(&manifest_path, &datasets_dir, &registry, &ids)
        .map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        for r in &report.results {
            if r.failures.is_empty() {
                println!("{} | {}", r.id, r.status);
            } else {
                println!("{} | {} | {}", r.id, r.status, r.failures.join("; "));
            }
        }
    }

    if report.all_verified {
        Ok(ExitCode::SUCCESS)
    } else {
        for r in &report.results {
            if !r.verified && r.status != "unavailable" {
                eprintln!("unverified dataset '{}': {}", r.id, r.failures.join("; "));
            }
        }
        Ok(ExitCode::from(1))
    }
}

fn run_datasets_list_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut manifest_path = PathBuf::from("lab/datasets.manifest.toml");
    let mut datasets_dir = PathBuf::from("lab/datasets");
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                i += 1;
                manifest_path = PathBuf::from(args.get(i).ok_or("--manifest requires a path")?);
            }
            "--datasets-dir" => {
                i += 1;
                datasets_dir = PathBuf::from(args.get(i).ok_or("--datasets-dir requires a path")?);
            }
            "--json" => json = true,
            other => return Err(format!("unknown argument '{other}' for datasets list")),
        }
        i += 1;
    }

    let registry = standard_benchmark_registry();
    let manifest = datasets::load_and_validate_manifest(&manifest_path, &registry)
        .map_err(|e| e.to_string())?;
    let rows =
        datasets::describe_datasets_status(&manifest, &datasets_dir).map_err(|e| e.to_string())?;

    if json {
        let payload = serde_json::json!({
            "schema_version": manifest.schema_version,
            "manifest_path": manifest_path.to_string_lossy(),
            "datasets": rows,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?
        );
    } else {
        for row in &rows {
            let sources_str = if row.sources.is_empty() {
                "none".to_owned()
            } else {
                row.sources
                    .iter()
                    .map(|s| {
                        let role = s.role.as_deref().unwrap_or("primary");
                        format!("{role}:{}:{}@{}", s.kind, s.location, s.revision)
                    })
                    .collect::<Vec<_>>()
                    .join("; ")
            };
            let pin_flag = if row.has_pending_pin {
                format!(" pending_pin=[{}]", row.pending_pin_items.join(","))
            } else {
                String::new()
            };
            let csum_flag = if row.missing_owner_checksums {
                " missing_owner_checksums"
            } else {
                ""
            };
            println!(
                "{} | kind={} | domain={:?} | lane={} | avail={} | lic={} ({}) | contam={} | verify={}{}{} | sources=[{}]",
                row.id,
                row.kind,
                row.domain,
                row.lane,
                row.availability,
                row.license_spdx,
                row.license_status,
                row.contamination_risk,
                row.verification_state,
                pin_flag,
                csum_flag,
                sources_str,
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_pareto_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut input_path: Option<PathBuf> = None;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--input" => {
                i += 1;
                input_path = Some(PathBuf::from(args.get(i).ok_or("--input requires a path")?));
            }
            "--json" => json = true,
            other => return Err(format!("unknown argument '{other}' for pareto")),
        }
        i += 1;
    }

    let path = input_path.ok_or("pareto requires --input <scorecard.json>")?;
    ensure_not_sealed_path(&path).map_err(|e| e.to_string())?;
    let raw = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let input: ParetoScorecardInput = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    let report = compute_convex_pareto(&input);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "pareto frontier_count={} co_primary_passed={} holm_rejected={}",
            report.undominated_frontier.len(),
            report.co_primary_ni_and_cost_passed,
            report
                .holm_results
                .iter()
                .filter(|c| c.rejected_null)
                .count()
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn run_report_cmd(args: &[String]) -> Result<ExitCode, String> {
    let mut run_dir: Option<PathBuf> = None;
    let mut out_dir: Option<PathBuf> = None;
    let mut manifest_path: Option<PathBuf> = None;
    let mut notes: Vec<String> = Vec::new();
    let mut label: Option<String> = None;
    let mut appendix_path: Option<PathBuf> = None;
    let mut skip_python_charts = false;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--run" => {
                i += 1;
                run_dir = Some(PathBuf::from(
                    args.get(i).ok_or("--run requires a directory path")?,
                ));
            }
            "--out" => {
                i += 1;
                out_dir = Some(PathBuf::from(
                    args.get(i).ok_or("--out requires a directory path")?,
                ));
            }
            "--manifest" => {
                i += 1;
                manifest_path = Some(PathBuf::from(
                    args.get(i).ok_or("--manifest requires a path")?,
                ));
            }
            "--note" => {
                i += 1;
                notes.push(args.get(i).ok_or("--note requires text")?.clone());
            }
            "--label" => {
                i += 1;
                label = Some(args.get(i).ok_or("--label requires text")?.clone());
            }
            "--appendix" => {
                i += 1;
                appendix_path = Some(PathBuf::from(
                    args.get(i).ok_or("--appendix requires a path")?,
                ));
            }
            "--no-python-charts" => skip_python_charts = true,
            "--json" => json = true,
            other => return Err(format!("unknown argument '{other}' for report")),
        }
        i += 1;
    }

    let run_dir = run_dir.ok_or("report requires --run <dir>")?;
    let appendix_markdown = match appendix_path {
        Some(path) => Some(
            fs::read_to_string(&path)
                .map_err(|e| format!("failed to read --appendix '{}': {e}", path.display()))?,
        ),
        None => None,
    };
    let summary = datasets::generate_benchmark_report(&datasets::ReportOptions {
        run_dir,
        out_dir,
        manifest_path,
        extra_notes: notes,
        skip_python_charts,
        suite: None,
        label,
        appendix_markdown,
    })
    .map_err(|e| e.to_string())?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&summary).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "report generated at {} (records={}, csvs={}, charts={})",
            summary.report_dir,
            summary.run_records_loaded,
            summary.csv_files.len(),
            summary.chart_files.len()
        );
    }
    Ok(ExitCode::SUCCESS)
}

struct SidecarProcessGuard {
    child: Child,
}

impl Drop for SidecarProcessGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn start_embeddinggemma2_sidecar(
    model_dir: &std::path::Path,
) -> Result<(SidecarProcessGuard, String, String), String> {
    let ws_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = ws_root.join("lab/adapters/eval_helpers/embeddinggemma2_sidecar.py");
    if !script.is_file() {
        return Err(format!(
            "EmbeddingGemma 2 sidecar script not found at '{}'",
            script.display()
        ));
    }
    let py = env::var("MONOAGENS_ADAPTER_PYTHON").unwrap_or_else(|_| {
        let venv_py = ws_root.join("lab/adapters/.venv/bin/python");
        if venv_py.is_file() {
            venv_py.to_string_lossy().into_owned()
        } else {
            "python3".to_owned()
        }
    });

    let mut child = Command::new(&py)
        .arg(&script)
        .arg("--model-path")
        .arg(model_dir)
        .arg("--port")
        .arg("0")
        .env("HF_HUB_OFFLINE", "1")
        .env("TRANSFORMERS_OFFLINE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn EmbeddingGemma 2 sidecar via '{py}': {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "sidecar stdout unavailable".to_owned())?;
    let mut reader = BufReader::new(stdout);
    let mut first_line = String::new();
    reader
        .read_line(&mut first_line)
        .map_err(|e| format!("failed reading EmbeddingGemma 2 sidecar startup line: {e}"))?;

    if !first_line.starts_with("READY ") {
        let _ = child.kill();
        let out = child.wait_with_output().ok();
        let stderr = out
            .map(|o| String::from_utf8_lossy(&o.stderr).into_owned())
            .unwrap_or_default();
        return Err(format!(
            "EmbeddingGemma 2 sidecar failed to start (stdout='{}', stderr='{}')",
            first_line.trim(),
            stderr.trim()
        ));
    }

    let rest = first_line.trim().strip_prefix("READY ").unwrap_or("");
    let mut parts = rest.splitn(2, ' ');
    let url = parts.next().unwrap_or("").to_owned();
    let device_info = parts.next().unwrap_or("unknown").to_owned();
    if url.is_empty() {
        let _ = child.kill();
        return Err("EmbeddingGemma 2 sidecar returned empty URL".to_owned());
    }
    Ok((SidecarProcessGuard { child }, url, device_info))
}

fn extract_endpoint_host(endpoint: &str) -> Option<String> {
    let trimmed = endpoint.trim();
    let after_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))?;
    let scheme = if trimmed.starts_with("https://") {
        "https://"
    } else {
        "http://"
    };
    let host = after_scheme.split('/').next()?.trim();
    if host.is_empty() {
        None
    } else {
        Some(format!("{scheme}{host}"))
    }
}

fn usage() -> String {
    "Usage:\n  monoagens-lab run --scenario <path> [--prior <path>] [--seed <u64>] [--json] [--record-out <path>]\n  monoagens-lab suite [--root <dir>] [--set development|validation] [--prior <path>] [--seeds 1,2] [--json]\n  monoagens-lab compare (--scenario <path> | --set development|validation [--root <dir>]) --mechanism <id> [--prior <path>] [--seeds 1,2,3,4,5] [--out-dir <dir>] [--json]\n  monoagens-lab replay --scenario <path> [--prior <path>] [--seed <u64>] [--json]\n  monoagens-lab baseline --adapter <slug> --scenario <path> [--prior <path>] [--seed <u64>] [--sealed] [--timeout-s <u64>] [--json]\n  monoagens-lab benchmark --suite <standard|in_tree_only|memory|prospective|timing|group|tom|indonesian|security> [--datasets-dir <path>] [--prior <path>] [--pareto] [--pareto-input <path>] [--report] [--json]\n  monoagens-lab report --run <dir> [--out <dir>] [--manifest <path>] [--note <text>] [--json]\n  monoagens-lab datasets <init|list|pin|fetch|verify> [ids...] [--manifest <path>] [--datasets-dir <path>] [--record-checksums] [--json]\n  monoagens-lab pareto --input <scorecard.json> [--json]\n  monoagens-lab mechanisms [--json]\n  monoagens-lab test-body [--state-dir <path>]".to_owned()
}

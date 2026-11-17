//! `monoagens-lab` CLI binary for running scenarios, suites, ablations, replay, and mechanisms.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io::{self, BufWriter};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use monoagens_core::{MechanismId, MemStore, ReplayMode, Switches, brain_map_registry};
use monoagens_lab::reason_seam::model::{OpenAiModelConfig, OpenAiModelProvider};
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
    let mut seed: u64 = 1;
    let mut live = false;
    let mut live_full_history = false;
    let mut max_cost_usd: Option<f64> = None;
    let mut model_endpoint: Option<String> = None;
    let mut model_name = "scripted".to_owned();
    let mut model_api_key_env: Option<String> = None;
    let mut judge_endpoint: Option<String> = None;
    let mut judge_model: Option<String> = None;
    let mut judge_api_key_env: Option<String> = None;
    let mut judge_prompt_version = "v1".to_owned();
    let mut judge_sub_policy = datasets::JudgeSubstitutionPolicy::RenameMetric;
    let mut secrets_path: Option<PathBuf> = None;
    let mut json = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--suite" => {
                i += 1;
                suite = args
                    .get(i)
                    .ok_or("--suite requires 'standard' or 'in_tree_only'")?
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
            "--live-full-history" => live_full_history = true,
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
            "--json" => json = true,
            other => return Err(format!("unknown argument '{other}' for benchmark")),
        }
        i += 1;
    }

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

    let live_probe_provider: Option<OpenAiModelProvider> = if live {
        if let Some(ep) = &resolved_model_endpoint {
            let key = model_api_key_env
                .as_deref()
                .and_then(|v| env::var(v).ok())
                .map(SecretString::new)
                .or_else(|| provider_secrets.model_api_key.clone());
            Some(OpenAiModelProvider::new(OpenAiModelConfig::new(ep, key))?)
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
            Some(OpenAiModelProvider::new(OpenAiModelConfig::new(ep, key))?)
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

    let opts = datasets::BenchmarkExecutionOptions {
        manifest_path,
        only_ids,
        seed,
        live,
        live_full_history,
        max_cost_usd,
        out_dir,
        probe_model_name: model_name,
        probe_provider: live_probe_provider
            .as_ref()
            .map(|p| p as &dyn monoagens_lab::reason_seam::model::ModelProvider),
        judge_config,
        judge_provider: live_judge_provider
            .as_ref()
            .map(|p| p as &dyn monoagens_lab::reason_seam::model::ModelProvider),
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

fn usage() -> String {
    "Usage:\n  monoagens-lab run --scenario <path> [--prior <path>] [--seed <u64>] [--json] [--record-out <path>]\n  monoagens-lab suite [--root <dir>] [--set development|validation] [--prior <path>] [--seeds 1,2] [--json]\n  monoagens-lab compare (--scenario <path> | --set development|validation [--root <dir>]) --mechanism <id> [--prior <path>] [--seeds 1,2,3,4,5] [--out-dir <dir>] [--json]\n  monoagens-lab replay --scenario <path> [--prior <path>] [--seed <u64>] [--json]\n  monoagens-lab baseline --adapter <slug> --scenario <path> [--prior <path>] [--seed <u64>] [--sealed] [--timeout-s <u64>] [--json]\n  monoagens-lab benchmark --suite <standard|in_tree_only> [--datasets-dir <path>] [--prior <path>] [--pareto] [--pareto-input <path>] [--json]\n  monoagens-lab datasets <init|list|pin|fetch|verify> [ids...] [--manifest <path>] [--datasets-dir <path>] [--record-checksums] [--json]\n  monoagens-lab pareto --input <scorecard.json> [--json]\n  monoagens-lab mechanisms [--json]\n  monoagens-lab test-body [--state-dir <path>]".to_owned()
}

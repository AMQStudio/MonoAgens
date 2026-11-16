//! `monoagens-lab` CLI binary for running scenarios, suites, ablations, replay, and mechanisms.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::io::{self, BufWriter};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use monoagens_core::{MechanismId, MemStore, ReplayMode, Switches, brain_map_registry};
use monoagens_lab::{
    BaselineKind, ParetoScorecardInput, Scenario, ScenarioSet, ScriptedActionRule, TestBody,
    TestBodySpec, aggregate_metrics_across_seeds, compute_convex_pareto, ensure_not_sealed_path,
    format_metrics_table, load_scenarios_from_set, replay_events_offline,
    run_ablation_compare_scenarios, run_baseline_adapter, run_benchmark_suite, run_scenario,
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
    let mut prior_path: Option<PathBuf> = None;
    let mut pareto_input_path: Option<PathBuf> = None;
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
            "--prior" => {
                i += 1;
                prior_path = Some(PathBuf::from(args.get(i).ok_or("--prior requires a path")?));
            }
            "--pareto" => {
                // Use --pareto-input <path> if provided; otherwise compute only suite checks.
            }
            "--pareto-input" => {
                i += 1;
                pareto_input_path = Some(PathBuf::from(
                    args.get(i).ok_or("--pareto-input requires a path")?,
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

    let report = run_benchmark_suite(
        &suite,
        &datasets_dir,
        pareto_input.as_ref(),
        &loaded_prior.prior,
    )
    .map_err(|e| e.to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "benchmark suite={} datasets_registered={} tool_rag_points={} arm_t_restart_ok={}",
            report.suite,
            report.dataset_specs.len(),
            report.tool_rag_scaling.len(),
            report.arm_t_restart_parity_ok
        );
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
    "Usage:\n  monoagens-lab run --scenario <path> [--prior <path>] [--seed <u64>] [--json] [--record-out <path>]\n  monoagens-lab suite [--root <dir>] [--set development|validation] [--prior <path>] [--seeds 1,2] [--json]\n  monoagens-lab compare (--scenario <path> | --set development|validation [--root <dir>]) --mechanism <id> [--prior <path>] [--seeds 1,2,3,4,5] [--out-dir <dir>] [--json]\n  monoagens-lab replay --scenario <path> [--prior <path>] [--seed <u64>] [--json]\n  monoagens-lab baseline --adapter <slug> --scenario <path> [--prior <path>] [--seed <u64>] [--sealed] [--timeout-s <u64>] [--json]\n  monoagens-lab benchmark --suite <standard|in_tree_only> [--datasets-dir <path>] [--prior <path>] [--pareto] [--pareto-input <path>] [--json]\n  monoagens-lab pareto --input <scorecard.json> [--json]\n  monoagens-lab mechanisms [--json]\n  monoagens-lab test-body [--state-dir <path>]".to_owned()
}

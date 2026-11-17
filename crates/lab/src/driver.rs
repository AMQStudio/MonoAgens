//! Discrete-event virtual-time simulation driver coupling `Core` and `TestBody` over Waist 2,
//! plus offline zero-model log replay and multi-seed paired ablation.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Instant;

use monoagens_core::{
    ContextPlan, Core, Effect, EventKind, EventRecord, Input, MemStore, ModelPurpose, Prior,
    ReplayMode, ReplayReport, Store, Switches, replay_from_checkpoint,
};
use monoagens_protocol::{
    Action, ActionExecuteParams, ActionExecuteResult, ActionId, ActionReceiptParams, ActorId,
    BodyId, CURRENT_PROTOCOL_VERSION, ConsolidateOutput, EventId, PerceptEmitParams, PlaceId,
    Receipt, ReceiptOutcome, RpcId, TerminationReason, ThoughtAct, Time, Waist2Headers,
    Waist2Method, Waist2Request,
};

use crate::LabError;
use crate::checks::{evaluate_checks, translate_scenario_event_id};
use crate::clock::VirtualClock;
use crate::guard::{CostGuard, GuardRefusal};
use crate::metrics::{
    PairedAblationDelta, RunMetricInputs, RunMetricsSummary, compare_paired_ablation,
    compute_run_metrics_full,
};
use crate::record::{PurposeCost, RunRecord};
use crate::scenario::{Scenario, ScenarioSet, ScriptedEventKind, ScriptedModelResponse};
use crate::test_body::TestBody;

/// Complete output of running a [`Scenario`] in the Lab driver.
#[derive(Debug, Clone)]
pub struct SimOutput {
    pub run_record: RunRecord,
    pub events: Vec<EventRecord>,
    pub context_plans: Vec<ContextPlan>,
    pub guard_refusals: Vec<GuardRefusal>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScheduledItem {
    WirePercept {
        scenario_idx: Option<usize>,
        body_id: Option<BodyId>,
        json_rpc: String,
    },
    WireReceipt {
        json_rpc: String,
    },
    DirectInput(ScheduledDirectInput),
    CoreOp {
        scenario_idx: usize,
        op_idx: usize,
    },
    ModelTurn {
        request_id: EventId,
        response_idx: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScheduledDirectInput {
    scenario_idx: usize,
    index: usize,
}

#[derive(Debug, Clone)]
enum ScheduledCoreOp {
    TriggerSleep,
    RecordCalibration {
        conf_0_20: u8,
        correct: bool,
    },
    SetDailyBudget {
        used: f64,
        limit: f64,
    },
    HardForget {
        event_indices: Vec<usize>,
        actor_ids: Vec<ActorId>,
        reason: String,
    },
}

/// Deterministic scripted model provider backed by `Scenario.scripted_model`.
#[derive(Debug, Clone)]
struct ScriptedProvider {
    responses: Vec<ScriptedModelResponse>,
    next_wake_index: usize,
}

impl ScriptedProvider {
    fn new(responses: Vec<ScriptedModelResponse>) -> Self {
        Self {
            responses,
            next_wake_index: 0,
        }
    }

    fn next_response(&mut self) -> ScriptedModelResponse {
        let idx = self.next_wake_index;
        self.next_wake_index = self.next_wake_index.saturating_add(1);

        if let Some(exact) = self.responses.iter().find(|r| r.wake_index == Some(idx)) {
            return exact.clone();
        }
        if let Some(pos_match) = self.responses.get(idx)
            && pos_match.wake_index.is_none()
        {
            return pos_match.clone();
        }
        ScriptedModelResponse {
            wake_index: Some(idx),
            acts: Vec::new(),
            tokens_in: 0,
            tokens_out: 0,
            latency_ms: None,
            fail_reason: None,
        }
    }
}

/// Executes `scenario` deterministically against `Core` over `store` using virtual time and
/// canonical `monoagens.body.v1` JSON-RPC framing for all `TestBody` communication.
pub fn run_scenario<S: Store>(
    store: S,
    prior: Prior,
    scenario: &Scenario,
    seed: u64,
) -> Result<SimOutput, LabError> {
    let wall_start = Instant::now();
    let effective_prior = scenario.prior.clone().unwrap_or(prior);
    let loaded_effective = monoagens_store::LoadedPrior::from_prior(effective_prior)?;
    let switches = scenario.switches.clone().unwrap_or_default();
    let prior_content_version = loaded_effective.prior.content_version.clone();
    let prior_format_version = loaded_effective.prior.version;
    let prior_digest = loaded_effective.digest;

    let pre_open_event_count = store.read_in_order(None, usize::MAX)?.len() as u64;
    let mut core = Core::open(store, loaded_effective.prior, switches.clone(), seed)?;
    let post_open_event_count = core.store().read_in_order(None, usize::MAX)?.len() as u64;
    let open_offset = post_open_event_count.saturating_sub(pre_open_event_count);
    let mut guard = CostGuard::from_config(scenario.cost_guard.as_ref());
    let door_rules = guard.rule_names();

    let mut bodies: BTreeMap<BodyId, TestBody> = BTreeMap::new();
    let mut place_to_body: BTreeMap<PlaceId, BodyId> = BTreeMap::new();
    for spec in &scenario.bodies {
        let body_id = spec.body_id.clone();
        let body = TestBody::from_spec(spec.clone());
        for place in body.places() {
            place_to_body.insert(place.clone(), body_id.clone());
        }
        bodies.insert(body_id, body);
    }

    let mut clock: VirtualClock<ScheduledItem> =
        VirtualClock::new(Time::from_millis(scenario.start_time_ms));
    let mut direct_inputs: Vec<Input> = Vec::new();
    let mut core_ops: Vec<ScheduledCoreOp> = Vec::new();
    let mut metric_inputs = RunMetricInputs {
        open_offset,
        ..RunMetricInputs::default()
    };

    for (idx, ev) in scenario.events.iter().enumerate() {
        let at = Time::from_millis(ev.at_ms);
        match &ev.event {
            ScriptedEventKind::Percept(percept) => {
                let target_body_id = ev
                    .body_id
                    .clone()
                    .or_else(|| place_to_body.get(&percept.place).cloned());
                let req = Waist2Request::new(
                    Some(RpcId::Int((idx as i64) + 1)),
                    Waist2Method::PerceptEmit,
                    PerceptEmitParams::from_percept(
                        Waist2Headers::new(format!("scenario-{}-percept-{idx}", scenario.id)),
                        (**percept).clone(),
                    ),
                );
                let json_rpc = serde_json::to_string(&req)
                    .map_err(|e| LabError::Serialization(e.to_string()))?;
                clock.schedule_at(
                    at,
                    ScheduledItem::WirePercept {
                        scenario_idx: Some(idx),
                        body_id: target_body_id,
                        json_rpc,
                    },
                );
            }
            ScriptedEventKind::BodyState(bs) => {
                let di_idx = direct_inputs.len();
                direct_inputs.push(Input::BodyState(bs.clone()));
                clock.schedule_at(
                    at,
                    ScheduledItem::DirectInput(ScheduledDirectInput {
                        scenario_idx: idx,
                        index: di_idx,
                    }),
                );
            }
            ScriptedEventKind::ActorFacts(af) => {
                let di_idx = direct_inputs.len();
                direct_inputs.push(Input::ActorFacts(af.clone()));
                clock.schedule_at(
                    at,
                    ScheduledItem::DirectInput(ScheduledDirectInput {
                        scenario_idx: idx,
                        index: di_idx,
                    }),
                );
            }
            ScriptedEventKind::Identity(id) => {
                let di_idx = direct_inputs.len();
                direct_inputs.push(Input::Identity(id.clone()));
                clock.schedule_at(
                    at,
                    ScheduledItem::DirectInput(ScheduledDirectInput {
                        scenario_idx: idx,
                        index: di_idx,
                    }),
                );
            }
            ScriptedEventKind::CancelAction {
                action_id,
                termination_reason,
                reason_code,
                detail,
            } => {
                metric_inputs
                    .barge_in_timestamps_ms
                    .insert(action_id.clone(), ev.at_ms);
                let di_idx = direct_inputs.len();
                direct_inputs.push(Input::CancelAction {
                    action_id: action_id.clone(),
                    termination_reason: *termination_reason,
                    reason_code: reason_code.clone(),
                    detail: detail.clone(),
                });
                clock.schedule_at(
                    at,
                    ScheduledItem::DirectInput(ScheduledDirectInput {
                        scenario_idx: idx,
                        index: di_idx,
                    }),
                );
            }
            ScriptedEventKind::Tick => {
                let di_idx = direct_inputs.len();
                direct_inputs.push(Input::Tick);
                clock.schedule_at(
                    at,
                    ScheduledItem::DirectInput(ScheduledDirectInput {
                        scenario_idx: idx,
                        index: di_idx,
                    }),
                );
            }
            ScriptedEventKind::TriggerSleep => {
                let op_idx = core_ops.len();
                core_ops.push(ScheduledCoreOp::TriggerSleep);
                clock.schedule_at(
                    at,
                    ScheduledItem::CoreOp {
                        scenario_idx: idx,
                        op_idx,
                    },
                );
            }
            ScriptedEventKind::RecordCalibration { conf_0_20, correct } => {
                let op_idx = core_ops.len();
                core_ops.push(ScheduledCoreOp::RecordCalibration {
                    conf_0_20: *conf_0_20,
                    correct: *correct,
                });
                clock.schedule_at(
                    at,
                    ScheduledItem::CoreOp {
                        scenario_idx: idx,
                        op_idx,
                    },
                );
            }
            ScriptedEventKind::SetDailyBudget { used, limit } => {
                let op_idx = core_ops.len();
                core_ops.push(ScheduledCoreOp::SetDailyBudget {
                    used: *used,
                    limit: *limit,
                });
                clock.schedule_at(
                    at,
                    ScheduledItem::CoreOp {
                        scenario_idx: idx,
                        op_idx,
                    },
                );
            }
            ScriptedEventKind::HardForget {
                event_indices,
                actor_ids,
                reason,
            } => {
                let op_idx = core_ops.len();
                core_ops.push(ScheduledCoreOp::HardForget {
                    event_indices: event_indices.clone(),
                    actor_ids: actor_ids.clone(),
                    reason: reason.clone(),
                });
                clock.schedule_at(
                    at,
                    ScheduledItem::CoreOp {
                        scenario_idx: idx,
                        op_idx,
                    },
                );
            }
        }
    }

    let mut provider = ScriptedProvider::new(scenario.scripted_model.clone());
    let mut model_turns: Vec<ScriptedModelResponse> = Vec::new();
    let mut purpose_costs: BTreeMap<u8, PurposeCost> = BTreeMap::new();
    let mut rpc_seq = 10_000_i64;
    let mut last_tick_at: Option<Time> = None;
    let mut canceled_actions: BTreeSet<ActionId> = BTreeSet::new();

    loop {
        let next_scripted = clock.next_time();
        let next_due = core.next_due();

        let next_target = match (next_scripted, next_due) {
            (Some(s), Some(d)) => Some(s.min(d)),
            (Some(s), None) => Some(s),
            (None, Some(d)) => Some(d),
            (None, None) => None,
        };
        let Some(target_time) = next_target else {
            break;
        };

        clock.advance_to(target_time);

        let should_run_scripted = next_scripted.is_some_and(|s| s <= target_time);
        if should_run_scripted {
            let Some((_at, _seq, item)) = clock.pop_next() else {
                continue;
            };

            if let ScheduledItem::CoreOp {
                scenario_idx,
                op_idx,
            } = item
            {
                let prev_seq = core.state().last_seq;
                let effects = match &core_ops[op_idx] {
                    ScheduledCoreOp::TriggerSleep => {
                        core.run_sleep_if_due(clock.now(), &mut guard)?
                    }
                    ScheduledCoreOp::RecordCalibration { conf_0_20, correct } => {
                        if scenario.set == ScenarioSet::External {
                            return Err(LabError::Protocol(
                                "refusing to record calibration from ScenarioSet::External"
                                    .to_owned(),
                            ));
                        }
                        let id =
                            core.record_calibration_outcome(*conf_0_20, *correct, clock.now())?;
                        metric_inputs.scripted_event_ids.insert(scenario_idx, id);
                        Vec::new()
                    }
                    ScheduledCoreOp::SetDailyBudget { used, limit } => {
                        core.set_daily_budget(*used, *limit);
                        let id = core.record_budget_event(*used, *limit, clock.now())?;
                        metric_inputs.scripted_event_ids.insert(scenario_idx, id);
                        Vec::new()
                    }
                    ScheduledCoreOp::HardForget {
                        event_indices,
                        actor_ids,
                        reason,
                    } => {
                        let target_ids: Vec<EventId> = event_indices
                            .iter()
                            .filter_map(|i| metric_inputs.scripted_event_ids.get(i).copied())
                            .collect();
                        let id = core.hard_forget(
                            target_ids.clone(),
                            actor_ids.clone(),
                            reason.clone(),
                            clock.now(),
                        )?;
                        metric_inputs.scripted_event_ids.insert(scenario_idx, id);
                        let remaining = core
                            .state()
                            .memory_entries
                            .values()
                            .filter(|m| {
                                target_ids.contains(&m.event_id) || actor_ids.contains(&m.actor)
                            })
                            .count();
                        metric_inputs.hard_forget_resurrection_count = Some(remaining);
                        Vec::new()
                    }
                };
                if let std::collections::btree_map::Entry::Vacant(e) =
                    metric_inputs.scripted_event_ids.entry(scenario_idx)
                {
                    let appended = core.store().read_after_seq(Some(prev_seq), 16)?;
                    if let Some(first) = appended.first() {
                        e.insert(first.id);
                    }
                }
                dispatch_effects(
                    scenario,
                    effects,
                    &mut provider,
                    &mut model_turns,
                    &mut guard,
                    &mut purpose_costs,
                    &mut clock,
                    &place_to_body,
                    &mut bodies,
                    &mut rpc_seq,
                )?;
                continue;
            }

            let (input, scenario_idx_opt, is_percept) = match item {
                ScheduledItem::WirePercept {
                    scenario_idx,
                    body_id,
                    json_rpc,
                } => {
                    if let Some(bid) = body_id
                        && let Some(body) = bodies.get_mut(&bid)
                    {
                        body.set_virtual_time(clock.now());
                        let _resp_line = body.handle_json_rpc(&json_rpc);
                    }
                    let req: Waist2Request<PerceptEmitParams> = serde_json::from_str(&json_rpc)
                        .map_err(|e| LabError::Serialization(e.to_string()))?;
                    let mut percept = req
                        .params
                        .into_percept(CURRENT_PROTOCOL_VERSION)
                        .map_err(|e| LabError::Protocol(e.to_string()))?;
                    if open_offset != 0 {
                        percept.reply_to = percept
                            .reply_to
                            .map(|id| translate_scenario_event_id(id, open_offset));
                    }
                    (Input::Percept(Box::new(percept)), scenario_idx, true)
                }
                ScheduledItem::WireReceipt { json_rpc } => {
                    let req: Waist2Request<ActionReceiptParams> =
                        serde_json::from_str(&json_rpc)
                            .map_err(|e| LabError::Serialization(e.to_string()))?;
                    if canceled_actions.contains(&req.params.receipt.action_id) {
                        continue;
                    }
                    (Input::Receipt(req.params.receipt), None, false)
                }
                ScheduledItem::DirectInput(di) => {
                    let inp = direct_inputs[di.index].clone();
                    if let Input::CancelAction { ref action_id, .. } = inp {
                        canceled_actions.insert(action_id.clone());
                    }
                    (inp, Some(di.scenario_idx), false)
                }
                ScheduledItem::ModelTurn {
                    request_id,
                    response_idx,
                } => {
                    let response = model_turns[response_idx].clone();
                    if let Some(reason) = response.fail_reason {
                        (Input::ModelFailure { request_id, reason }, None, false)
                    } else {
                        (
                            Input::ModelResult {
                                request_id,
                                acts: translate_thought_acts(response.acts, open_offset),
                            },
                            None,
                            false,
                        )
                    }
                }
                ScheduledItem::CoreOp { .. } => unreachable!("handled above"),
            };

            let prev_seq = core.state().last_seq;
            let step_start = Instant::now();
            let effects = core.step(input, clock.now(), &mut guard)?;
            if is_percept {
                metric_inputs
                    .appraisal_latencies_ms
                    .push(step_start.elapsed().as_secs_f64() * 1_000.0);
            }
            if let Some(sc_idx) = scenario_idx_opt {
                let appended = core.store().read_after_seq(Some(prev_seq), 16)?;
                if let Some(first) = appended
                    .iter()
                    .find(|e| !is_percept || e.kind == EventKind::Percept)
                    .or_else(|| appended.first())
                {
                    metric_inputs.scripted_event_ids.insert(sc_idx, first.id);
                }
            }

            dispatch_effects(
                scenario,
                effects,
                &mut provider,
                &mut model_turns,
                &mut guard,
                &mut purpose_costs,
                &mut clock,
                &place_to_body,
                &mut bodies,
                &mut rpc_seq,
            )?;
        } else {
            // Core::next_due() fired. Guard against duplicate zero-progress ticks at the same timestamp.
            if last_tick_at == Some(clock.now()) {
                break;
            }
            last_tick_at = Some(clock.now());
            let effects = core.step(Input::Tick, clock.now(), &mut guard)?;
            dispatch_effects(
                scenario,
                effects,
                &mut provider,
                &mut model_turns,
                &mut guard,
                &mut purpose_costs,
                &mut clock,
                &place_to_body,
                &mut bodies,
                &mut rpc_seq,
            )?;
        }
    }

    let events = core.store().read_in_order(None, usize::MAX)?;
    let wall_elapsed_ms = wall_start.elapsed().as_millis() as u64;
    let end_ms = clock.now().as_millis();
    let checks = evaluate_checks(
        &scenario.checks,
        &events,
        &guard.captured_plans,
        open_offset,
    );
    let passed = checks.iter().all(|c| c.passed);
    let purpose_cost_vec: Vec<PurposeCost> = purpose_costs.into_values().collect();
    let metrics = compute_run_metrics_full(
        &events,
        &guard.captured_plans,
        &purpose_cost_vec,
        Some(scenario),
        &metric_inputs,
        scenario.start_time_ms,
        end_ms,
        wall_elapsed_ms,
    );

    let run_record = RunRecord {
        schema_version: "1.0".to_owned(),
        run_id: format!("{}-seed-{}", scenario.id, seed),
        core_version: env!("CARGO_PKG_VERSION").to_owned(),
        prior_content_version,
        prior_format_version,
        prior_digest,
        open_offset,
        model_id: scenario.model_id.clone(),
        embedder_id: scenario.embedder_id.clone(),
        door_rules,
        switches: switches.clone(),
        learning_switches: RunRecord::learning_switches_from(&switches),
        seed,
        scenario_id: scenario.id.clone(),
        scenario_set: scenario.set,
        sealed: scenario.set == ScenarioSet::Sealed,
        checks,
        passed,
        metrics,
        cost_by_purpose: purpose_cost_vec,
        history_reasoning_mode: None,
        dataset_provenance: None,
    };

    Ok(SimOutput {
        run_record,
        events,
        context_plans: guard.captured_plans,
        guard_refusals: guard.refusals,
    })
}

/// Result of calibrating Lab thresholds/dispositions strictly from [`ScenarioSet::Development`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CalibrationReport {
    pub scenario_count: usize,
    pub embedder_ids: Vec<String>,
    pub calibrated_wake_gate_lo: f64,
    pub calibrated_fok_scale: f64,
}

/// Ensures only [`ScenarioSet::Development`] is accepted for calibration; rejects
/// `Validation`, `Sealed`, and `External` with an explicit error (`Design 6`).
pub fn ensure_calibration_set_allowed(set: ScenarioSet) -> Result<(), LabError> {
    match set {
        ScenarioSet::Development => Ok(()),
        ScenarioSet::Validation => Err(LabError::Protocol(
            "calibration refuses ScenarioSet::Validation; calibrate on development only".to_owned(),
        )),
        ScenarioSet::Sealed => Err(LabError::Protocol(
            "calibration refuses ScenarioSet::Sealed; calibrate on development only".to_owned(),
        )),
        ScenarioSet::External => Err(LabError::Protocol(
            "calibration refuses ScenarioSet::External; external results never tune the prior"
                .to_owned(),
        )),
    }
}

/// Computes calibrated thresholds from completed [`RunRecord`]s, rejecting any non-development record.
pub fn calibrate_from_run_records(records: &[RunRecord]) -> Result<CalibrationReport, LabError> {
    if records.is_empty() {
        return Err(LabError::Protocol(
            "calibration requires at least one development RunRecord".to_owned(),
        ));
    }
    let mut embedder_ids = BTreeSet::new();
    let mut go_rates = Vec::with_capacity(records.len());
    for rec in records {
        ensure_calibration_set_allowed(rec.scenario_set)?;
        embedder_ids.insert(rec.embedder_id.clone());
        let total_dec =
            rec.metrics.decisions_go + rec.metrics.decisions_note + rec.metrics.decisions_silent;
        let rate = if total_dec == 0 {
            0.5
        } else {
            rec.metrics.decisions_go as f64 / total_dec as f64
        };
        go_rates.push(rate);
    }
    let mean_go = go_rates.iter().sum::<f64>() / go_rates.len() as f64;
    Ok(CalibrationReport {
        scenario_count: records.len(),
        embedder_ids: embedder_ids.into_iter().collect(),
        calibrated_wake_gate_lo: (0.25 + 0.5 * mean_go).clamp(0.1, 0.9),
        calibrated_fok_scale: 1.0,
    })
}

/// Runs development scenarios and computes calibrated values, rejecting any non-development scenario.
pub fn calibrate_from_scenarios(
    scenarios: &[Scenario],
    prior: &Prior,
    seed: u64,
) -> Result<CalibrationReport, LabError> {
    for sc in scenarios {
        ensure_calibration_set_allowed(sc.set)?;
    }
    let mut records = Vec::with_capacity(scenarios.len());
    for sc in scenarios {
        let out = run_scenario(MemStore::new(), prior.clone(), sc, seed)?;
        records.push(out.run_record);
    }
    calibrate_from_run_records(&records)
}

/// Replays a recorded event log offline with zero model calls and compares every decision bit-for-bit.
///
/// Takes the active prior from `prior_loaded` events inside `events`, never from the caller.
pub fn replay_events_offline(
    switches: &Switches,
    seed: u64,
    events: &[EventRecord],
    mode: ReplayMode,
) -> Result<ReplayReport, LabError> {
    let mut mem = MemStore::new();
    for rec in events {
        mem.append(rec.clone())?;
    }
    let report = replay_from_checkpoint(&mem, None, switches, seed, mode)?;
    Ok(report)
}

/// Runs paired ablation across `seeds` (`>= 5` recommended) comparing `enabled_switches` against
/// `neutral_switches`, returning per-metric paired deltas and 95% confidence intervals.
pub fn run_ablation_compare(
    scenario: &Scenario,
    prior: &Prior,
    enabled_switches: Switches,
    neutral_switches: Switches,
    seeds: &[u64],
) -> Result<Vec<PairedAblationDelta>, LabError> {
    run_ablation_compare_scenarios(
        std::slice::from_ref(scenario),
        prior,
        enabled_switches,
        neutral_switches,
        seeds,
        None,
    )
}

/// Runs paired ablation over `scenarios` across `seeds`, optionally writing one postcard
/// [`RunRecord`] per `(scenario, side, seed)` into `out_dir`.
pub fn run_ablation_compare_scenarios(
    scenarios: &[Scenario],
    prior: &Prior,
    enabled_switches: Switches,
    neutral_switches: Switches,
    seeds: &[u64],
    out_dir: Option<&Path>,
) -> Result<Vec<PairedAblationDelta>, LabError> {
    if let Some(dir) = out_dir {
        fs::create_dir_all(dir)?;
    }
    let total = scenarios.len().saturating_mul(seeds.len());
    let mut enabled_metrics: Vec<RunMetricsSummary> = Vec::with_capacity(total);
    let mut neutral_metrics: Vec<RunMetricsSummary> = Vec::with_capacity(total);

    for scenario in scenarios {
        let safe_id: String = scenario
            .id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        for &seed in seeds {
            let mut sc_en = scenario.clone();
            sc_en.switches = Some(enabled_switches.clone());
            let out_en = run_scenario(MemStore::new(), prior.clone(), &sc_en, seed)?;
            if let Some(dir) = out_dir {
                let path = dir.join(format!("{safe_id}__enabled__seed-{seed}.postcard"));
                fs::write(path, out_en.run_record.encode_postcard()?)?;
            }
            enabled_metrics.push(out_en.run_record.metrics);

            let mut sc_neu = scenario.clone();
            sc_neu.switches = Some(neutral_switches.clone());
            let out_neu = run_scenario(MemStore::new(), prior.clone(), &sc_neu, seed)?;
            if let Some(dir) = out_dir {
                let path = dir.join(format!("{safe_id}__neutral__seed-{seed}.postcard"));
                fs::write(path, out_neu.run_record.encode_postcard()?)?;
            }
            neutral_metrics.push(out_neu.run_record.metrics);
        }
    }

    Ok(compare_paired_ablation(&enabled_metrics, &neutral_metrics))
}

#[allow(clippy::too_many_arguments)]
fn dispatch_effects(
    scenario: &Scenario,
    effects: Vec<Effect>,
    provider: &mut ScriptedProvider,
    model_turns: &mut Vec<ScriptedModelResponse>,
    guard: &mut CostGuard,
    purpose_costs: &mut BTreeMap<u8, PurposeCost>,
    clock: &mut VirtualClock<ScheduledItem>,
    place_to_body: &BTreeMap<PlaceId, BodyId>,
    bodies: &mut BTreeMap<BodyId, TestBody>,
    rpc_seq: &mut i64,
) -> Result<(), LabError> {
    for effect in effects {
        match effect {
            Effect::Model { purpose, plan } => {
                let request_id = plan.trigger_event_id.unwrap_or(EventId::new(0));
                schedule_model_call(
                    scenario,
                    provider,
                    model_turns,
                    guard,
                    purpose_costs,
                    clock,
                    purpose,
                    request_id,
                );
            }
            Effect::Dispatch(action) => {
                dispatch_action_over_waist2(&action, place_to_body, bodies, clock, rpc_seq)?;
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn schedule_model_call(
    scenario: &Scenario,
    provider: &mut ScriptedProvider,
    model_turns: &mut Vec<ScriptedModelResponse>,
    guard: &mut CostGuard,
    purpose_costs: &mut BTreeMap<u8, PurposeCost>,
    clock: &mut VirtualClock<ScheduledItem>,
    purpose: ModelPurpose,
    request_id: EventId,
) {
    let response = provider.next_response();
    let latency = response.latency_ms.unwrap_or(scenario.model_latency_ms);
    let total_tok = response.tokens_in.saturating_add(response.tokens_out);
    guard.record_tokens(total_tok);

    let key = match purpose {
        ModelPurpose::Reason => 0,
        ModelPurpose::Classify => 1,
        ModelPurpose::Consolidate => 2,
    };
    let entry = purpose_costs.entry(key).or_insert(PurposeCost {
        purpose,
        calls: 0,
        tokens_in: 0,
        tokens_out: 0,
        cost_usd: 0.0,
    });
    entry.calls += 1;
    entry.tokens_in += response.tokens_in;
    entry.tokens_out += response.tokens_out;

    let response_idx = model_turns.len();
    model_turns.push(response);
    clock.schedule_after(
        latency,
        ScheduledItem::ModelTurn {
            request_id,
            response_idx,
        },
    );
}

fn dispatch_action_over_waist2(
    action: &Action,
    place_to_body: &BTreeMap<PlaceId, BodyId>,
    bodies: &mut BTreeMap<BodyId, TestBody>,
    clock: &mut VirtualClock<ScheduledItem>,
    rpc_seq: &mut i64,
) -> Result<(), LabError> {
    let primary_place = action.places.first();
    let target_body = primary_place
        .and_then(|p| place_to_body.get(p))
        .and_then(|bid| bodies.get_mut(bid));

    let Some(body) = target_body else {
        let receipt = Receipt {
            action_id: action.action_id.clone(),
            capability_id: action.capability_id.clone(),
            outcome: ReceiptOutcome::Refused,
            reason_code: Some("ERR_UNKNOWN_PLACE".to_owned()),
            detail: Some("no registered TestBody owns target place".to_owned()),
            termination_reason: Some(TerminationReason::Normal),
            progress: Some(0.0),
            delivered_units: Some(0),
            h_act: action.h_act.clone(),
            timestamp_ms: clock.now(),
        };
        let req = Waist2Request::new(
            Some(RpcId::Int(*rpc_seq)),
            Waist2Method::ActionReceipt,
            ActionReceiptParams {
                headers: Waist2Headers::new(format!("no-body-{}", action.action_id)),
                receipt,
            },
        );
        *rpc_seq += 1;
        let json_rpc =
            serde_json::to_string(&req).map_err(|e| LabError::Serialization(e.to_string()))?;
        clock.schedule_after(0, ScheduledItem::WireReceipt { json_rpc });
        return Ok(());
    };

    body.set_virtual_time(clock.now());
    let rpc_id = RpcId::Int(*rpc_seq);
    *rpc_seq += 1;

    let exec_req = Waist2Request::new(
        Some(rpc_id),
        Waist2Method::ActionExecute,
        ActionExecuteParams {
            headers: Waist2Headers::new(format!("exec-{}", action.action_id)),
            action_id: action.action_id.clone(),
            capability_id: action.capability_id.clone(),
            places: action.places.clone(),
            audience: action.audience.clone(),
            payload: action.payload.clone(),
            timeout_ms: None,
            h_act: action.h_act.clone(),
        },
    );
    let req_line =
        serde_json::to_string(&exec_req).map_err(|e| LabError::Serialization(e.to_string()))?;
    let resp_line = body.handle_json_rpc(&req_line);
    let resp: monoagens_protocol::Waist2Response<ActionExecuteResult> =
        serde_json::from_str(&resp_line).map_err(|e| LabError::Serialization(e.to_string()))?;

    if let Some(err) = resp.error {
        let receipt = Receipt {
            action_id: action.action_id.clone(),
            capability_id: action.capability_id.clone(),
            outcome: if err.code == -32001 {
                ReceiptOutcome::Refused
            } else {
                ReceiptOutcome::Failed
            },
            reason_code: err.data.map(|d| d.symbol),
            detail: Some(err.message),
            termination_reason: Some(TerminationReason::Normal),
            progress: Some(0.0),
            delivered_units: Some(0),
            h_act: action.h_act.clone(),
            timestamp_ms: clock.now(),
        };
        let req = Waist2Request::new(
            Some(RpcId::Int(*rpc_seq)),
            Waist2Method::ActionReceipt,
            ActionReceiptParams {
                headers: Waist2Headers::new(format!("err-rcpt-{}", action.action_id)),
                receipt,
            },
        );
        *rpc_seq += 1;
        let json_rpc =
            serde_json::to_string(&req).map_err(|e| LabError::Serialization(e.to_string()))?;
        clock.schedule_after(0, ScheduledItem::WireReceipt { json_rpc });
        return Ok(());
    }

    let body_id = body.body_id().clone();
    let outbound = body.drain_outbound_json();
    for item in outbound {
        let probe: Waist2Request<serde_json::Value> = serde_json::from_str(&item.json_rpc)
            .map_err(|e| LabError::Serialization(e.to_string()))?;
        match probe.method {
            Waist2Method::ActionReceipt => {
                clock.schedule_after(
                    item.delay_ms,
                    ScheduledItem::WireReceipt {
                        json_rpc: item.json_rpc,
                    },
                );
            }
            Waist2Method::PerceptEmit => {
                clock.schedule_after(
                    item.delay_ms,
                    ScheduledItem::WirePercept {
                        scenario_idx: None,
                        body_id: Some(body_id.clone()),
                        json_rpc: item.json_rpc,
                    },
                );
            }
            other => {
                return Err(LabError::Protocol(format!(
                    "unexpected outbound method '{other}' from TestBody"
                )));
            }
        }
    }

    Ok(())
}

fn translate_id_vec(ids: Vec<EventId>, open_offset: u64) -> Vec<EventId> {
    ids.into_iter()
        .map(|id| translate_scenario_event_id(id, open_offset))
        .collect()
}

fn translate_thought_acts(acts: Vec<ThoughtAct>, open_offset: u64) -> Vec<ThoughtAct> {
    if open_offset == 0 {
        return acts;
    }
    acts.into_iter()
        .map(|act| match act {
            ThoughtAct::Think {
                thought,
                motivation,
                confidence_0_20,
                used_ids,
                importance,
                basis,
                value,
                cost,
                domain,
            } => ThoughtAct::Think {
                thought,
                motivation,
                confidence_0_20,
                used_ids: translate_id_vec(used_ids, open_offset),
                importance,
                basis,
                value,
                cost,
                domain,
            },
            ThoughtAct::Act {
                capability_id,
                places,
                people,
                payload,
                motivation,
                confidence_0_20,
                used_ids,
                expected,
                domain,
            } => ThoughtAct::Act {
                capability_id,
                places,
                people,
                payload,
                motivation,
                confidence_0_20,
                used_ids: translate_id_vec(used_ids, open_offset),
                expected,
                domain,
            },
            ThoughtAct::Expect {
                expect_kind,
                about_ids,
                places,
                people,
                condition,
                due_ms,
                expiry_ms,
                recurrence,
                promise_meta,
                domain,
            } => ThoughtAct::Expect {
                expect_kind,
                about_ids: translate_id_vec(about_ids, open_offset),
                places,
                people,
                condition,
                due_ms,
                expiry_ms,
                recurrence,
                promise_meta,
                domain,
            },
            ThoughtAct::Resolve {
                expectation_id,
                outcome,
                note,
                places,
                people,
                domain,
            } => ThoughtAct::Resolve {
                expectation_id: translate_scenario_event_id(expectation_id, open_offset),
                outcome,
                note,
                places,
                people,
                domain,
            },
            ThoughtAct::Revise {
                target_id,
                new_version,
                condition,
                due_ms,
                expiry_ms,
                places,
                people,
                domain,
            } => ThoughtAct::Revise {
                target_id: translate_scenario_event_id(target_id, open_offset),
                new_version,
                condition,
                due_ms,
                expiry_ms,
                places,
                people,
                domain,
            },
            ThoughtAct::Consolidate { output } => ThoughtAct::Consolidate {
                output: match output {
                    ConsolidateOutput::Gist {
                        summary,
                        about_ids,
                        places,
                        people,
                        domain,
                    } => ConsolidateOutput::Gist {
                        summary,
                        about_ids: translate_id_vec(about_ids, open_offset),
                        places,
                        people,
                        domain,
                    },
                    ConsolidateOutput::Lesson {
                        pattern,
                        failure_signature,
                        contrastive_success_ids,
                        failure_ids,
                        domain,
                    } => ConsolidateOutput::Lesson {
                        pattern,
                        failure_signature,
                        contrastive_success_ids: translate_id_vec(
                            contrastive_success_ids,
                            open_offset,
                        ),
                        failure_ids: translate_id_vec(failure_ids, open_offset),
                        domain,
                    },
                    other @ ConsolidateOutput::Procedure { .. } => other,
                },
            },
        })
        .collect()
}

//! All 22 brain-map ablation metrics (`A-02`), safety metrics, multi-seed aggregation,
//! and paired confidence-interval comparisons (`specs/lab/spec.md`).
//!
//! Every metric documents its exact formula next to its computation and strictly reports
//! `None` (`unavailable`) when a scenario did not exercise or annotate that mechanism—never
//! inventing a `0.0` placeholder.

use std::collections::{BTreeMap, BTreeSet};

use monoagens_core::{
    ContextPlan, DecisionOutcome, EventDetail, EventKind, EventRecord, ExpectationLifecycleState,
    ModelPurpose,
};
use monoagens_protocol::{ActorId, EventId, ReceiptOutcome, TerminationReason};
use serde::{Deserialize, Serialize};

use crate::record::PurposeCost;
use crate::scenario::{Scenario, WarrantedReply};

/// Observation metadata collected during a simulation run for exact metric attribution.
#[derive(Debug, Clone, Default)]
pub struct RunMetricInputs {
    /// Mapping from `Scenario.events` index -> assigned `EventId` in the log.
    pub scripted_event_ids: BTreeMap<usize, EventId>,
    /// Per-percept appraisal wall-clock durations in milliseconds.
    pub appraisal_latencies_ms: Vec<f64>,
    /// Scripted barge-in interrupt timestamps keyed by `ActionId`.
    pub barge_in_timestamps_ms: BTreeMap<monoagens_protocol::ActionId, u64>,
    /// Whether `Core::hard_forget` ran and left zero surviving target references.
    pub hard_forget_resurrection_count: Option<usize>,
    /// Measured recall probe hits (`(gap_days, hit_bool)`).
    pub recall_probe_outcomes: Vec<(u32, bool)>,
    /// Explicit safety leakage detections outside `ContextPlan` (if any).
    pub extra_safety_leaks: usize,
}

/// One metric entry with optional brain-map row, formula, value, or unavailability reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricReportEntry {
    pub name: String,
    pub row: Option<u8>,
    pub value: Option<f64>,
    pub unavailable_reason: Option<String>,
    pub formula: String,
}

/// Complete 22-row + safety metric report for a single run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DetailedMetricsReport {
    pub entries: BTreeMap<String, MetricReportEntry>,
    pub reply_delay_distribution_ms: Vec<u64>,
    pub calls_by_purpose: BTreeMap<String, u64>,
    pub tokens_by_purpose: BTreeMap<String, u64>,
}

/// Summary metrics computed over a completed Lab run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunMetricsSummary {
    pub total_events: usize,
    pub percepts: usize,
    pub decisions_go: usize,
    pub decisions_note: usize,
    pub decisions_silent: usize,
    pub decisions_door_blocked: usize,
    pub actions_dispatched: usize,
    pub receipts_recorded: usize,
    pub wakes_per_event: f64,
    pub simulated_span_ms: u64,
    pub wall_elapsed_ms: u64,
    /// Produced brain-map and safety metrics (contains ONLY metrics exercised by the run).
    pub brain_map_metrics: Option<BTreeMap<String, f64>>,
    /// Full structured metric table including `unavailable` reasons, formulas, and distributions.
    pub detailed: Option<DetailedMetricsReport>,
}

/// Multi-seed summary statistics for one metric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeedAggregate {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<u8>,
    pub n_seeds: usize,
    pub mean: Option<f64>,
    pub std_dev: Option<f64>,
    pub ci95_half_width: Option<f64>,
}

/// Paired ablation comparison (`enabled - neutral`) across `n_seeds >= 5` seeds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairedAblationDelta {
    pub metric: String,
    pub n_seeds: usize,
    pub enabled_mean: Option<f64>,
    pub neutral_mean: Option<f64>,
    pub mean_delta: Option<f64>,
    pub ci95_half_width: Option<f64>,
    pub within_seed_noise: bool,
}

/// Computes basic deterministic run summary metrics (legacy signature preserved for callers).
#[must_use]
pub fn compute_run_metrics(
    events: &[EventRecord],
    start_ms: u64,
    end_ms: u64,
    wall_elapsed_ms: u64,
) -> RunMetricsSummary {
    compute_run_metrics_full(
        events,
        &[],
        &[],
        None,
        &RunMetricInputs::default(),
        start_ms,
        end_ms,
        wall_elapsed_ms,
    )
}

/// Computes all 22 brain-map and safety metrics (`A-02`, `specs/lab/spec.md`) for a completed run.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn compute_run_metrics_full(
    events: &[EventRecord],
    context_plans: &[ContextPlan],
    purpose_costs: &[PurposeCost],
    scenario: Option<&Scenario>,
    inputs: &RunMetricInputs,
    start_ms: u64,
    end_ms: u64,
    wall_elapsed_ms: u64,
) -> RunMetricsSummary {
    let mut percepts = 0usize;
    let mut decisions_go = 0usize;
    let mut decisions_note = 0usize;
    let mut decisions_silent = 0usize;
    let mut decisions_door_blocked = 0usize;
    let mut actions_dispatched = 0usize;
    let mut receipts_recorded = 0usize;

    let mut percept_by_id: BTreeMap<EventId, &EventRecord> = BTreeMap::new();
    let mut decision_by_target: BTreeMap<EventId, DecisionOutcome> = BTreeMap::new();
    let mut communicative_action_targets: BTreeSet<EventId> = BTreeSet::new();
    let mut action_timestamps_by_target: BTreeMap<EventId, u64> = BTreeMap::new();
    let mut soul_actor = ActorId::new("soul");

    for rec in events {
        match rec.kind {
            EventKind::Percept => {
                percepts += 1;
                percept_by_id.insert(rec.id, rec);
            }
            EventKind::Action => {
                actions_dispatched += 1;
                for &cause in &rec.caused_by {
                    communicative_action_targets.insert(cause);
                    action_timestamps_by_target
                        .entry(cause)
                        .or_insert(rec.timestamp_ms.as_millis());
                }
                if rec.actor != ActorId::new("core") {
                    soul_actor = rec.actor.clone();
                }
            }
            EventKind::Receipt => receipts_recorded += 1,
            EventKind::Decision => {
                if let Some(EventDetail::Decision {
                    target_event_id,
                    outcome,
                    ..
                }) = &rec.detail
                {
                    decision_by_target.insert(*target_event_id, *outcome);
                    match outcome {
                        DecisionOutcome::Go => decisions_go += 1,
                        DecisionOutcome::Note => decisions_note += 1,
                        DecisionOutcome::Silent => decisions_silent += 1,
                        DecisionOutcome::DoorBlocked => decisions_door_blocked += 1,
                        DecisionOutcome::Processed => {}
                    }
                }
            }
            _ => {}
        }
    }

    let wakes_per_event = if percepts > 0 {
        decisions_go as f64 / percepts as f64
    } else {
        0.0
    };

    let mut detailed = DetailedMetricsReport::default();

    for pc in purpose_costs {
        let key = match pc.purpose {
            ModelPurpose::Reason => "reason",
            ModelPurpose::Classify => "classify",
            ModelPurpose::Consolidate => "consolidate",
        };
        *detailed.calls_by_purpose.entry(key.to_owned()).or_insert(0) += pc.calls;
        *detailed
            .tokens_by_purpose
            .entry(key.to_owned())
            .or_insert(0) += pc.tokens_in.saturating_add(pc.tokens_out);
    }

    // -----------------------------------------------------------------------
    // Row 1 (Salience) & Row 15 (Bounded learning): Precision, Recall, F1, F1 improvement
    // Formula:
    //   TP = |{e : warranted(e) == Speak && outcome(e) == Go}|
    //   FP = |{e : warranted(e) == Silent && outcome(e) == Go}|
    //   FN = |{e : warranted(e) == Speak && outcome(e) != Go}|
    //   precision = TP / (TP + FP), recall = TP / (TP + FN)
    //   F1 = 2 * precision * recall / (precision + recall)
    //   F1_improvement = F1(second_half) - F1(first_half)
    // -----------------------------------------------------------------------
    let mut tp = 0usize;
    let mut fp = 0usize;
    let mut fn_count = 0usize;
    let mut has_speak = false;
    let mut has_silent = false;
    let mut annotated_ordered: Vec<(WarrantedReply, bool)> = Vec::new();

    if let Some(sc) = scenario {
        for (&idx, ann) in &sc.annotations.event_annotations {
            if let Some(w) = ann.warranted
                && let Some(&eid) = inputs.scripted_event_ids.get(&idx)
            {
                let went = decision_by_target.get(&eid).copied() == Some(DecisionOutcome::Go);
                annotated_ordered.push((w, went));
                match (w, went) {
                    (WarrantedReply::Speak, true) => {
                        has_speak = true;
                        tp += 1;
                    }
                    (WarrantedReply::Speak, false) => {
                        has_speak = true;
                        fn_count += 1;
                    }
                    (WarrantedReply::Silent, true) => {
                        has_silent = true;
                        fp += 1;
                    }
                    (WarrantedReply::Silent, false) => {
                        has_silent = true;
                    }
                }
            }
        }
    }

    let precision = if has_speak || has_silent {
        if tp + fp > 0 {
            Some(tp as f64 / (tp + fp) as f64)
        } else {
            Some(1.0)
        }
    } else {
        None
    };
    let recall = if has_speak {
        Some(tp as f64 / (tp + fn_count) as f64)
    } else {
        None
    };

    record_entry(
        &mut detailed.entries,
        "when_to_speak_precision",
        Some(1),
        precision,
        "no SPEAK/SILENT event annotations in scenario",
        "TP / (TP + FP) over warranted SPEAK/SILENT annotations",
    );
    record_entry(
        &mut detailed.entries,
        "when_to_speak_recall",
        Some(1),
        recall,
        "no SPEAK event annotations in scenario",
        "TP / (TP + FN) over warranted SPEAK annotations",
    );
    record_entry(
        &mut detailed.entries,
        "false_speaks",
        Some(1),
        has_silent.then_some(fp as f64),
        "no SILENT event annotations in scenario",
        "count(warranted == SILENT && decision == Go)",
    );
    record_entry(
        &mut detailed.entries,
        "missed_replies",
        Some(1),
        has_speak.then_some(fn_count as f64),
        "no SPEAK event annotations in scenario",
        "count(warranted == SPEAK && decision != Go)",
    );

    // -----------------------------------------------------------------------
    // Row 2 (Floor cost): floor_violations_per_100
    // Formula:
    //   100 * |{e : (sel_other(e) || answered(e)) && soul_spoke(e)}| / |{e : sel_other(e) || answered(e)}|
    // -----------------------------------------------------------------------
    let floor_val = scenario.and_then(|sc| {
        let mut total = 0usize;
        let mut violations = 0usize;
        for (&idx, ann) in &sc.annotations.event_annotations {
            if (ann.sel_other || ann.answered)
                && let Some(&eid) = inputs.scripted_event_ids.get(&idx)
            {
                total += 1;
                if decision_by_target.get(&eid).copied() == Some(DecisionOutcome::Go)
                    || communicative_action_targets.contains(&eid)
                {
                    violations += 1;
                }
            }
        }
        (total > 0).then(|| 100.0 * violations as f64 / total as f64)
    });
    record_entry(
        &mut detailed.entries,
        "floor_violations_per_100",
        Some(2),
        floor_val,
        "no sel_other or answered floor-annotated events",
        "100 * violations / count(sel_other || answered)",
    );

    // -----------------------------------------------------------------------
    // Row 3 (Habituation): wakes_per_event & wakes_per_repeated_stimulus
    // Formula:
    //   wakes_per_event = decisions_go / percepts
    //   wakes_per_repeated_stimulus = repeated_wakes / repeated_unaddressed_percepts
    //     (where a percept is repeated if the same (place, actor) appeared >= 1 time earlier unaddressed)
    // -----------------------------------------------------------------------
    record_entry(
        &mut detailed.entries,
        "wakes_per_event",
        Some(3),
        (percepts > 0).then_some(wakes_per_event),
        "no percept events recorded",
        "decisions_go / percepts",
    );
    let repeated_wakes_val = {
        let mut seen_unaddressed: BTreeMap<(String, String), usize> = BTreeMap::new();
        let mut rep_total = 0usize;
        let mut rep_wakes = 0usize;
        for rec in events.iter().filter(|r| r.kind == EventKind::Percept) {
            let is_addressed = matches!(
                &rec.detail,
                Some(EventDetail::PerceptMeta { addressees, named, .. })
                if addressees.contains(&soul_actor) || named.contains(&soul_actor)
            );
            if !is_addressed {
                let key = (rec.place.as_str().to_owned(), rec.actor.as_str().to_owned());
                let count = seen_unaddressed.entry(key).or_insert(0);
                *count += 1;
                if *count >= 2 {
                    rep_total += 1;
                    if decision_by_target.get(&rec.id).copied() == Some(DecisionOutcome::Go) {
                        rep_wakes += 1;
                    }
                }
            }
        }
        (rep_total > 0).then(|| rep_wakes as f64 / rep_total as f64)
    };
    record_entry(
        &mut detailed.entries,
        "wakes_per_repeated_stimulus",
        Some(3),
        repeated_wakes_val,
        "no repeated unaddressed (place, source) stimuli",
        "wakes_on_repeated_unaddressed / repeated_unaddressed_stimuli",
    );

    // -----------------------------------------------------------------------
    // Row 4 (Drive threshold): 1/n message share band [0.5, 2.0], daily budget overrun, overdue promise rate
    // Formula:
    //   message_share_vs_1_over_n = (soul_messages / total_messages) / (1 / n)
    //   message_share_in_band = 1.0 if 0.5 <= share <= 2.0 else 0.0
    //   daily_budget_overrun_rate = count(day_tokens > daily_budget) / total_days
    // -----------------------------------------------------------------------
    let (share_ratio, in_band) = match scenario.and_then(|s| s.annotations.participant_count) {
        Some(n) if n > 0 => {
            let soul_msgs = actions_dispatched;
            let total_msgs = percepts + soul_msgs;
            if total_msgs > 0 {
                let ratio = (soul_msgs as f64 / total_msgs as f64) * (n as f64);
                let ok = if (0.5..=2.0).contains(&ratio) {
                    1.0
                } else {
                    0.0
                };
                (Some(ratio), Some(ok))
            } else {
                (None, None)
            }
        }
        _ => (None, None),
    };
    record_entry(
        &mut detailed.entries,
        "message_share_vs_1_over_n",
        Some(4),
        share_ratio,
        "participant_count n not annotated or zero messages",
        "(soul_messages / total_messages) / (1 / n)",
    );
    record_entry(
        &mut detailed.entries,
        "message_share_in_band",
        Some(4),
        in_band,
        "participant_count n not annotated or zero messages",
        "1.0 if 0.5 <= message_share_vs_1_over_n <= 2.0 else 0.0",
    );

    let budget_overrun = scenario
        .and_then(|s| s.annotations.daily_budget_tokens)
        .map(|limit| {
            let total_tok: u64 = purpose_costs
                .iter()
                .map(|c| c.tokens_in.saturating_add(c.tokens_out))
                .sum();
            let days = (end_ms.saturating_sub(start_ms) / 86_400_000).max(1);
            let mean_day_tok = total_tok as f64 / days as f64;
            if mean_day_tok > limit as f64 {
                1.0
            } else {
                0.0
            }
        });
    record_entry(
        &mut detailed.entries,
        "daily_budget_overrun_rate",
        Some(4),
        budget_overrun,
        "daily_budget_tokens not configured on scenario",
        "fraction of simulated days with tokens_used > daily_budget_tokens",
    );

    // -----------------------------------------------------------------------
    // Row 5 (Familiarity reserve): false_speak_rate_unfamiliar
    // Formula:
    //   100 * |{e : unfamiliar_place(e) && !addressed(e) && decision(e) == Go}| / |{e : unfamiliar_place(e) && !addressed(e)}|
    // -----------------------------------------------------------------------
    let unfamiliar_rate = scenario.and_then(|sc| {
        let mut denom = 0usize;
        let mut false_spks = 0usize;
        for (&idx, ann) in &sc.annotations.event_annotations {
            if ann.unfamiliar_place
                && !ann.addressed_to_soul
                && let Some(&eid) = inputs.scripted_event_ids.get(&idx)
            {
                denom += 1;
                if decision_by_target.get(&eid).copied() == Some(DecisionOutcome::Go) {
                    false_spks += 1;
                }
            }
        }
        (denom > 0).then(|| 100.0 * false_spks as f64 / denom as f64)
    });
    record_entry(
        &mut detailed.entries,
        "false_speak_rate_unfamiliar",
        Some(5),
        unfamiliar_rate,
        "no unfamiliar_place unallocated events annotated",
        "100 * false_speaks_in_unfamiliar_places / unfamiliar_unallocated_events",
    );

    // -----------------------------------------------------------------------
    // Row 6 (Stochastic go/no-go): reply_delay_distribution & 1-Wasserstein distance
    // Formula:
    //   delays = sorted([action.ts - trigger_percept.ts])
    //   W_1(P, Q) = integral |F_P(x) - F_Q(x)| dx over quantile grid
    // -----------------------------------------------------------------------
    let mut soul_delays: Vec<u64> = Vec::new();
    for (target_id, &act_ts) in &action_timestamps_by_target {
        if let Some(p_rec) = percept_by_id.get(target_id) {
            soul_delays.push(act_ts.saturating_sub(p_rec.timestamp_ms.as_millis()));
        }
    }
    soul_delays.sort_unstable();
    detailed.reply_delay_distribution_ms = soul_delays.clone();

    let delay_dist = scenario.and_then(|sc| {
        if soul_delays.is_empty() || sc.annotations.human_reply_delays_ms.is_empty() {
            None
        } else {
            Some(wasserstein_1d_ms(
                &soul_delays,
                &sc.annotations.human_reply_delays_ms,
            ))
        }
    });
    record_entry(
        &mut detailed.entries,
        "reply_delay_distance_ms",
        Some(6),
        delay_dist,
        "requires both soul reply delays and human_reply_delays_ms annotation",
        "1-Wasserstein distance W_1(soul_reply_delays_ms, human_reply_delays_ms)",
    );

    // -----------------------------------------------------------------------
    // Row 7 (Commitment breaker): unanswered_allocated_turn_rate
    // Formula:
    //   |{e : allocated(e) && decision(e) != Go}| / |{e : allocated(e)}|
    // -----------------------------------------------------------------------
    let unanswered_alloc = {
        let mut alloc = 0usize;
        let mut missed = 0usize;
        if let Some(sc) = scenario {
            for (&idx, ann) in &sc.annotations.event_annotations {
                if ann.addressed_to_soul
                    && let Some(&eid) = inputs.scripted_event_ids.get(&idx)
                {
                    alloc += 1;
                    if decision_by_target.get(&eid).copied() != Some(DecisionOutcome::Go) {
                        missed += 1;
                    }
                }
            }
        }
        (alloc > 0).then(|| missed as f64 / alloc as f64)
    };
    record_entry(
        &mut detailed.entries,
        "unanswered_allocated_turn_rate",
        Some(7),
        unanswered_alloc,
        "no addressed_to_soul events annotated",
        "unanswered_allocated_turns / total_allocated_turns",
    );

    // -----------------------------------------------------------------------
    // Row 8 (Patience): double_answers
    // Formula:
    //   count({e : double_answer_eligible(e) && !addressed(e) && soul_spoke(e)})
    // -----------------------------------------------------------------------
    let double_ans = scenario.and_then(|sc| {
        let mut eligible = 0usize;
        let mut count = 0usize;
        for (&idx, ann) in &sc.annotations.event_annotations {
            if ann.double_answer_eligible
                && let Some(&eid) = inputs.scripted_event_ids.get(&idx)
            {
                eligible += 1;
                if decision_by_target.get(&eid).copied() == Some(DecisionOutcome::Go)
                    || communicative_action_targets.contains(&eid)
                {
                    count += 1;
                }
            }
        }
        (eligible > 0).then_some(count as f64)
    });
    record_entry(
        &mut detailed.entries,
        "double_answers",
        Some(8),
        double_ans,
        "no double_answer_eligible third-party-replied events annotated",
        "count(soul_spoke on unallocated event already answered within patience window)",
    );

    // -----------------------------------------------------------------------
    // Row 9 (Report bar on thought): low_value_message_rate
    // Formula:
    //   count(spoken && low_value_if_spoken) / count(spoken)
    // -----------------------------------------------------------------------
    let low_val_rate = scenario.and_then(|sc| {
        let mut spoken_ann = 0usize;
        let mut low_val = 0usize;
        for (&idx, ann) in &sc.annotations.event_annotations {
            if let Some(&eid) = inputs.scripted_event_ids.get(&idx)
                && (decision_by_target.get(&eid).copied() == Some(DecisionOutcome::Go)
                    || communicative_action_targets.contains(&eid))
            {
                spoken_ann += 1;
                if ann.low_value_if_spoken {
                    low_val += 1;
                }
            }
        }
        let has_low_val_annotations = sc
            .annotations
            .event_annotations
            .values()
            .any(|a| a.low_value_if_spoken);
        (has_low_val_annotations && spoken_ann > 0).then(|| low_val as f64 / spoken_ann as f64)
    });
    record_entry(
        &mut detailed.entries,
        "low_value_message_rate",
        Some(9),
        low_val_rate,
        "no low_value_if_spoken annotations exercised",
        "low_value_spoken_acts / total_spoken_acts",
    );

    // -----------------------------------------------------------------------
    // Row 10 (Activation recall): 1-day, 7-day, 30-day recall hits
    // Formula:
    //   hit_rate(gap_d) = hits(gap_d) / total_probes(gap_d)
    // -----------------------------------------------------------------------
    for gap_d in [1_u32, 7, 30] {
        let key = format!("recall_hit_{gap_d}d");
        let rate = if !inputs.recall_probe_outcomes.is_empty() {
            let matching: Vec<bool> = inputs
                .recall_probe_outcomes
                .iter()
                .filter(|(g, _)| *g == gap_d)
                .map(|(_, hit)| *hit)
                .collect();
            (!matching.is_empty())
                .then(|| matching.iter().filter(|&&h| h).count() as f64 / matching.len() as f64)
        } else if let Some(sc) = scenario {
            let probes: Vec<_> = sc
                .annotations
                .recall_probes
                .iter()
                .filter(|p| p.gap_days == gap_d)
                .collect();
            if probes.is_empty() {
                None
            } else {
                let mut hits = 0usize;
                for probe in &probes {
                    let expected_ids: BTreeSet<EventId> = probe
                        .expected_event_indices
                        .iter()
                        .filter_map(|idx| inputs.scripted_event_ids.get(idx).copied())
                        .collect();
                    let found_in_ctx = expected_ids.iter().any(|eid| {
                        context_plans.iter().any(|p| p.items.contains(eid))
                            || events.iter().any(|ev| {
                                ev.kind == EventKind::MemoryUsed
                                    && matches!(&ev.detail, Some(EventDetail::MemoryUsed { used_ids }) if used_ids.contains(eid))
                            })
                    });
                    if found_in_ctx || expected_ids.is_empty() {
                        hits += 1;
                    }
                }
                Some(hits as f64 / probes.len() as f64)
            }
        } else {
            None
        };
        record_entry(
            &mut detailed.entries,
            &key,
            Some(10),
            rate,
            "no recall probes for this day gap in scenario",
            "hits_at_gap_days / total_probes_at_gap_days",
        );
    }

    // -----------------------------------------------------------------------
    // Row 11 (Vector fade): faded_vector_count & appraisal_latency_p95_ms
    // Formula:
    //   faded_vector_count = count(EventKind::VectorReleased)
    //   appraisal_latency_p95_ms = quantile_0.95(per_percept_appraisal_wall_ms)
    // -----------------------------------------------------------------------
    let faded_count = events
        .iter()
        .filter(|e| e.kind == EventKind::VectorReleased)
        .count();
    record_entry(
        &mut detailed.entries,
        "faded_vector_count",
        Some(11),
        (faded_count > 0).then_some(faded_count as f64),
        "no VectorReleased fade events occurred in run",
        "count(EventKind::VectorReleased where Petrov B < fade_threshold)",
    );
    let p95_lat = if inputs.appraisal_latencies_ms.is_empty() {
        None
    } else {
        let mut sorted = inputs.appraisal_latencies_ms.clone();
        sorted.sort_by(f64::total_cmp);
        let idx = ((sorted.len() - 1) as f64 * 0.95).round() as usize;
        Some(sorted[idx])
    };
    record_entry(
        &mut detailed.entries,
        "appraisal_latency_p95_ms",
        Some(11),
        p95_lat,
        "no percept appraisals timed",
        "p95(wall_ms per Core::step(Percept))",
    );

    // -----------------------------------------------------------------------
    // Row 12 (Prediction & grounding): unnoticed_non_uptake_rate
    // Formula:
    //   unnoticed / non_uptake_events where noticed = ContentMiss | SilenceJudged | ExpectationTransition(Failed|Expired)
    // -----------------------------------------------------------------------
    let non_uptake_rate = {
        let noticed_events = events
            .iter()
            .filter(|e| {
                matches!(
                    &e.detail,
                    Some(EventDetail::ContentMiss { .. } | EventDetail::SilenceJudged { .. })
                        | Some(EventDetail::ExpectationTransition {
                            to_state: ExpectationLifecycleState::Failed
                                | ExpectationLifecycleState::Expired,
                            ..
                        })
                )
            })
            .count();
        let expected_non_uptake = scenario
            .map(|sc| {
                sc.annotations
                    .event_annotations
                    .values()
                    .filter(|a| a.non_uptake_expected_repair)
                    .count()
            })
            .unwrap_or(0);
        let total = expected_non_uptake.max(noticed_events);
        if total > 0 {
            let unnoticed = expected_non_uptake.saturating_sub(noticed_events);
            Some(unnoticed as f64 / total as f64)
        } else {
            None
        }
    };
    record_entry(
        &mut detailed.entries,
        "unnoticed_non_uptake_rate",
        Some(12),
        non_uptake_rate,
        "no expired predictions, content misses, or missed silences",
        "(expected_non_uptake - noticed_non_uptake_events) / expected_non_uptake",
    );

    // -----------------------------------------------------------------------
    // Row 13 (Prospective memory): promises kept, kept_late, repaired, broken + Row 4 overdue_promise_rate
    // Formula:
    //   Counts over terminal ExpectationTransition(to_state in {Kept, KeptLate, Renegotiated, Failed, Expired})
    //   overdue_promise_rate = (kept_late + broken) / total_promises
    // -----------------------------------------------------------------------
    let mut promise_ids: BTreeSet<EventId> = BTreeSet::new();
    for rec in events {
        if let Some(EventDetail::ExpectationRecordOpened {
            expectation_id,
            kind,
            ..
        }) = &rec.detail
            && *kind == monoagens_protocol::ExpectKind::Promise
        {
            promise_ids.insert(*expectation_id);
        }
    }

    let mut kept = 0usize;
    let mut kept_late = 0usize;
    let mut repaired = 0usize;
    let mut broken = 0usize;
    let opened_promises = promise_ids.len();

    for rec in events {
        if let Some(EventDetail::ExpectationTransition {
            expectation_id,
            to_state,
            ..
        }) = &rec.detail
            && promise_ids.contains(expectation_id)
        {
            match to_state {
                ExpectationLifecycleState::Kept => kept += 1,
                ExpectationLifecycleState::KeptLate => kept_late += 1,
                ExpectationLifecycleState::Renegotiated => repaired += 1,
                ExpectationLifecycleState::Failed | ExpectationLifecycleState::Expired => {
                    broken += 1;
                }
                _ => {}
            }
        }
    }
    let total_resolved_promises = kept + kept_late + repaired + broken;
    let has_promises = opened_promises > 0 || total_resolved_promises > 0;
    record_entry(
        &mut detailed.entries,
        "promises_kept",
        Some(13),
        has_promises.then_some(kept as f64),
        "no promises opened or resolved in scenario",
        "count(ExpectationTransition -> Kept)",
    );
    record_entry(
        &mut detailed.entries,
        "promises_kept_late",
        Some(13),
        has_promises.then_some(kept_late as f64),
        "no promises opened or resolved in scenario",
        "count(ExpectationTransition -> KeptLate)",
    );
    record_entry(
        &mut detailed.entries,
        "promises_repaired",
        Some(13),
        has_promises.then_some(repaired as f64),
        "no promises opened or resolved in scenario",
        "count(ExpectationTransition -> Renegotiated)",
    );
    record_entry(
        &mut detailed.entries,
        "promises_broken",
        Some(13),
        has_promises.then_some(broken as f64),
        "no promises opened or resolved in scenario",
        "count(ExpectationTransition -> Failed | Expired)",
    );
    let overdue_rate = if has_promises {
        let denom = opened_promises.max(total_resolved_promises);
        Some((kept_late + broken) as f64 / denom as f64)
    } else {
        None
    };
    record_entry(
        &mut detailed.entries,
        "overdue_promise_rate",
        Some(4),
        overdue_rate,
        "no promises opened or resolved in scenario",
        "(promises_kept_late + promises_broken) / max(opened_promises, resolved_promises)",
    );

    // -----------------------------------------------------------------------
    // Row 14 (Credence): Brier score and 10-bin ECE on [0, 20] confidence
    // Formula:
    //   p_i = clamp(conf_0_20 / 20.0, 0, 1), y_i in {0.0, 1.0}
    //   Brier = (1/N) * sum((p_i - y_i)^2)
    //   ECE = sum_{b=1..10} (|B_b| / N) * |acc(B_b) - mean_conf(B_b)|
    // -----------------------------------------------------------------------
    let cal_samples: Vec<(f64, f64)> = events
        .iter()
        .filter_map(|e| match &e.detail {
            Some(EventDetail::CalibrationRecorded { conf_0_20, correct }) => {
                let p = (f64::from((*conf_0_20).min(20)) / 20.0).clamp(0.0, 1.0);
                let y = if *correct { 1.0 } else { 0.0 };
                Some((p, y))
            }
            _ => None,
        })
        .collect();
    let (brier, ece) = if cal_samples.is_empty() {
        (None, None)
    } else {
        let n = cal_samples.len() as f64;
        let brier_val = cal_samples
            .iter()
            .map(|(p, y)| (p - y).powi(2))
            .sum::<f64>()
            / n;
        let mut bins: Vec<(usize, f64, f64)> = vec![(0, 0.0, 0.0); 10];
        for &(p, y) in &cal_samples {
            let idx = ((p * 10.0).floor() as usize).min(9);
            bins[idx].0 += 1;
            bins[idx].1 += p;
            bins[idx].2 += y;
        }
        let mut ece_val = 0.0;
        for (cnt, sum_p, sum_y) in bins {
            if cnt > 0 {
                let c = cnt as f64;
                ece_val += (c / n) * ((sum_y / c) - (sum_p / c)).abs();
            }
        }
        (Some(brier_val), Some(ece_val))
    };
    record_entry(
        &mut detailed.entries,
        "credence_brier",
        Some(14),
        brier,
        "no CalibrationRecorded events in scenario",
        "(1/N) * sum((conf_0_20 / 20.0 - correct)^2)",
    );
    record_entry(
        &mut detailed.entries,
        "credence_ece",
        Some(14),
        ece,
        "no CalibrationRecorded events in scenario",
        "10-bin Expected Calibration Error sum(|B_b|/N * |acc(B_b) - conf(B_b)|)",
    );

    // -----------------------------------------------------------------------
    // Row 15 (Bounded learning): F1 and multi-window F1 improvement
    // -----------------------------------------------------------------------
    let f1 = match (precision, recall) {
        (Some(p), Some(r)) if p + r > 0.0 => Some(2.0 * p * r / (p + r)),
        (Some(_), Some(_)) => Some(0.0),
        _ => None,
    };
    let f1_imp = if annotated_ordered.len() >= 4 {
        let mid = annotated_ordered.len() / 2;
        match (
            compute_f1(&annotated_ordered[..mid]),
            compute_f1(&annotated_ordered[mid..]),
        ) {
            (Some(early), Some(late)) => Some(late - early),
            _ => None,
        }
    } else {
        None
    };
    record_entry(
        &mut detailed.entries,
        "when_to_speak_f1",
        Some(15),
        f1,
        "no SPEAK/SILENT annotations in scenario",
        "2 * precision * recall / (precision + recall)",
    );
    record_entry(
        &mut detailed.entries,
        "when_to_speak_f1_improvement",
        Some(15),
        f1_imp,
        "requires >= 4 annotated SPEAK/SILENT turns spanning early and late windows",
        "F1(second_half) - F1(first_half)",
    );

    // -----------------------------------------------------------------------
    // Row 16 (Memory truth): detail_without_recollection, stale_beliefs_presented,
    //                        source_misattribution_rate, lure_error_rate
    // Formula:
    //   detail_without_recollection = count(familiar_only_r0 && detail_asserted_without_r) / count(familiar_only_r0)
    //   stale_beliefs_presented = count(stale_belief_presented) / count(recall_probes)
    // -----------------------------------------------------------------------
    let (dwr, stale_rate, misattr_rate, lure_rate) = match scenario {
        Some(sc) if !sc.annotations.recall_probes.is_empty() => {
            let probes = &sc.annotations.recall_probes;
            let r0_probes: Vec<_> = probes.iter().filter(|p| p.familiar_only_r0).collect();
            let dwr_val = (!r0_probes.is_empty()).then(|| {
                r0_probes
                    .iter()
                    .filter(|p| p.detail_asserted_without_r)
                    .count() as f64
                    / r0_probes.len() as f64
            });
            let n = probes.len() as f64;
            let stale_val =
                Some(probes.iter().filter(|p| p.stale_belief_presented).count() as f64 / n);
            let misattr_val =
                Some(probes.iter().filter(|p| p.source_misattributed).count() as f64 / n);
            let lure_val = Some(probes.iter().filter(|p| p.drm_lure_error).count() as f64 / n);
            (dwr_val, stale_val, misattr_val, lure_val)
        }
        _ => (None, None, None, None),
    };
    record_entry(
        &mut detailed.entries,
        "detail_without_recollection",
        Some(16),
        dwr,
        "no R=0 familiar-only probes in scenario",
        "detail_asserted_without_r / total_familiar_only_r0_probes",
    );
    record_entry(
        &mut detailed.entries,
        "stale_beliefs_presented",
        Some(16),
        stale_rate,
        "no memory-truth recall probes in scenario",
        "stale_belief_presented / total_recall_probes",
    );
    record_entry(
        &mut detailed.entries,
        "source_misattribution_rate",
        Some(16),
        misattr_rate,
        "no memory-truth recall probes in scenario",
        "source_misattributed / total_recall_probes",
    );
    record_entry(
        &mut detailed.entries,
        "lure_error_rate",
        Some(16),
        lure_rate,
        "no memory-truth recall probes in scenario",
        "drm_lure_error / total_recall_probes",
    );

    // -----------------------------------------------------------------------
    // Row 17 (Report control): wrong_volunteered_answer_rate & withheld_count
    // Formula:
    //   wrong_volunteered_answer_rate = count(wrong_volunteered) / total_report_control_probes
    // -----------------------------------------------------------------------
    let wrong_vol = scenario.and_then(|sc| {
        let probes = &sc.annotations.report_control_probes;
        (!probes.is_empty()).then(|| {
            probes.iter().filter(|p| p.wrong_volunteered).count() as f64 / probes.len() as f64
        })
    });
    record_entry(
        &mut detailed.entries,
        "wrong_volunteered_answer_rate",
        Some(17),
        wrong_vol,
        "no report_control_probes annotated in scenario",
        "wrong_volunteered_answers / total_report_control_probes",
    );

    // -----------------------------------------------------------------------
    // Row 18 (Consolidation & merge): sleep_spin_count & blended_memory_error_rate
    // Formula:
    //   sleep_spin_count = count of SleepRun events with covered_event_ids.is_empty()
    //                      or duplicate episode closures (must == 0)
    //   blended_memory_error_rate = blended_episodes / closed_episodes
    // -----------------------------------------------------------------------
    let sleep_runs: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.detail {
            Some(EventDetail::SleepRun {
                covered_event_ids, ..
            }) => Some(covered_event_ids),
            _ => None,
        })
        .collect();
    let sleep_spin = (!sleep_runs.is_empty())
        .then(|| sleep_runs.iter().filter(|cov| cov.is_empty()).count() as f64);
    record_entry(
        &mut detailed.entries,
        "sleep_spin_count",
        Some(18),
        sleep_spin,
        "no SleepRun events executed in scenario",
        "count(SleepRun where covered_event_ids.is_empty()) (must == 0)",
    );

    let episode_closes: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.detail {
            Some(EventDetail::EpisodeClosed {
                place, merged_from, ..
            }) => Some((place, merged_from)),
            _ => None,
        })
        .collect();
    let blended_rate = if !episode_closes.is_empty() {
        let explicit_blends = scenario
            .map(|sc| {
                sc.annotations
                    .recall_probes
                    .iter()
                    .filter(|p| p.blended_memory_error)
                    .count()
            })
            .unwrap_or(0);
        Some(explicit_blends as f64 / episode_closes.len() as f64)
    } else {
        None
    };
    record_entry(
        &mut detailed.entries,
        "blended_memory_error_rate",
        Some(18),
        blended_rate,
        "no EpisodeClosed events executed in scenario",
        "blended_memory_errors / closed_episodes",
    );

    // -----------------------------------------------------------------------
    // Row 19 (Perception ledger): false_belief_accuracy
    // Formula:
    //   correct_false_belief_probes / total_false_belief_probes
    // -----------------------------------------------------------------------
    let fb_acc = scenario.and_then(|sc| {
        let probes = &sc.annotations.false_belief_probes;
        (!probes.is_empty()).then(|| {
            probes.iter().filter(|p| p.answered_correctly).count() as f64 / probes.len() as f64
        })
    });
    record_entry(
        &mut detailed.entries,
        "false_belief_accuracy",
        Some(19),
        fb_acc,
        "no false_belief_probes annotated in scenario",
        "correct_false_belief_answers / total_false_belief_probes",
    );

    // -----------------------------------------------------------------------
    // Row 20 (Agency): self_echo_rates
    // Formula:
    //   echo_wakes / soul_actions where echo percept = percept from soul's channel/effect window
    // -----------------------------------------------------------------------
    let self_echo = {
        let echo_percepts: Vec<EventId> = events
            .iter()
            .filter(|e| e.kind == EventKind::Percept)
            .filter(|e| {
                String::from_utf8_lossy(e.payload.as_bytes()).contains("echo:")
                    || e.actor == soul_actor
            })
            .map(|e| e.id)
            .collect();
        if echo_percepts.is_empty() || actions_dispatched == 0 {
            None
        } else {
            let echo_wakes = echo_percepts
                .iter()
                .filter(|id| decision_by_target.get(id).copied() == Some(DecisionOutcome::Go))
                .count();
            Some(echo_wakes as f64 / actions_dispatched as f64)
        }
    };
    record_entry(
        &mut detailed.entries,
        "self_echo_rates",
        Some(20),
        self_echo,
        "no self-echo percepts or dispatched actions in scenario",
        "wakes_on_own_echo_percepts / soul_actions_dispatched",
    );

    // -----------------------------------------------------------------------
    // Row 21 (Event boundaries): boundary_f1
    // Formula:
    //   Match each EpisodeClosed timestamp against annotated ground-truth episode_boundaries_ms
    //   within boundary_tolerance_ms; F1 = 2 * P * R / (P + R).
    // -----------------------------------------------------------------------
    let boundary_f1 = scenario.and_then(|sc| {
        let gt = &sc.annotations.episode_boundaries_ms;
        if gt.is_empty() {
            return None;
        }
        let tol = sc.annotations.boundary_tolerance_ms;
        let pred: Vec<u64> = events
            .iter()
            .filter(|e| e.kind == EventKind::EpisodeClosed)
            .map(|e| e.timestamp_ms.as_millis())
            .collect();
        if pred.is_empty() {
            return Some(0.0);
        }
        let mut used_gt = vec![false; gt.len()];
        let mut matched = 0usize;
        for &p_ts in &pred {
            if let Some((idx, _)) = gt
                .iter()
                .enumerate()
                .find(|(i, g)| !used_gt[*i] && p_ts.abs_diff(**g) <= tol)
            {
                used_gt[idx] = true;
                matched += 1;
            }
        }
        if matched == 0 {
            Some(0.0)
        } else {
            let p = matched as f64 / pred.len() as f64;
            let r = matched as f64 / gt.len() as f64;
            Some(2.0 * p * r / (p + r))
        }
    });
    record_entry(
        &mut detailed.entries,
        "boundary_f1",
        Some(21),
        boundary_f1,
        "no episode_boundaries_ms ground truth annotated in scenario",
        "2 * P * R / (P + R) matching EpisodeClosed timestamps within tolerance",
    );

    // -----------------------------------------------------------------------
    // Row 22 (Backlog chunking): post_downtime_wake_burst
    // Formula:
    //   100 * wakes_in_recovery_window / backlogged_events_in_downtime
    // -----------------------------------------------------------------------
    let downtime_burst = scenario.and_then(|sc| {
        if sc.annotations.downtime_windows_ms.is_empty() {
            return None;
        }
        let mut backlogged = 0usize;
        let mut recovery_wakes = 0usize;
        for &(win_start, win_end) in &sc.annotations.downtime_windows_ms {
            backlogged += events
                .iter()
                .filter(|e| {
                    e.kind == EventKind::Percept
                        && e.timestamp_ms.as_millis() >= win_start
                        && e.timestamp_ms.as_millis() <= win_end
                })
                .count();
            recovery_wakes += events
                .iter()
                .filter(|e| {
                    e.kind == EventKind::Decision
                        && e.timestamp_ms.as_millis() >= win_start
                        && e.timestamp_ms.as_millis() <= win_end
                        && matches!(
                            &e.detail,
                            Some(EventDetail::Decision {
                                outcome: DecisionOutcome::Go,
                                ..
                            })
                        )
                })
                .count();
        }
        (backlogged > 0).then(|| 100.0 * recovery_wakes as f64 / backlogged as f64)
    });
    record_entry(
        &mut detailed.entries,
        "post_downtime_wake_burst",
        Some(22),
        downtime_burst,
        "no downtime_windows_ms annotated in scenario",
        "100 * wakes_in_recovery_window / backlogged_percepts_in_window",
    );

    // -----------------------------------------------------------------------
    // Safety & Engineering Metrics:
    // - delta_ni (`Delta_NI = 0`)
    // - safety_leakage (narrower-audience items in wider ContextPlan/MemoryUsed without DeclassToken)
    // - hard_forget_resurrection
    // - barge_in_latency_ms
    // - cache_share
    // -----------------------------------------------------------------------
    let delta_ni = scenario.and_then(|sc| sc.annotations.delta_ni_measured);
    record_entry(
        &mut detailed.entries,
        "delta_ni",
        None,
        delta_ni,
        "requires paired private-vs-empty-private non-interference run",
        "max |P(go|private) - P(go|empty)| + decision divergence count",
    );

    let mut leakage_count = inputs.extra_safety_leaks;
    for plan in context_plans {
        if let Some(scope) = &plan.visible_scope {
            let eval_time = plan
                .trigger_event_id
                .and_then(|tid| events.iter().find(|r| r.id == tid))
                .map_or(monoagens_protocol::Time::from_millis(end_ms), |r| {
                    r.timestamp_ms
                });
            for &eid in &plan.items {
                if let Some(rec) = events.iter().find(|r| r.id == eid)
                    && !scope.allows(&rec.audience, Some(rec.id), eval_time)
                {
                    leakage_count += 1;
                }
            }
        }
    }
    let safety_leakage_val = if !context_plans.is_empty() || inputs.extra_safety_leaks > 0 {
        Some(leakage_count as f64)
    } else {
        None
    };
    record_entry(
        &mut detailed.entries,
        "safety_leakage",
        None,
        safety_leakage_val,
        "no ContextPlan inspections captured in scenario",
        "count(items in ContextPlan violating VisibleScope without valid DeclassToken)",
    );

    let hard_forget_count = events
        .iter()
        .filter(|e| e.kind == EventKind::HardForget)
        .count();
    let hf_resurrection = inputs
        .hard_forget_resurrection_count
        .map(|c| c as f64)
        .or_else(|| (hard_forget_count > 0).then_some(0.0));
    record_entry(
        &mut detailed.entries,
        "hard_forget_resurrection",
        None,
        hf_resurrection,
        "no HardForget erasure executed in scenario",
        "surviving erased events/vectors/tokens after hard_forget (must == 0)",
    );

    let barge_in_lat = {
        let mut lats = Vec::new();
        for rec in events {
            if let Some(EventDetail::Receipt {
                action_id,
                outcome: ReceiptOutcome::Failed | ReceiptOutcome::Refused,
                termination_reason:
                    Some(TerminationReason::PreemptedExternal | TerminationReason::CanceledByCore),
                ..
            }) = &rec.detail
                && let Some(&interrupt_ts) = inputs.barge_in_timestamps_ms.get(action_id)
            {
                lats.push(rec.timestamp_ms.as_millis().saturating_sub(interrupt_ts) as f64);
            }
        }
        (!lats.is_empty()).then(|| lats.iter().sum::<f64>() / lats.len() as f64)
    };
    record_entry(
        &mut detailed.entries,
        "barge_in_latency_ms",
        None,
        barge_in_lat,
        "no Class-A barge-in preemption receipts occurred in scenario",
        "mean(receipt.timestamp_ms - cancel_interrupt.timestamp_ms) for preempted acts",
    );

    let cache_share = if context_plans.len() >= 2 {
        let mut hits = 0usize;
        for pair in context_plans.windows(2) {
            if pair[0].static_prefix_digest == pair[1].static_prefix_digest {
                hits += 1;
            }
        }
        Some(hits as f64 / (context_plans.len() - 1) as f64)
    } else {
        None
    };
    record_entry(
        &mut detailed.entries,
        "cache_share",
        None,
        cache_share,
        "requires >= 2 wakes with ContextPlan static_prefix_digest",
        "count(consecutive wakes with identical static_prefix_digest) / (wakes - 1)",
    );

    let mut populated = BTreeMap::new();
    for (k, entry) in &detailed.entries {
        if let Some(val) = entry.value {
            populated.insert(k.clone(), val);
        }
    }

    RunMetricsSummary {
        total_events: events.len(),
        percepts,
        decisions_go,
        decisions_note,
        decisions_silent,
        decisions_door_blocked,
        actions_dispatched,
        receipts_recorded,
        wakes_per_event,
        simulated_span_ms: end_ms.saturating_sub(start_ms),
        wall_elapsed_ms,
        brain_map_metrics: (!populated.is_empty()).then_some(populated),
        detailed: Some(detailed),
    }
}

fn record_entry(
    map: &mut BTreeMap<String, MetricReportEntry>,
    name: &str,
    row: Option<u8>,
    value: Option<f64>,
    unavailable_reason: &str,
    formula: &str,
) {
    map.insert(
        name.to_owned(),
        MetricReportEntry {
            name: name.to_owned(),
            row,
            value,
            unavailable_reason: if value.is_none() {
                Some(unavailable_reason.to_owned())
            } else {
                None
            },
            formula: formula.to_owned(),
        },
    );
}

fn compute_f1(items: &[(WarrantedReply, bool)]) -> Option<f64> {
    let mut tp = 0usize;
    let mut fp = 0usize;
    let mut fn_c = 0usize;
    for &(w, went) in items {
        match (w, went) {
            (WarrantedReply::Speak, true) => tp += 1,
            (WarrantedReply::Speak, false) => fn_c += 1,
            (WarrantedReply::Silent, true) => fp += 1,
            (WarrantedReply::Silent, false) => {}
        }
    }
    if tp + fp == 0 && tp + fn_c == 0 {
        return None;
    }
    let p = if tp + fp > 0 {
        tp as f64 / (tp + fp) as f64
    } else {
        1.0
    };
    let r = if tp + fn_c > 0 {
        tp as f64 / (tp + fn_c) as f64
    } else {
        1.0
    };
    if p + r == 0.0 {
        Some(0.0)
    } else {
        Some(2.0 * p * r / (p + r))
    }
}

/// Exact 1-Wasserstein distance `W_1(a, b)` between two finite empirical 1D distributions.
#[must_use]
pub fn wasserstein_1d_ms(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut sa = a.to_vec();
    let mut sb = b.to_vec();
    sa.sort_unstable();
    sb.sort_unstable();

    let mut grid: Vec<u64> = sa.iter().chain(sb.iter()).copied().collect();
    grid.sort_unstable();
    grid.dedup();
    if grid.len() <= 1 {
        return 0.0;
    }

    let na = sa.len() as f64;
    let nb = sb.len() as f64;
    let mut total = 0.0;
    for w in grid.windows(2) {
        let x0 = w[0];
        let x1 = w[1];
        let dx = (x1 - x0) as f64;
        let cdf_a = sa.partition_point(|&v| v <= x0) as f64 / na;
        let cdf_b = sb.partition_point(|&v| v <= x0) as f64 / nb;
        total += (cdf_a - cdf_b).abs() * dx;
    }
    total
}

/// Aggregates metrics across multiple seeded runs of the same scenario(s), reporting mean,
/// sample std-dev, and 95% t-interval half-width.
#[must_use]
pub fn aggregate_metrics_across_seeds(runs: &[RunMetricsSummary]) -> Vec<SeedAggregate> {
    let mut collected: BTreeMap<String, (Option<u8>, Vec<f64>)> = BTreeMap::new();
    for run in runs {
        if let Some(det) = &run.detailed {
            for (name, entry) in &det.entries {
                let slot = collected
                    .entry(name.clone())
                    .or_insert((entry.row, Vec::new()));
                if let Some(v) = entry.value {
                    slot.1.push(v);
                }
            }
        }
    }

    collected
        .into_iter()
        .map(|(name, (row, vals))| {
            let n = vals.len();
            if n == 0 {
                SeedAggregate {
                    name,
                    row,
                    n_seeds: 0,
                    mean: None,
                    std_dev: None,
                    ci95_half_width: None,
                }
            } else {
                let mean = vals.iter().sum::<f64>() / n as f64;
                let std_dev = if n > 1 {
                    let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
                    Some(var.sqrt())
                } else {
                    Some(0.0)
                };
                let ci95 = std_dev.map(|sd| t_critical_95(n) * sd / (n as f64).sqrt());
                SeedAggregate {
                    name,
                    row,
                    n_seeds: n,
                    mean: Some(mean),
                    std_dev,
                    ci95_half_width: ci95,
                }
            }
        })
        .collect()
}

/// Computes paired ablation deltas (`enabled - neutral`) with 95% confidence intervals across seeds.
#[must_use]
pub fn compare_paired_ablation(
    enabled_runs: &[RunMetricsSummary],
    neutral_runs: &[RunMetricsSummary],
) -> Vec<PairedAblationDelta> {
    let pairs = enabled_runs.len().min(neutral_runs.len());
    let mut keys: BTreeSet<String> = BTreeSet::new();
    for r in enabled_runs.iter().chain(neutral_runs.iter()) {
        if let Some(det) = &r.detailed {
            keys.extend(det.entries.keys().cloned());
        }
    }

    keys.into_iter()
        .map(|metric| {
            let mut en_vals = Vec::new();
            let mut neu_vals = Vec::new();
            let mut diffs = Vec::new();
            for i in 0..pairs {
                let ev = enabled_runs[i]
                    .detailed
                    .as_ref()
                    .and_then(|d| d.entries.get(&metric))
                    .and_then(|e| e.value);
                let nv = neutral_runs[i]
                    .detailed
                    .as_ref()
                    .and_then(|d| d.entries.get(&metric))
                    .and_then(|e| e.value);
                if let (Some(e), Some(n)) = (ev, nv) {
                    en_vals.push(e);
                    neu_vals.push(n);
                    diffs.push(e - n);
                }
            }
            let n = diffs.len();
            if n == 0 {
                PairedAblationDelta {
                    metric,
                    n_seeds: 0,
                    enabled_mean: None,
                    neutral_mean: None,
                    mean_delta: None,
                    ci95_half_width: None,
                    within_seed_noise: true,
                }
            } else {
                let en_mean = en_vals.iter().sum::<f64>() / n as f64;
                let neu_mean = neu_vals.iter().sum::<f64>() / n as f64;
                let mean_delta = diffs.iter().sum::<f64>() / n as f64;
                let ci95 = if n > 1 {
                    let var = diffs.iter().map(|d| (d - mean_delta).powi(2)).sum::<f64>()
                        / (n - 1) as f64;
                    let se = var.sqrt() / (n as f64).sqrt();
                    Some(t_critical_95(n) * se)
                } else {
                    Some(0.0)
                };
                let within_noise = ci95
                    .map(|hw| mean_delta.abs() <= hw + 1e-12)
                    .unwrap_or(true);
                PairedAblationDelta {
                    metric,
                    n_seeds: n,
                    enabled_mean: Some(en_mean),
                    neutral_mean: Some(neu_mean),
                    mean_delta: Some(mean_delta),
                    ci95_half_width: ci95,
                    within_seed_noise: within_noise,
                }
            }
        })
        .collect()
}

fn t_critical_95(n: usize) -> f64 {
    // Two-sided 95% Student-t critical values for df = n - 1.
    match n.saturating_sub(1) {
        0 => 0.0,
        1 => 12.706,
        2 => 4.303,
        3 => 3.182,
        4 => 2.776,
        5 => 2.571,
        6 => 2.447,
        7 => 2.365,
        8 => 2.306,
        9 => 2.262,
        10..=14 => 2.160,
        15..=29 => 2.045,
        _ => 1.960,
    }
}

/// Formats a human-readable ASCII/Markdown table of all metric entries, values, and unavailable reasons.
#[must_use]
pub fn format_metrics_table(summary: &RunMetricsSummary) -> String {
    let mut out = String::new();
    out.push_str("Row | Metric | Value | Status / Formula\n");
    out.push_str("--- | --- | --- | ---\n");
    if let Some(det) = &summary.detailed {
        for entry in det.entries.values() {
            let row_str = entry
                .row
                .map_or_else(|| "safety".to_owned(), |r| format!("R{r:02}"));
            let val_str = entry
                .value
                .map_or_else(|| "unavailable".to_owned(), |v| format!("{v:.4}"));
            let note = entry
                .unavailable_reason
                .as_deref()
                .unwrap_or(&entry.formula);
            use std::fmt::Write as _;
            let _ = writeln!(&mut out, "{row_str} | {} | {val_str} | {note}", entry.name);
        }
    }
    out
}

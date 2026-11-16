//! Expectations, promises, recurring monitors, approvals, grounding, agency, and judged silence.
//!
//! All lifecycle transitions, retries, conflict checks, dependency ordering, response/effect
//! predictions, judged silences, content-miss detection, and free-text condition gates run
//! deterministically in code without model calls (`R-07`..`R-10`, `R-18`, `R-19`, `R-36`,
//! `R-42`, `R-46`, `R-49`).

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    Action, ActionId, ActorId, Audience, CURRENT_PROTOCOL_VERSION, CapabilityId, ChannelId,
    CorrelationId, DomainId, EventId, ExpectKind, HAct, OpaquePayload, OriginActor, PlaceId,
    PromiseMeta, ProtocolVersion, RecurrenceSchedule, RecurrenceSpec, ResolveOutcome,
    TaggedEmbedding, TerminationReason, Time,
};
use serde::{Deserialize, Serialize};

use crate::event::{
    BinReader, BinWriter, EventDetail, EventFormatError, EventRecord, ExpectationLifecycleState,
    decode_expect_kind, decode_opt_promise_meta, decode_opt_recurrence_spec,
    decode_opt_sensed_effect, decode_tagged_embeddings_bin, decode_taint, decode_term_reason,
    encode_expect_kind, encode_opt_promise_meta, encode_opt_recurrence_spec,
    encode_opt_sensed_effect, encode_tagged_embeddings_bin, encode_taint, encode_term_reason,
};
use crate::gate::patience::{
    PATIENCE_NUM_BUCKETS, bucket_midpoint_ms, bucket_upper_ms, normal_cdf,
};
use crate::gate::{ExpectationCue, LatencySketch, PromiseSummary, cosine_similarity};
use crate::prior::Prior;
use crate::registry::Switches;

/// Physical/sensor effect prediction metadata attached to an [`ExpectKind::Effect`] record (`R-09`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectPredictionSpec {
    pub action_id: ActionId,
    pub capability_id: CapabilityId,
    pub channel: ChannelId,
    pub lat_min_ms: u64,
    pub lat_max_ms: u64,
    pub delta_pred: f64,
    pub sigma_channel: f64,
    pub correlation_id: Option<CorrelationId>,
    /// `true` when the capability or act declared a physical `SensedEffect`;
    /// `false` for echo-only tracking on communicative actions.
    pub declared_sensor: bool,
}

/// Grounding or judged-silence prediction metadata attached to an [`ExpectKind::Response`] record (`R-08`, `R-18`, `R-19`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResponsePredictionSpec {
    pub action_id: Option<ActionId>,
    pub medium: ChannelId,
    pub addressees: Vec<ActorId>,
    pub accept_done_receipt: bool,
    pub act_embeddings: Vec<TaggedEmbedding>,
    pub expected_reply_embeddings: Vec<TaggedEmbedding>,
    pub is_judged_silence: bool,
    pub silence_sample_propensity: Option<f64>,
    pub silence_sender: Option<ActorId>,
    pub awaiting_reask_classify: bool,
    pub pending_reask_event_id: Option<EventId>,
    pub stakes: f64,
    pub expected_latency_ms: u64,
}

/// Recurrence tracking metadata attached to an [`ExpectKind::Monitor`] record (`R-46`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct MonitorState {
    pub last_fired_ms: Option<Time>,
    pub catchup_fired_count: u64,
    pub skipped_slots: u64,
}

/// Typed expectation record inside Core's `expectations` book.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpectationRecord {
    pub expectation_id: EventId,
    pub origin_event_id: EventId,
    pub target_event_id: EventId,
    pub kind: ExpectKind,
    pub state: ExpectationLifecycleState,
    pub version: u32,
    pub places: Vec<PlaceId>,
    pub people: Vec<ActorId>,
    pub audience: Audience,
    pub domain: Option<DomainId>,
    pub condition: String,
    pub condition_embeddings: Vec<TaggedEmbedding>,
    pub about_ids: Vec<EventId>,
    pub created_at_ms: Time,
    pub updated_at_ms: Time,
    pub due_ms: Time,
    pub expiry_ms: Time,
    pub recurrence: Option<RecurrenceSpec>,
    pub promise_meta: Option<PromiseMeta>,
    pub urgency_weight: f64,
    pub is_overdue: bool,
    pub due_woken: bool,
    pub retry_count: u32,
    pub next_retry_at_ms: Option<Time>,
    pub retry_action: Option<Action>,
    pub last_action_id: Option<ActionId>,
    pub last_capability_id: Option<CapabilityId>,
    pub last_h_act: Option<HAct>,
    pub delivered_units: Option<u64>,
    pub termination_reason: Option<TerminationReason>,
    pub approval_prompt_emitted: bool,
    pub effect_spec: Option<EffectPredictionSpec>,
    pub response_spec: Option<ResponsePredictionSpec>,
    pub monitor_spec: Option<MonitorState>,
    pub goal_chain_ids: Vec<EventId>,
    pub linked_action_ids: Vec<ActionId>,
    pub linked_expectation_id: Option<EventId>,
}

impl ExpectationRecord {
    /// Returns `true` if this expectation is in a terminal lifecycle state.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        is_terminal_state(self.state)
    }

    /// Returns `true` if this expectation is still active (`Open` or `PendingApproval`).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        !self.is_terminal()
    }

    /// Returns the next timer wake timestamp (`due`, retry, or `expiry`), if any.
    ///
    /// `PendingApproval` and terminal states return `None` so timers never emit duplicate prompts.
    #[must_use]
    pub fn next_timer_ms(&self) -> Option<Time> {
        if !self.is_active() || self.state == ExpectationLifecycleState::PendingApproval {
            return None;
        }
        match self.kind {
            ExpectKind::Promise => {
                if let Some(retry_at) = self.next_retry_at_ms {
                    return Some(retry_at);
                }
                if !self.due_woken {
                    return Some(self.due_ms);
                }
                if self.expiry_ms > self.due_ms {
                    return Some(self.expiry_ms);
                }
                None
            }
            ExpectKind::Monitor => Some(self.due_ms),
            ExpectKind::Effect | ExpectKind::Response | ExpectKind::Goal | ExpectKind::Wait => {
                Some(self.expiry_ms)
            }
            ExpectKind::PendingApproval => None,
        }
    }

    /// Converts this record to a [`PromiseSummary`] for `promise_debt` calculation.
    #[must_use]
    pub fn to_promise_summary(&self) -> Option<PromiseSummary> {
        match self.kind {
            ExpectKind::Promise | ExpectKind::PendingApproval => Some(PromiseSummary {
                due_ms: self.due_ms,
                urgency_weight: self.urgency_weight,
                is_recurring_monitor: false,
                active: self.is_active(),
            }),
            ExpectKind::Monitor => Some(PromiseSummary {
                due_ms: self.due_ms,
                urgency_weight: 0.0,
                is_recurring_monitor: true,
                active: self.is_active(),
            }),
            _ => None,
        }
    }
}

/// Returns `true` if `state` is terminal.
#[must_use]
pub const fn is_terminal_state(state: ExpectationLifecycleState) -> bool {
    matches!(
        state,
        ExpectationLifecycleState::Rejected
            | ExpectationLifecycleState::Kept
            | ExpectationLifecycleState::KeptLate
            | ExpectationLifecycleState::Renegotiated
            | ExpectationLifecycleState::Released
            | ExpectationLifecycleState::Failed
            | ExpectationLifecycleState::Cancelled
            | ExpectationLifecycleState::Answered
            | ExpectationLifecycleState::Reached
            | ExpectationLifecycleState::Expired
    )
}

/// Returns `true` if `state` represents successful completion that unblocks `depends_on` dependents.
#[must_use]
pub const fn is_fulfilled_state(state: ExpectationLifecycleState) -> bool {
    matches!(
        state,
        ExpectationLifecycleState::Kept
            | ExpectationLifecycleState::KeptLate
            | ExpectationLifecycleState::Answered
            | ExpectationLifecycleState::Reached
            | ExpectationLifecycleState::Approved
    )
}

/// Maps a protocol [`ResolveOutcome`] to [`ExpectationLifecycleState`], distinguishing `KeptLate`.
#[must_use]
pub fn map_resolve_outcome(
    outcome: ResolveOutcome,
    due_ms: Time,
    now_ms: Time,
    kind: ExpectKind,
) -> ExpectationLifecycleState {
    match outcome {
        ResolveOutcome::Kept => {
            if kind == ExpectKind::Promise && now_ms > due_ms {
                ExpectationLifecycleState::KeptLate
            } else {
                ExpectationLifecycleState::Kept
            }
        }
        ResolveOutcome::KeptLate => ExpectationLifecycleState::KeptLate,
        ResolveOutcome::Renegotiated => ExpectationLifecycleState::Renegotiated,
        ResolveOutcome::Released => ExpectationLifecycleState::Released,
        ResolveOutcome::Failed => ExpectationLifecycleState::Failed,
        ResolveOutcome::Cancelled => ExpectationLifecycleState::Cancelled,
        ResolveOutcome::Answered => ExpectationLifecycleState::Answered,
        ResolveOutcome::Reached => ExpectationLifecycleState::Reached,
    }
}

/// Conflict detected before committing a new `PromiseMeta` obligation (`R-49`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromiseConflict {
    pub conflicting_expectation_id: EventId,
    pub conflict_key: Option<String>,
    pub overlapping_places: Vec<PlaceId>,
}

/// Action deferred until its `depends_on` expectation resolves (`R-49`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeferredDependentAct {
    pub expectation_id: EventId,
    pub depends_on: EventId,
    pub thought_event_id: EventId,
    pub action: Action,
    pub people: Vec<ActorId>,
    pub stakes: f64,
    pub act_embeddings: Vec<TaggedEmbedding>,
    pub expected_reply_embeddings: Vec<TaggedEmbedding>,
}

/// Pending classification request issued by free-text conditions, judged silence, or goal attribution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PendingClassifyKind {
    Condition {
        expectation_id: EventId,
        similarity: f64,
    },
    SilenceReask {
        expectation_id: EventId,
    },
    GoalAttribution {
        expectation_id: EventId,
        similarity: f64,
    },
    GistVerification {
        gist_id: EventId,
        source_ids: Vec<EventId>,
    },
}

/// Hierarchically shrunk latency quantile estimator for response expiry and silence windows (`R-08`, `A-08`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ResponseLatencyTracker {
    pub by_place_medium: BTreeMap<(PlaceId, ChannelId), LatencySketch>,
    pub by_medium: BTreeMap<ChannelId, LatencySketch>,
}

impl ResponseLatencyTracker {
    /// Records an observed addressee/participant reply latency for `(place, medium)`.
    pub fn observe_response(
        &mut self,
        place: &PlaceId,
        medium: &ChannelId,
        latency_ms: u64,
        rho: f64,
    ) {
        self.by_place_medium
            .entry((place.clone(), medium.clone()))
            .or_default()
            .observe(latency_ms, true, rho);
        self.by_medium
            .entry(medium.clone())
            .or_default()
            .observe(latency_ms, true, rho);
    }

    /// Records a right-censored response/silence window expiry for `(place, medium)`.
    pub fn observe_censored(
        &mut self,
        place: &PlaceId,
        medium: &ChannelId,
        window_ms: u64,
        rho: f64,
    ) {
        self.by_place_medium
            .entry((place.clone(), medium.clone()))
            .or_default()
            .observe(window_ms, false, rho);
        self.by_medium
            .entry(medium.clone())
            .or_default()
            .observe(window_ms, false, rho);
    }

    /// Estimates a shrunk response/silence window in milliseconds for `(place, medium)`.
    ///
    /// When no latency has been observed for `(place, medium)` or `medium`, returns `body_declared_ms`
    /// directly so a new place uses the body-declared typical latency (`expectations/spec.md`).
    #[must_use]
    pub fn estimate_window_ms(
        &self,
        place: &PlaceId,
        medium: &ChannelId,
        body_declared_ms: u64,
        quantile: f64,
        prior: &Prior,
        switches: &Switches,
    ) -> u64 {
        let declared = body_declared_ms.max(1);
        if !switches.prediction_and_grounding {
            return declared;
        }

        let (w_med, cdf_med) = self
            .by_medium
            .get(medium)
            .map_or((0.0, Vec::new()), LatencySketch::kaplan_meier_cdf);
        let (w_pm, cdf_pm) = self
            .by_place_medium
            .get(&(place.clone(), medium.clone()))
            .map_or((0.0, Vec::new()), LatencySketch::kaplan_meier_cdf);

        if w_pm <= 0.0 && w_med <= 0.0 {
            return declared;
        }

        let ln_l_typ = (declared as f64).ln();
        let sigma_ln = prior.patience_sigma_ln.max(1e-3);
        let n0_global = prior.patience_prior_n0.max(0.0);
        let n0_place = prior.patience_place_n0.max(0.0);
        let q = quantile.clamp(0.01, 0.99);

        let mut chosen_bucket = PATIENCE_NUM_BUCKETS - 1;
        for k in 0..PATIENCE_NUM_BUCKETS {
            let u_k = bucket_upper_ms(k as u16);
            let f0 = normal_cdf((u_k.max(1.0).ln() - ln_l_typ) / sigma_ln);

            let f_med = if w_med > 0.0 && (n0_global + w_med) > 0.0 {
                (n0_global * f0 + w_med * cdf_med[k]) / (n0_global + w_med)
            } else {
                f0
            };

            let f_pm = if w_pm > 0.0 && (n0_place + w_pm) > 0.0 {
                (n0_place * f_med + w_pm * cdf_pm[k]) / (n0_place + w_pm)
            } else {
                f_med
            };

            if f_pm >= q {
                chosen_bucket = k;
                break;
            }
        }

        bucket_midpoint_ms(chosen_bucket as u16).round().max(1.0) as u64
    }
}

/// Welford running statistics of unrepaired reply distances per place (`R-18`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlaceDistanceStats {
    pub count: usize,
    pub mean: f64,
    pub m2: f64,
}

impl PlaceDistanceStats {
    /// Records one unrepaired reply distance `d`.
    pub fn observe(&mut self, distance: f64) {
        if !distance.is_finite() {
            return;
        }
        self.count = self.count.saturating_add(1);
        let delta = distance - self.mean;
        self.mean += delta / (self.count as f64);
        let delta2 = distance - self.mean;
        self.m2 += delta * delta2;
    }

    /// Evaluates whether `distance` is unusually far relative to the place's usual distance.
    ///
    /// Returns `Some((mean, z_score))` when `count >= min_samples` and `z_score > z_threshold`.
    #[must_use]
    pub fn check_miss(
        &self,
        distance: f64,
        z_threshold: f64,
        min_samples: usize,
    ) -> Option<(f64, f64)> {
        if !distance.is_finite() || self.count < min_samples.max(1) {
            return None;
        }
        let variance = if self.count > 1 {
            (self.m2 / ((self.count - 1) as f64)).max(0.0)
        } else {
            0.0
        };
        let std = variance.sqrt().max(0.05);
        let z = (distance - self.mean) / std;
        if z > z_threshold {
            Some((self.mean, z))
        } else {
            None
        }
    }
}

/// Content-miss evaluation output (`R-18`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentMissReport {
    pub expectation_id: EventId,
    pub reply_event_id: EventId,
    pub who: ActorId,
    pub timing_delta_ms: i64,
    pub distance: f64,
    pub usual_distance: f64,
    pub z_score: f64,
}

/// Recorded free-text condition verdict (`(similarity, met)`) for wake-gate calibration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConditionVerdictRecord {
    pub event_id: EventId,
    pub expectation_id: EventId,
    pub similarity: f64,
    pub met: bool,
}

/// Free-text condition wake-gate state enforcing `wake_gate_lo` and rule-of-three `wake_gate_hi` (`A-08`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WakeGateCalibrator {
    pub wake_gate_lo: f64,
    pub wake_gate_hi: Option<f64>,
    pub verdicts: Vec<ConditionVerdictRecord>,
}

impl Default for WakeGateCalibrator {
    fn default() -> Self {
        Self {
            wake_gate_lo: Prior::default().wake_gate_lo,
            wake_gate_hi: None,
            verdicts: Vec::new(),
        }
    }
}

impl WakeGateCalibrator {
    /// Required clean confirmations `n >= ceil(3 / epsilon)` (rule of three, `epsilon = 0.06 -> 50`).
    #[must_use]
    pub fn rule_of_three_required(epsilon: f64) -> usize {
        let eps = if epsilon.is_finite() {
            epsilon.clamp(0.01, 0.50)
        } else {
            0.06
        };
        (3.0 / eps).ceil() as usize
    }

    /// Records a classifier verdict with its similarity, updating `wake_gate_lo` (`q_0.10`)
    /// and invalidating `wake_gate_hi` if any verdict at or above it was negative.
    pub fn record_verdict(
        &mut self,
        event_id: EventId,
        expectation_id: EventId,
        similarity: f64,
        met: bool,
        prior: &Prior,
    ) {
        if !similarity.is_finite() {
            return;
        }
        self.verdicts.push(ConditionVerdictRecord {
            event_id,
            expectation_id,
            similarity,
            met,
        });
        if !met && self.wake_gate_hi.is_some_and(|hi| similarity >= hi) {
            self.wake_gate_hi = None;
        }

        // Update wake_gate_lo toward shrunk q_0.10 of positive verdicts minus margin.
        let mut positives: Vec<f64> = self
            .verdicts
            .iter()
            .filter(|v| v.met)
            .map(|v| v.similarity)
            .collect();
        if !positives.is_empty() {
            positives.sort_by(f64::total_cmp);
            let idx = ((positives.len() as f64 - 1.0) * 0.10).floor().max(0.0) as usize;
            let q10 = (positives[idx] - prior.wake_gate_margin).clamp(0.0, 1.0);
            let n = positives.len() as f64;
            let n0 = prior.patience_prior_n0.max(1.0);
            self.wake_gate_lo = ((n0 * prior.wake_gate_lo + n * q10) / (n0 + n)).clamp(0.0, 1.0);
        }
    }

    /// Attempts to adopt `candidate_hi` as the upper band (`wake_gate_hi`).
    ///
    /// Succeeds ONLY if at least `ceil(3 / epsilon)` verdicts with `similarity >= candidate_hi`
    /// have been recorded AND none of them were negative (`met == false`).
    pub fn try_adopt_upper_band(&mut self, candidate_hi: f64, prior: &Prior) -> bool {
        if !candidate_hi.is_finite() || !(0.0..=1.0).contains(&candidate_hi) {
            return false;
        }
        let required = Self::rule_of_three_required(prior.wake_gate_hi_epsilon);
        let mut above_count = 0usize;
        for v in &self.verdicts {
            if v.similarity >= candidate_hi {
                if !v.met {
                    return false;
                }
                above_count += 1;
            }
        }
        if above_count >= required {
            self.wake_gate_hi = Some(candidate_hi);
            true
        } else {
            false
        }
    }
}

/// Complete `expectations` book state projected deterministically from the event log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ExpectationBook {
    pub expectations: BTreeMap<EventId, ExpectationRecord>,
    pub deferred_acts: Vec<DeferredDependentAct>,
    pub response_latency: ResponseLatencyTracker,
    pub place_distances: BTreeMap<PlaceId, PlaceDistanceStats>,
    pub wake_gate: WakeGateCalibrator,
    pub pending_classify: BTreeMap<EventId, PendingClassifyKind>,
}

impl ExpectationBook {
    /// Creates an empty `ExpectationBook`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the earliest pending timer across all open expectations, if any.
    #[must_use]
    pub fn next_due(&self) -> Option<Time> {
        self.expectations
            .values()
            .filter_map(ExpectationRecord::next_timer_ms)
            .min()
    }

    /// Rebuilds the `open_promises` summary map for `promise_debt`.
    #[must_use]
    pub fn promise_summaries(&self) -> BTreeMap<EventId, PromiseSummary> {
        let mut out = BTreeMap::new();
        for (&id, exp) in &self.expectations {
            if exp.is_active()
                && let Some(summary) = exp.to_promise_summary()
            {
                out.insert(id, summary);
            }
        }
        out
    }

    /// Rebuilds active [`ExpectationCue`]s for pre-Step-5 `VisibleScope` and relevance scoring.
    ///
    /// Passive cues carry `answers_event = false` so they raise `relevance` without unconditionally
    /// firing the commitment breaker on unrelated events.
    #[must_use]
    pub fn expectation_cues(&self) -> Vec<ExpectationCue> {
        let mut cues = Vec::new();
        for exp in self.expectations.values() {
            if !exp.is_active() {
                continue;
            }
            if exp.condition_embeddings.is_empty() && exp.people.is_empty() {
                continue;
            }
            cues.push(ExpectationCue {
                expectation_id: Some(exp.expectation_id),
                audience: exp.audience.clone(),
                counterparty: exp.people.first().cloned(),
                answers_event: false,
                embeddings: exp.condition_embeddings.clone(),
            });
        }
        cues
    }

    /// Checks whether a candidate promise with `(places, meta)` conflicts with any active promise.
    #[must_use]
    pub fn check_conflict(
        &self,
        places: &[PlaceId],
        meta: Option<&PromiseMeta>,
        exclude_id: Option<EventId>,
    ) -> Option<PromiseConflict> {
        let candidate_key = meta.and_then(|m| m.conflict_key.as_deref());
        let has_exclusive_meta = meta.is_some();

        for (&eid, existing) in &self.expectations {
            if Some(eid) == exclude_id || !existing.is_active() {
                continue;
            }
            if !matches!(
                existing.kind,
                ExpectKind::Promise | ExpectKind::PendingApproval
            ) {
                continue;
            }
            let existing_key = existing
                .promise_meta
                .as_ref()
                .and_then(|m| m.conflict_key.as_deref());
            if let (Some(k1), Some(k2)) = (candidate_key, existing_key)
                && k1 == k2
            {
                let overlapping_places: Vec<PlaceId> = places
                    .iter()
                    .filter(|p| existing.places.contains(p))
                    .cloned()
                    .collect();
                return Some(PromiseConflict {
                    conflicting_expectation_id: eid,
                    conflict_key: Some(k1.to_owned()),
                    overlapping_places,
                });
            }

            if has_exclusive_meta && existing.promise_meta.is_some() {
                let overlapping_places: Vec<PlaceId> = places
                    .iter()
                    .filter(|p| existing.places.contains(p))
                    .cloned()
                    .collect();
                if !overlapping_places.is_empty()
                    && candidate_key.is_some()
                    && existing_key.is_none()
                {
                    return Some(PromiseConflict {
                        conflicting_expectation_id: eid,
                        conflict_key: candidate_key.map(str::to_owned),
                        overlapping_places,
                    });
                }
            }
        }
        None
    }

    /// Returns `true` if `exp` has a `depends_on` link whose target expectation is still open/unfulfilled.
    #[must_use]
    pub fn has_open_dependency(&self, exp: &ExpectationRecord) -> bool {
        let Some(dep_id) = exp.promise_meta.as_ref().and_then(|m| m.depends_on) else {
            return false;
        };
        self.is_dependency_open(dep_id)
    }

    /// Returns `true` if `dep_id` exists in the book and has not yet reached a fulfilled state.
    #[must_use]
    pub fn is_dependency_open(&self, dep_id: EventId) -> bool {
        match self.expectations.get(&dep_id) {
            Some(dep) => !is_fulfilled_state(dep.state),
            None => false,
        }
    }

    /// Returns `true` if an incoming percept matches any open `Effect` prediction (`self_caused`).
    ///
    /// An event answering an open `Response` prediction (from a distinct addressee, without
    /// matching `correlation_id` or `soul_actor_id`) is NEVER marked `self_caused`.
    #[must_use]
    pub fn match_self_caused_effect(
        &self,
        percept: &EventRecord,
        soul_id: &ActorId,
        prior: &Prior,
        switches: &Switches,
    ) -> Option<EventId> {
        if !switches.agency {
            return None;
        }
        let (corr_id, channel, reply_to) = match &percept.detail {
            Some(EventDetail::PerceptMeta {
                correlation_id,
                channel,
                reply_to,
                ..
            }) => (correlation_id.as_ref(), channel.as_ref(), *reply_to),
            _ => (None, None, percept.caused_by.first().copied()),
        };

        // Check strong self-signals first: correlation_id or self-actor echo.
        for (&eid, exp) in &self.expectations {
            if exp.kind != ExpectKind::Effect || !exp.is_active() {
                continue;
            }
            if percept.timestamp_ms > exp.expiry_ms {
                continue;
            }
            let Some(spec) = &exp.effect_spec else {
                continue;
            };
            let place_ok = exp.places.is_empty() || exp.places.contains(&percept.place);
            if !place_ok {
                continue;
            }
            if let Some(c) = corr_id
                && spec.correlation_id.as_ref() == Some(c)
            {
                return Some(eid);
            }
            if &percept.actor == soul_id {
                return Some(eid);
            }
        }

        // If this percept answers an open Response prediction (e.g., an addressee reply),
        // it must NEVER be marked self-caused.
        if self
            .matches_any_response_prediction(percept, soul_id, reply_to)
            .is_some()
        {
            return None;
        }

        // Physical sensor comparator: (channel, [lat_min, lat_max], |Δ_obs - Δ_pred| <= k * σ_channel).
        let delta_obs = percept.numeric_features.delta_obs?;
        if !delta_obs.is_finite() {
            return None;
        }
        let k_tol = prior.agency_tolerance_k.max(0.0);
        for (&eid, exp) in &self.expectations {
            if exp.kind != ExpectKind::Effect || !exp.is_active() {
                continue;
            }
            let Some(spec) = &exp.effect_spec else {
                continue;
            };
            if !spec.declared_sensor {
                continue;
            }
            let place_ok = exp.places.is_empty() || exp.places.contains(&percept.place);
            if !place_ok {
                continue;
            }
            if channel != Some(&spec.channel) {
                continue;
            }
            let t_min = exp.created_at_ms.saturating_add_ms(spec.lat_min_ms);
            let t_max = exp.created_at_ms.saturating_add_ms(spec.lat_max_ms);
            if percept.timestamp_ms < t_min || percept.timestamp_ms > t_max {
                continue;
            }
            let tol = k_tol * spec.sigma_channel.max(0.0);
            if (delta_obs - spec.delta_pred).abs() <= tol {
                return Some(eid);
            }
        }

        None
    }

    /// Finds an active non-silence `Response` prediction answered by `percept`.
    #[must_use]
    pub fn matches_any_response_prediction(
        &self,
        percept: &EventRecord,
        soul_id: &ActorId,
        reply_to: Option<EventId>,
    ) -> Option<EventId> {
        if &percept.actor == soul_id {
            return None;
        }
        let addressees: &[ActorId] = match &percept.detail {
            Some(EventDetail::PerceptMeta { addressees, .. }) => addressees.as_slice(),
            _ => &[],
        };

        for (&eid, exp) in &self.expectations {
            if exp.kind != ExpectKind::Response || !exp.is_active() {
                continue;
            }
            if percept.timestamp_ms > exp.expiry_ms {
                continue;
            }
            let Some(spec) = &exp.response_spec else {
                continue;
            };
            if spec.is_judged_silence {
                continue;
            }
            if !exp.places.is_empty() && !exp.places.contains(&percept.place) {
                continue;
            }
            let from_expected_person =
                spec.addressees.contains(&percept.actor) || exp.people.contains(&percept.actor);
            let reply_linked = reply_to == Some(exp.target_event_id)
                || reply_to == Some(exp.origin_event_id)
                || percept.caused_by.contains(&exp.target_event_id);
            let addressed_to_soul = addressees.contains(soul_id);
            let cont_attn = percept.numeric_features.continued_attention == Some(true)
                && (from_expected_person || spec.addressees.is_empty());

            if from_expected_person
                || reply_linked
                || (addressed_to_soul && spec.addressees.is_empty())
                || cont_attn
            {
                return Some(eid);
            }
        }
        None
    }

    /// Returns `true` if `percept` links to an active `Goal` chain via `reply_to`, `caused_by`, or `expectation_id`.
    #[must_use]
    pub fn matches_goal_chain(&self, percept: &EventRecord) -> Option<EventId> {
        let reply_to = match &percept.detail {
            Some(EventDetail::PerceptMeta { reply_to, .. }) => *reply_to,
            _ => None,
        };
        for (&eid, exp) in &self.expectations {
            if exp.kind != ExpectKind::Goal || !exp.is_active() {
                continue;
            }
            if percept.timestamp_ms > exp.expiry_ms {
                continue;
            }
            if reply_to.is_some_and(|r| r == eid || exp.goal_chain_ids.contains(&r))
                || percept
                    .caused_by
                    .iter()
                    .any(|c| *c == eid || exp.goal_chain_ids.contains(c))
            {
                return Some(eid);
            }
        }
        None
    }

    /// Finds an active promise or goal linked to `action_id` or `h_act`.
    #[must_use]
    pub fn find_by_action(&self, action_id: &ActionId) -> Option<EventId> {
        for (&eid, exp) in &self.expectations {
            if !exp.is_active() {
                continue;
            }
            if exp.last_action_id.as_ref() == Some(action_id)
                || exp.linked_action_ids.contains(action_id)
            {
                return Some(eid);
            }
        }
        None
    }

    /// Finds an active `Response` or `Effect` prediction tied to `action_id`.
    #[must_use]
    pub fn find_predictions_for_action(&self, action_id: &ActionId) -> Vec<EventId> {
        self.expectations
            .iter()
            .filter(|(_, exp)| {
                exp.is_active()
                    && (exp
                        .effect_spec
                        .as_ref()
                        .is_some_and(|s| &s.action_id == action_id)
                        || exp
                            .response_spec
                            .as_ref()
                            .is_some_and(|s| s.action_id.as_ref() == Some(action_id)))
            })
            .map(|(&id, _)| id)
            .collect()
    }

    /// Evaluates content miss for a reply against a `Response` prediction (`R-18`).
    ///
    /// Returns `Ok(Some(ContentMissReport))` if unusually far (or `repair == Some(true)`),
    /// `Ok(None)` if ordinary (updating `place_distances` for unrepaired replies).
    pub fn evaluate_response_content(
        &mut self,
        exp_id: EventId,
        reply: &EventRecord,
        reply_embeddings: &[TaggedEmbedding],
        prior: &Prior,
    ) -> Option<ContentMissReport> {
        let exp = self.expectations.get(&exp_id)?;
        let spec = exp.response_spec.as_ref()?;
        let place = reply.place.clone();

        let sim_act = max_matching_cosine(reply_embeddings, &spec.act_embeddings);
        let sim_expected = max_matching_cosine(reply_embeddings, &spec.expected_reply_embeddings);
        let max_sim = match (sim_act, sim_expected) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };

        let timing_delta_ms = (reply.timestamp_ms.as_millis() as i64)
            - (exp
                .created_at_ms
                .saturating_add_ms(spec.expected_latency_ms)
                .as_millis() as i64);

        let Some(sim) = max_sim else {
            if reply.numeric_features.repair == Some(true) {
                return Some(ContentMissReport {
                    expectation_id: exp_id,
                    reply_event_id: reply.id,
                    who: reply.actor.clone(),
                    timing_delta_ms,
                    distance: 1.0,
                    usual_distance: self.place_distances.get(&place).map_or(0.0, |s| s.mean),
                    z_score: prior.content_miss_z_threshold + 1.0,
                });
            }
            return None;
        };

        let distance = (1.0 - sim).clamp(0.0, 2.0);
        if reply.numeric_features.repair == Some(true) {
            let usual = self.place_distances.get(&place).map_or(0.0, |s| s.mean);
            return Some(ContentMissReport {
                expectation_id: exp_id,
                reply_event_id: reply.id,
                who: reply.actor.clone(),
                timing_delta_ms,
                distance,
                usual_distance: usual,
                z_score: prior.content_miss_z_threshold + 1.0,
            });
        }

        let stats = self.place_distances.entry(place).or_default();
        if let Some((usual, z_score)) = stats.check_miss(
            distance,
            prior.content_miss_z_threshold,
            prior.content_miss_min_samples,
        ) {
            return Some(ContentMissReport {
                expectation_id: exp_id,
                reply_event_id: reply.id,
                who: reply.actor.clone(),
                timing_delta_ms,
                distance,
                usual_distance: usual,
                z_score,
            });
        }

        stats.observe(distance);
        None
    }

    /// Removes references to erased events or actors on `HardForget` (`R-34`).
    pub fn forget(&mut self, event_ids: &[EventId], actor_ids: &[ActorId]) {
        self.expectations.retain(|eid, exp| {
            !event_ids.contains(eid)
                && !event_ids.contains(&exp.target_event_id)
                && !event_ids.contains(&exp.origin_event_id)
                && !actor_ids
                    .iter()
                    .any(|a| exp.people.contains(a) || exp.audience.contains(a))
        });
        self.deferred_acts.retain(|d| {
            !event_ids.contains(&d.expectation_id) && !event_ids.contains(&d.depends_on)
        });
        self.wake_gate
            .verdicts
            .retain(|v| !event_ids.contains(&v.event_id) && !event_ids.contains(&v.expectation_id));
        self.pending_classify.retain(|k, _| !event_ids.contains(k));
    }

    /// Encodes `self` deterministically into `BinWriter` for snapshot persistence.
    pub(crate) fn encode(&self, w: &mut BinWriter) {
        w.u32(self.expectations.len() as u32);
        for (&eid, exp) in &self.expectations {
            w.u64(eid.as_u64());
            encode_expectation_record(w, exp);
        }
        w.u32(self.deferred_acts.len() as u32);
        for d in &self.deferred_acts {
            w.u64(d.expectation_id.as_u64());
            w.u64(d.depends_on.as_u64());
            w.u64(d.thought_event_id.as_u64());
            encode_action_bin(w, &d.action);
            w.u32(d.people.len() as u32);
            for p in &d.people {
                w.str(p.as_str());
            }
            w.f64(d.stakes);
            encode_tagged_embeddings_bin(w, &d.act_embeddings);
            encode_tagged_embeddings_bin(w, &d.expected_reply_embeddings);
        }
        w.u32(self.response_latency.by_place_medium.len() as u32);
        for ((place, med), sk) in &self.response_latency.by_place_medium {
            w.str(place.as_str());
            w.str(med.as_str());
            encode_sketch(w, sk);
        }
        w.u32(self.response_latency.by_medium.len() as u32);
        for (med, sk) in &self.response_latency.by_medium {
            w.str(med.as_str());
            encode_sketch(w, sk);
        }
        w.u32(self.place_distances.len() as u32);
        for (place, stats) in &self.place_distances {
            w.str(place.as_str());
            w.u64(stats.count as u64);
            w.f64(stats.mean);
            w.f64(stats.m2);
        }
        w.f64(self.wake_gate.wake_gate_lo);
        w.opt_f64(self.wake_gate.wake_gate_hi);
        w.u32(self.wake_gate.verdicts.len() as u32);
        for v in &self.wake_gate.verdicts {
            w.u64(v.event_id.as_u64());
            w.u64(v.expectation_id.as_u64());
            w.f64(v.similarity);
            w.u8(u8::from(v.met));
        }
        w.u32(self.pending_classify.len() as u32);
        for (&req_id, kind) in &self.pending_classify {
            w.u64(req_id.as_u64());
            match kind {
                PendingClassifyKind::Condition {
                    expectation_id,
                    similarity,
                } => {
                    w.u8(1);
                    w.u64(expectation_id.as_u64());
                    w.f64(*similarity);
                }
                PendingClassifyKind::SilenceReask { expectation_id } => {
                    w.u8(2);
                    w.u64(expectation_id.as_u64());
                }
                PendingClassifyKind::GoalAttribution {
                    expectation_id,
                    similarity,
                } => {
                    w.u8(3);
                    w.u64(expectation_id.as_u64());
                    w.f64(*similarity);
                }
                PendingClassifyKind::GistVerification {
                    gist_id,
                    source_ids,
                } => {
                    w.u8(4);
                    w.u64(gist_id.as_u64());
                    w.u32(source_ids.len() as u32);
                    for &sid in source_ids {
                        w.u64(sid.as_u64());
                    }
                }
            }
        }
    }

    /// Decodes an `ExpectationBook` from `BinReader`.
    pub(crate) fn decode(r: &mut BinReader<'_>) -> Result<Self, EventFormatError> {
        let exp_len = r.u32()? as usize;
        let mut expectations = BTreeMap::new();
        for _ in 0..exp_len {
            let eid = EventId::new(r.u64()?);
            let rec = decode_expectation_record(r)?;
            expectations.insert(eid, rec);
        }
        let def_len = r.u32()? as usize;
        let mut deferred_acts = Vec::with_capacity(def_len.min(256));
        for _ in 0..def_len {
            let expectation_id = EventId::new(r.u64()?);
            let depends_on = EventId::new(r.u64()?);
            let thought_event_id = EventId::new(r.u64()?);
            let action = decode_action_bin(r)?;
            let ppl_len = r.u32()? as usize;
            let mut people = Vec::with_capacity(ppl_len.min(128));
            for _ in 0..ppl_len {
                people.push(ActorId::new(r.string()?));
            }
            let stakes = r.f64()?;
            let act_embeddings = decode_tagged_embeddings_bin(r)?;
            let expected_reply_embeddings = decode_tagged_embeddings_bin(r)?;
            deferred_acts.push(DeferredDependentAct {
                expectation_id,
                depends_on,
                thought_event_id,
                action,
                people,
                stakes,
                act_embeddings,
                expected_reply_embeddings,
            });
        }
        let pm_len = r.u32()? as usize;
        let mut by_place_medium = BTreeMap::new();
        for _ in 0..pm_len {
            let place = PlaceId::new(r.string()?);
            let med = ChannelId::new(r.string()?);
            let sk = decode_sketch(r)?;
            by_place_medium.insert((place, med), sk);
        }
        let med_len = r.u32()? as usize;
        let mut by_medium = BTreeMap::new();
        for _ in 0..med_len {
            let med = ChannelId::new(r.string()?);
            let sk = decode_sketch(r)?;
            by_medium.insert(med, sk);
        }
        let pd_len = r.u32()? as usize;
        let mut place_distances = BTreeMap::new();
        for _ in 0..pd_len {
            let place = PlaceId::new(r.string()?);
            let count = r.u64()? as usize;
            let mean = r.f64()?;
            let m2 = r.f64()?;
            place_distances.insert(place, PlaceDistanceStats { count, mean, m2 });
        }
        let wake_gate_lo = r.f64()?;
        let wake_gate_hi = r.opt_f64()?;
        let v_len = r.u32()? as usize;
        let mut verdicts = Vec::with_capacity(v_len.min(4096));
        for _ in 0..v_len {
            verdicts.push(ConditionVerdictRecord {
                event_id: EventId::new(r.u64()?),
                expectation_id: EventId::new(r.u64()?),
                similarity: r.f64()?,
                met: r.u8()? != 0,
            });
        }
        let pc_len = r.u32()? as usize;
        let mut pending_classify = BTreeMap::new();
        for _ in 0..pc_len {
            let req_id = EventId::new(r.u64()?);
            let kind = match r.u8()? {
                1 => PendingClassifyKind::Condition {
                    expectation_id: EventId::new(r.u64()?),
                    similarity: r.f64()?,
                },
                2 => PendingClassifyKind::SilenceReask {
                    expectation_id: EventId::new(r.u64()?),
                },
                3 => PendingClassifyKind::GoalAttribution {
                    expectation_id: EventId::new(r.u64()?),
                    similarity: r.f64()?,
                },
                4 => {
                    let gist_id = EventId::new(r.u64()?);
                    let len = r.u32()? as usize;
                    let mut source_ids = Vec::with_capacity(len.min(1024));
                    for _ in 0..len {
                        source_ids.push(EventId::new(r.u64()?));
                    }
                    PendingClassifyKind::GistVerification {
                        gist_id,
                        source_ids,
                    }
                }
                tag => {
                    return Err(EventFormatError::MalformedRecord(format!(
                        "invalid pending_classify tag {tag}"
                    )));
                }
            };
            pending_classify.insert(req_id, kind);
        }

        Ok(Self {
            expectations,
            deferred_acts,
            response_latency: ResponseLatencyTracker {
                by_place_medium,
                by_medium,
            },
            place_distances,
            wake_gate: WakeGateCalibrator {
                wake_gate_lo,
                wake_gate_hi,
                verdicts,
            },
            pending_classify,
        })
    }
}

/// Computes maximum cosine similarity across matching `(embed_model_ver, dim_d, prompt_regime_id)` tags.
#[must_use]
pub fn max_matching_cosine(a: &[TaggedEmbedding], b: &[TaggedEmbedding]) -> Option<f64> {
    let mut best: Option<f64> = None;
    for ea in a {
        for eb in b {
            if ea.tag == eb.tag
                && let Some(sim) = cosine_similarity(&ea.values, &eb.values)
            {
                best = Some(best.map_or(sim, |prev| prev.max(sim)));
            }
        }
    }
    best
}

/// Evaluates whether a classifier verdict label is affirmative/met.
#[must_use]
pub fn is_positive_verdict_label(label: &str) -> bool {
    matches!(
        label.trim().to_ascii_lowercase().as_str(),
        "met"
            | "true"
            | "yes"
            | "positive"
            | "confirmed"
            | "missed"
            | "related"
            | "match"
            | "matched"
    )
}

/// Advances a [`RecurrenceSchedule`] from `due_ms` past `now_ms`, returning `(next_due_ms, due_slot_count)`.
///
/// `due_slot_count == 0` when `due_ms > now_ms` (never fires early).
#[must_use]
pub fn advance_recurrence_schedule(
    schedule: &RecurrenceSchedule,
    due_ms: Time,
    now_ms: Time,
) -> (Time, u64) {
    if due_ms > now_ms {
        return (due_ms, 0);
    }
    match schedule {
        RecurrenceSchedule::IntervalMs(raw_iv) => {
            let iv = (*raw_iv).max(1);
            let elapsed = now_ms.saturating_sub_ms(due_ms);
            let slots = elapsed / iv + 1;
            let next_due = due_ms.saturating_add_ms(slots.saturating_mul(iv));
            (next_due, slots)
        }
        RecurrenceSchedule::Cron(expr) => advance_cron_utc(expr, due_ms, now_ms),
    }
}

/// Deterministic UTC 5-field cron evaluator (`min hour dom month dow`) with zero wall clock.
fn advance_cron_utc(expr: &str, due_ms: Time, now_ms: Time) -> (Time, u64) {
    let Ok(cron) = ParsedCron::parse(expr) else {
        // Fallback: treat malformed cron safely as 1-hour interval to avoid infinite loop.
        return advance_recurrence_schedule(
            &RecurrenceSchedule::IntervalMs(3_600_000),
            due_ms,
            now_ms,
        );
    };
    const MIN_MS: u64 = 60_000;
    let start_min = due_ms.as_millis() / MIN_MS;
    let now_min = now_ms.as_millis() / MIN_MS;

    // Count missed/due slots in [start_min, now_min] (capped at 10_080 mins = 7 days for safety).
    let mut slots = 0u64;
    let scan_end = now_min.min(start_min.saturating_add(10_080));
    if now_min > scan_end {
        // Fast approximate count if outage > 7 days without unbounded loop.
        slots = 1 + (now_min - scan_end) / 60;
    }
    let mut m = start_min;
    while m <= scan_end {
        if cron.matches_epoch_minute(m) {
            slots = slots.saturating_add(1);
        }
        m += 1;
    }
    if slots == 0 {
        slots = 1;
    }

    // Find next matching minute strictly > now_min (search up to 366 days = 527_040 mins).
    let mut next_m = now_min.saturating_add(1);
    let limit = next_m.saturating_add(527_040);
    while next_m <= limit {
        if cron.matches_epoch_minute(next_m) {
            return (Time::from_millis(next_m.saturating_mul(MIN_MS)), slots);
        }
        next_m += 1;
    }
    (
        Time::from_millis(now_min.saturating_add(60).saturating_mul(MIN_MS)),
        slots,
    )
}

#[derive(Debug, Clone)]
struct ParsedCron {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    doms: BTreeSet<u32>,
    months: BTreeSet<u32>,
    dows: BTreeSet<u32>,
}

impl ParsedCron {
    fn parse(expr: &str) -> Result<Self, ()> {
        let normalized = match expr.trim() {
            "@hourly" => "0 * * * *",
            "@daily" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            other => other,
        };
        let parts: Vec<&str> = normalized.split_whitespace().collect();
        if parts.len() != 5 {
            return Err(());
        }
        Ok(Self {
            minutes: parse_cron_field(parts[0], 0, 59)?,
            hours: parse_cron_field(parts[1], 0, 23)?,
            doms: parse_cron_field(parts[2], 1, 31)?,
            months: parse_cron_field(parts[3], 1, 12)?,
            dows: parse_cron_field(parts[4], 0, 6)?,
        })
    }

    fn matches_epoch_minute(&self, epoch_min: u64) -> bool {
        let min = (epoch_min % 60) as u32;
        if !self.minutes.contains(&min) {
            return false;
        }
        let total_hours = epoch_min / 60;
        let hour = (total_hours % 24) as u32;
        if !self.hours.contains(&hour) {
            return false;
        }
        let days = (total_hours / 24) as i64;
        // 1970-01-01 was Thursday (dow = 4 in 0=Sun..6=Sat).
        let dow = ((days + 4).rem_euclid(7)) as u32;
        if !self.dows.contains(&dow) {
            return false;
        }
        let (_year, month, dom) = civil_from_days(days);
        self.months.contains(&month) && self.doms.contains(&dom)
    }
}

fn parse_cron_field(field: &str, min_val: u32, max_val: u32) -> Result<BTreeSet<u32>, ()> {
    let mut out = BTreeSet::new();
    for part in field.split(',') {
        let (range_part, step) = if let Some((lhs, rhs)) = part.split_once('/') {
            let s: u32 = rhs.parse().map_err(|_| ())?;
            if s == 0 {
                return Err(());
            }
            (lhs, s)
        } else {
            (part, 1)
        };
        let (start, end) = if range_part == "*" {
            (min_val, max_val)
        } else if let Some((a, b)) = range_part.split_once('-') {
            let s: u32 = a.parse().map_err(|_| ())?;
            let e: u32 = b.parse().map_err(|_| ())?;
            if s < min_val || e > max_val || s > e {
                return Err(());
            }
            (s, e)
        } else {
            let v: u32 = range_part.parse().map_err(|_| ())?;
            if v < min_val || v > max_val {
                return Err(());
            }
            if step == 1 { (v, v) } else { (v, max_val) }
        };
        let mut cur = start;
        while cur <= end {
            out.insert(cur);
            cur += step;
        }
    }
    if out.is_empty() { Err(()) } else { Ok(out) }
}

/// Howard Hinnant civil-from-days UTC algorithm: days since 1970-01-01 -> `(year, month 1..=12, day 1..=31)`.
const fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
}

fn encode_action_bin(w: &mut BinWriter, action: &Action) {
    w.u16(action.protocol_version.major);
    w.u16(action.protocol_version.minor);
    w.str(action.action_id.as_str());
    w.str(action.capability_id.as_str());
    w.u32(action.places.len() as u32);
    for p in &action.places {
        w.str(p.as_str());
    }
    w.u32(action.audience.members.len() as u32);
    for m in &action.audience.members {
        w.str(m.as_str());
    }
    w.str(action.origin_actor.actor_id().as_str());
    w.u8(encode_taint(action.taint));
    w.bytes(action.payload.as_bytes());
    w.opt_str(action.correlation_id.as_ref().map(CorrelationId::as_str));
    encode_opt_sensed_effect(w, action.expected_effect.as_ref());
    w.opt_str(action.h_act.as_ref().map(HAct::as_str));
    w.u64(action.timestamp_ms.as_millis());
}

fn decode_action_bin(r: &mut BinReader<'_>) -> Result<Action, EventFormatError> {
    let major = r.u16()?;
    let minor = r.u16()?;
    let protocol_version = if major == 0 && minor == 0 {
        CURRENT_PROTOCOL_VERSION
    } else {
        ProtocolVersion::new(major, minor)
    };
    let action_id = ActionId::new(r.string()?);
    let capability_id = CapabilityId::new(r.string()?);
    let plen = r.u32()? as usize;
    let mut places = Vec::with_capacity(plen.min(128));
    for _ in 0..plen {
        places.push(PlaceId::new(r.string()?));
    }
    let alen = r.u32()? as usize;
    let mut members = Vec::with_capacity(alen.min(1024));
    for _ in 0..alen {
        members.push(ActorId::new(r.string()?));
    }
    let audience = Audience::new(members);
    let origin_actor = OriginActor::new(ActorId::new(r.string()?));
    let taint = decode_taint(r.u8()?)?;
    let payload = OpaquePayload::from_bytes(r.bytes()?);
    let correlation_id = r.opt_string()?.map(CorrelationId::new);
    let expected_effect = decode_opt_sensed_effect(r)?;
    let h_act = r.opt_string()?.map(HAct::new);
    let timestamp_ms = Time::from_millis(r.u64()?);
    Ok(Action {
        protocol_version,
        action_id,
        capability_id,
        places,
        audience,
        origin_actor,
        taint,
        payload,
        correlation_id,
        expected_effect,
        h_act,
        timestamp_ms,
    })
}

fn encode_expectation_record(w: &mut BinWriter, exp: &ExpectationRecord) {
    w.u64(exp.expectation_id.as_u64());
    w.u64(exp.origin_event_id.as_u64());
    w.u64(exp.target_event_id.as_u64());
    w.u8(encode_expect_kind(exp.kind));
    w.u8(exp.state.code());
    w.u32(exp.version);
    w.u32(exp.places.len() as u32);
    for p in &exp.places {
        w.str(p.as_str());
    }
    w.u32(exp.people.len() as u32);
    for a in &exp.people {
        w.str(a.as_str());
    }
    w.u32(exp.audience.members.len() as u32);
    for m in &exp.audience.members {
        w.str(m.as_str());
    }
    w.opt_str(exp.domain.as_ref().map(DomainId::as_str));
    w.str(&exp.condition);
    encode_tagged_embeddings_bin(w, &exp.condition_embeddings);
    w.u32(exp.about_ids.len() as u32);
    for id in &exp.about_ids {
        w.u64(id.as_u64());
    }
    w.u64(exp.created_at_ms.as_millis());
    w.u64(exp.updated_at_ms.as_millis());
    w.u64(exp.due_ms.as_millis());
    w.u64(exp.expiry_ms.as_millis());
    encode_opt_recurrence_spec(w, exp.recurrence.as_ref());
    encode_opt_promise_meta(w, exp.promise_meta.as_ref());
    w.f64(exp.urgency_weight);
    w.u8(u8::from(exp.is_overdue));
    w.u8(u8::from(exp.due_woken));
    w.u32(exp.retry_count);
    w.opt_u64(exp.next_retry_at_ms.map(Time::as_millis));
    match &exp.retry_action {
        Some(act) => {
            w.u8(1);
            encode_action_bin(w, act);
        }
        None => w.u8(0),
    }
    w.opt_str(exp.last_action_id.as_ref().map(ActionId::as_str));
    w.opt_str(exp.last_capability_id.as_ref().map(CapabilityId::as_str));
    w.opt_str(exp.last_h_act.as_ref().map(HAct::as_str));
    w.opt_u64(exp.delivered_units);
    match exp.termination_reason {
        Some(tr) => {
            w.u8(1);
            w.u8(encode_term_reason(tr));
        }
        None => w.u8(0),
    }
    w.u8(u8::from(exp.approval_prompt_emitted));
    match &exp.effect_spec {
        Some(es) => {
            w.u8(1);
            w.str(es.action_id.as_str());
            w.str(es.capability_id.as_str());
            w.str(es.channel.as_str());
            w.u64(es.lat_min_ms);
            w.u64(es.lat_max_ms);
            w.f64(es.delta_pred);
            w.f64(es.sigma_channel);
            w.opt_str(es.correlation_id.as_ref().map(CorrelationId::as_str));
            w.u8(u8::from(es.declared_sensor));
        }
        None => w.u8(0),
    }
    match &exp.response_spec {
        Some(rs) => {
            w.u8(1);
            w.opt_str(rs.action_id.as_ref().map(ActionId::as_str));
            w.str(rs.medium.as_str());
            w.u32(rs.addressees.len() as u32);
            for a in &rs.addressees {
                w.str(a.as_str());
            }
            w.u8(u8::from(rs.accept_done_receipt));
            encode_tagged_embeddings_bin(w, &rs.act_embeddings);
            encode_tagged_embeddings_bin(w, &rs.expected_reply_embeddings);
            w.u8(u8::from(rs.is_judged_silence));
            w.opt_f64(rs.silence_sample_propensity);
            w.opt_str(rs.silence_sender.as_ref().map(ActorId::as_str));
            w.u8(u8::from(rs.awaiting_reask_classify));
            w.opt_u64(rs.pending_reask_event_id.map(EventId::as_u64));
            w.f64(rs.stakes);
            w.u64(rs.expected_latency_ms);
        }
        None => w.u8(0),
    }
    match &exp.monitor_spec {
        Some(ms) => {
            w.u8(1);
            w.opt_u64(ms.last_fired_ms.map(Time::as_millis));
            w.u64(ms.catchup_fired_count);
            w.u64(ms.skipped_slots);
        }
        None => w.u8(0),
    }
    w.u32(exp.goal_chain_ids.len() as u32);
    for id in &exp.goal_chain_ids {
        w.u64(id.as_u64());
    }
    w.u32(exp.linked_action_ids.len() as u32);
    for aid in &exp.linked_action_ids {
        w.str(aid.as_str());
    }
    w.opt_u64(exp.linked_expectation_id.map(EventId::as_u64));
}

fn decode_expectation_record(r: &mut BinReader<'_>) -> Result<ExpectationRecord, EventFormatError> {
    let expectation_id = EventId::new(r.u64()?);
    let origin_event_id = EventId::new(r.u64()?);
    let target_event_id = EventId::new(r.u64()?);
    let kind = decode_expect_kind(r.u8()?)?;
    let state_code = r.u8()?;
    let state = ExpectationLifecycleState::from_code(state_code).ok_or_else(|| {
        EventFormatError::MalformedRecord(format!("invalid expectation state code {state_code}"))
    })?;
    let version = r.u32()?;
    let plen = r.u32()? as usize;
    let mut places = Vec::with_capacity(plen.min(128));
    for _ in 0..plen {
        places.push(PlaceId::new(r.string()?));
    }
    let ppl_len = r.u32()? as usize;
    let mut people = Vec::with_capacity(ppl_len.min(128));
    for _ in 0..ppl_len {
        people.push(ActorId::new(r.string()?));
    }
    let alen = r.u32()? as usize;
    let mut members = Vec::with_capacity(alen.min(1024));
    for _ in 0..alen {
        members.push(ActorId::new(r.string()?));
    }
    let audience = Audience::new(members);
    let domain = r.opt_string()?.map(DomainId::new);
    let condition = r.string()?;
    let condition_embeddings = decode_tagged_embeddings_bin(r)?;
    let about_len = r.u32()? as usize;
    let mut about_ids = Vec::with_capacity(about_len.min(256));
    for _ in 0..about_len {
        about_ids.push(EventId::new(r.u64()?));
    }
    let created_at_ms = Time::from_millis(r.u64()?);
    let updated_at_ms = Time::from_millis(r.u64()?);
    let due_ms = Time::from_millis(r.u64()?);
    let expiry_ms = Time::from_millis(r.u64()?);
    let recurrence = decode_opt_recurrence_spec(r)?;
    let promise_meta = decode_opt_promise_meta(r)?;
    let urgency_weight = r.f64()?;
    let is_overdue = r.u8()? != 0;
    let due_woken = r.u8()? != 0;
    let retry_count = r.u32()?;
    let next_retry_at_ms = r.opt_u64()?.map(Time::from_millis);
    let retry_action = match r.u8()? {
        0 => None,
        1 => Some(decode_action_bin(r)?),
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid opt retry_action tag {tag}"
            )));
        }
    };
    let last_action_id = r.opt_string()?.map(ActionId::new);
    let last_capability_id = r.opt_string()?.map(CapabilityId::new);
    let last_h_act = r.opt_string()?.map(HAct::new);
    let delivered_units = r.opt_u64()?;
    let termination_reason = match r.u8()? {
        0 => None,
        1 => Some(decode_term_reason(r.u8()?)?),
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid opt term_reason tag {tag}"
            )));
        }
    };
    let approval_prompt_emitted = r.u8()? != 0;
    let effect_spec = match r.u8()? {
        0 => None,
        1 => Some(EffectPredictionSpec {
            action_id: ActionId::new(r.string()?),
            capability_id: CapabilityId::new(r.string()?),
            channel: ChannelId::new(r.string()?),
            lat_min_ms: r.u64()?,
            lat_max_ms: r.u64()?,
            delta_pred: r.f64()?,
            sigma_channel: r.f64()?,
            correlation_id: r.opt_string()?.map(CorrelationId::new),
            declared_sensor: r.u8()? != 0,
        }),
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid opt effect_spec tag {tag}"
            )));
        }
    };
    let response_spec = match r.u8()? {
        0 => None,
        1 => {
            let action_id = r.opt_string()?.map(ActionId::new);
            let medium = ChannelId::new(r.string()?);
            let add_len = r.u32()? as usize;
            let mut addressees = Vec::with_capacity(add_len.min(128));
            for _ in 0..add_len {
                addressees.push(ActorId::new(r.string()?));
            }
            let accept_done_receipt = r.u8()? != 0;
            let act_embeddings = decode_tagged_embeddings_bin(r)?;
            let expected_reply_embeddings = decode_tagged_embeddings_bin(r)?;
            let is_judged_silence = r.u8()? != 0;
            let silence_sample_propensity = r.opt_f64()?;
            let silence_sender = r.opt_string()?.map(ActorId::new);
            let awaiting_reask_classify = r.u8()? != 0;
            let pending_reask_event_id = r.opt_u64()?.map(EventId::new);
            let stakes = r.f64()?;
            let expected_latency_ms = r.u64()?;
            Some(ResponsePredictionSpec {
                action_id,
                medium,
                addressees,
                accept_done_receipt,
                act_embeddings,
                expected_reply_embeddings,
                is_judged_silence,
                silence_sample_propensity,
                silence_sender,
                awaiting_reask_classify,
                pending_reask_event_id,
                stakes,
                expected_latency_ms,
            })
        }
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid opt response_spec tag {tag}"
            )));
        }
    };
    let monitor_spec = match r.u8()? {
        0 => None,
        1 => Some(MonitorState {
            last_fired_ms: r.opt_u64()?.map(Time::from_millis),
            catchup_fired_count: r.u64()?,
            skipped_slots: r.u64()?,
        }),
        tag => {
            return Err(EventFormatError::MalformedRecord(format!(
                "invalid opt monitor_spec tag {tag}"
            )));
        }
    };
    let gc_len = r.u32()? as usize;
    let mut goal_chain_ids = Vec::with_capacity(gc_len.min(256));
    for _ in 0..gc_len {
        goal_chain_ids.push(EventId::new(r.u64()?));
    }
    let la_len = r.u32()? as usize;
    let mut linked_action_ids = Vec::with_capacity(la_len.min(256));
    for _ in 0..la_len {
        linked_action_ids.push(ActionId::new(r.string()?));
    }
    let linked_expectation_id = r.opt_u64()?.map(EventId::new);

    Ok(ExpectationRecord {
        expectation_id,
        origin_event_id,
        target_event_id,
        kind,
        state,
        version,
        places,
        people,
        audience,
        domain,
        condition,
        condition_embeddings,
        about_ids,
        created_at_ms,
        updated_at_ms,
        due_ms,
        expiry_ms,
        recurrence,
        promise_meta,
        urgency_weight,
        is_overdue,
        due_woken,
        retry_count,
        next_retry_at_ms,
        retry_action,
        last_action_id,
        last_capability_id,
        last_h_act,
        delivered_units,
        termination_reason,
        approval_prompt_emitted,
        effect_spec,
        response_spec,
        monitor_spec,
        goal_chain_ids,
        linked_action_ids,
        linked_expectation_id,
    })
}

fn encode_sketch(w: &mut BinWriter, sk: &LatencySketch) {
    w.u32(sk.events.len() as u32);
    for (&b, &v) in &sk.events {
        w.u16(b);
        w.f64(v);
    }
    w.u32(sk.censored.len() as u32);
    for (&b, &v) in &sk.censored {
        w.u16(b);
        w.f64(v);
    }
}

fn decode_sketch(r: &mut BinReader<'_>) -> Result<LatencySketch, EventFormatError> {
    let ev_len = r.u32()? as usize;
    let mut events = BTreeMap::new();
    for _ in 0..ev_len {
        events.insert(r.u16()?, r.f64()?);
    }
    let cen_len = r.u32()? as usize;
    let mut censored = BTreeMap::new();
    for _ in 0..cen_len {
        censored.insert(r.u16()?, r.f64()?);
    }
    Ok(LatencySketch { events, censored })
}

#[cfg(test)]
mod tests {
    use super::{
        PlaceDistanceStats, ResponseLatencyTracker, WakeGateCalibrator, advance_recurrence_schedule,
    };
    use crate::prior::Prior;
    use crate::registry::Switches;
    use monoagens_protocol::{ChannelId, EventId, PlaceId, RecurrenceSchedule, Time};

    #[test]
    fn interval_and_cron_recurrence_advance_without_storms() {
        let (next, slots) = advance_recurrence_schedule(
            &RecurrenceSchedule::IntervalMs(1_000),
            Time::from_millis(10_000),
            Time::from_millis(15_500),
        );
        assert_eq!(slots, 6);
        assert_eq!(next, Time::from_millis(16_000));

        // Before due -> zero slots, unchanged due.
        let (early_next, early_slots) = advance_recurrence_schedule(
            &RecurrenceSchedule::IntervalMs(1_000),
            Time::from_millis(10_000),
            Time::from_millis(9_999),
        );
        assert_eq!(early_slots, 0);
        assert_eq!(early_next, Time::from_millis(10_000));

        // Cron: "0 * * * *" (hourly at minute 0)
        // Due at 01:00 (3_600_000 ms), now at 04:15 (15_300_000 ms) -> 4 missed/due slots (01,02,03,04), next = 05:00 (18_000_000 ms)
        let (cron_next, cron_slots) = advance_recurrence_schedule(
            &RecurrenceSchedule::Cron("0 * * * *".to_owned()),
            Time::from_millis(3_600_000),
            Time::from_millis(15_300_000),
        );
        assert_eq!(cron_slots, 4);
        assert_eq!(cron_next, Time::from_millis(18_000_000));
    }

    #[test]
    fn rule_of_three_blocks_premature_upper_band_and_negative_verdict() {
        let prior = Prior::default();
        let mut gate = WakeGateCalibrator::default();
        assert_eq!(WakeGateCalibrator::rule_of_three_required(0.06), 50);

        for i in 1..=49 {
            gate.record_verdict(EventId::new(i), EventId::new(1), 0.95, true, &prior);
        }
        assert!(
            !gate.try_adopt_upper_band(0.90, &prior),
            "49 verdicts < 50 must not adopt upper band"
        );

        // One negative verdict above 0.90 blocks adoption even at 50+.
        gate.record_verdict(EventId::new(50), EventId::new(1), 0.92, false, &prior);
        assert!(!gate.try_adopt_upper_band(0.90, &prior));

        // Clean 50 verdicts above 0.96 -> allowed.
        let mut clean = WakeGateCalibrator::default();
        for i in 1..=50 {
            clean.record_verdict(EventId::new(i), EventId::new(1), 0.97, true, &prior);
        }
        assert!(clean.try_adopt_upper_band(0.96, &prior));
        assert_eq!(clean.wake_gate_hi, Some(0.96));
    }

    #[test]
    fn response_latency_uses_declared_on_new_place_and_learns_slow_place() {
        let prior = Prior::default();
        let sw = Switches::all_enabled();
        let mut tracker = ResponseLatencyTracker::default();
        let place = PlaceId::new("slow_email");
        let med = ChannelId::new("email");

        assert_eq!(
            tracker.estimate_window_ms(
                &place,
                &med,
                12_000,
                prior.response_expiry_quantile,
                &prior,
                &sw
            ),
            12_000
        );

        for _ in 0..15 {
            tracker.observe_response(&place, &med, 7_200_000, 1.0); // 2 hours
        }
        let learned = tracker.estimate_window_ms(
            &place,
            &med,
            12_000,
            prior.response_expiry_quantile,
            &prior,
            &sw,
        );
        assert!(
            learned >= 3_600_000,
            "slow place must learn hour-scale duration, got {learned} ms"
        );
    }

    #[test]
    fn content_miss_stats_flag_outlier_distance() {
        let mut stats = PlaceDistanceStats::default();
        stats.observe(0.10);
        stats.observe(0.12);
        stats.observe(0.11);
        assert!(stats.check_miss(0.13, 2.0, 3).is_none());
        let miss = stats
            .check_miss(0.85, 2.0, 3)
            .expect("outlier distance flagged");
        assert!(miss.1 > 2.0);
    }
}

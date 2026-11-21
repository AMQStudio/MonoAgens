//! `ContextPlan` assembly, Slot-0 `IdentityAnchor`, Spotlighting trust boundaries,
//! 4-pillar `self_card`, progressive `Tool RAG`, report control `P_rc`, and `CoreSelfSnapshot`.
//!
//! Core stays sans-IO and content-blind for identity and percept payloads (`ARCHITECTURE.md`).
//! Prompt text rendering (`reason/src/render.rs`) is a separate crate concern consuming
//! [`ContextPlan`] and the exhaustive [`self_guide_template_table`].

use std::collections::{BTreeMap, BTreeSet};

use monoagens_protocol::{
    ActorId, AttachmentHandle, Audience, CapabilityDescriptor, CapabilityId, CapabilitySchemaHash,
    DomainId, EventId, ExpectKind, OpaquePayload, OriginActor, OwnerSigned, PlaceId,
    ProtocolVersion, TaggedEmbedding, Taint, Time, VisibleScope, sort_capabilities,
};
use serde::{Deserialize, Serialize};

use crate::blake3;
use crate::config::CoreConfig;
use crate::event::{EventRecord, LogSeq};
use crate::expectations::max_matching_cosine;
use crate::recall::{MemoryBasis, RecalledItem, RecollectionConstraint};

/// Opaque owner-signed identity anchor pinned in Slot 0 of [`crate::ports::ContextPlan`] (`R-48`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityAnchor {
    pub anchor_id: String,
    pub identity_blob_hash: [u8; 32],
    pub owner_sig_hash: [u8; 32],
    pub payload: OpaquePayload,
}

impl IdentityAnchor {
    /// Constructs a content-blind `IdentityAnchor` from an [`OwnerSigned<OpaquePayload>`].
    #[must_use]
    pub fn from_owner_signed(signed: &OwnerSigned<OpaquePayload>) -> OwnerSigned<Self> {
        let blob_hash = blake3::hash(signed.payload.as_bytes());
        let sig_hash = blake3::hash(signed.signature.as_bytes());
        let hex = blake3::to_hex(&blob_hash);
        OwnerSigned {
            payload: Self {
                anchor_id: format!("anchor:{}", &hex[..16]),
                identity_blob_hash: blob_hash,
                owner_sig_hash: sig_hash,
                payload: signed.payload.clone(),
            },
            signer: signed.signer.clone(),
            signed_at_ms: signed.signed_at_ms,
            signature: signed.signature.clone(),
        }
    }
}

/// Severity tier of an owner-signed [`PrincipleDescriptor`] (`R-48`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipleSeverity {
    HardLine,
    StrongNorm,
    Advisory,
}

/// Content-blind principle descriptor enforced by `[ANCHOR_CHECK]` and `PrincipleAudit` (`R-48`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrincipleDescriptor {
    pub id: String,
    pub severity: PrincipleSeverity,
    pub blocked_capabilities: Vec<CapabilityId>,
    pub forbid_identity_change: bool,
    pub forbid_untrusted_taint: bool,
    pub created_seq: LogSeq,
    pub owner_sig_hash: [u8; 32],
}

/// Deliberative audit verdict recorded alongside candidate actions (`R-48`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipleAuditVerdict {
    Aligned,
    TensionResolved,
    HardConflict,
}

/// Spotlighting trust delimiter class carried on each context segment (`R-43`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustSegmentKind {
    System,
    Anchor,
    Trusted,
    UntrustedQuoted,
}

/// Classifies a log item's Spotlighting trust segment strictly by provenance (`R-43`).
///
/// Any content originating outside the soul (`actor != soul_id` or `origin_actor != soul_id` or
/// `taint != Trusted`) is classified as [`TrustSegmentKind::UntrustedQuoted`].
#[must_use]
pub fn classify_trust_segment(
    actor: &ActorId,
    origin_actor: &OriginActor,
    soul_id: &ActorId,
    taint: Taint,
) -> TrustSegmentKind {
    if actor == soul_id && origin_actor.actor_id() == soul_id && taint == Taint::Trusted {
        TrustSegmentKind::Trusted
    } else {
        TrustSegmentKind::UntrustedQuoted
    }
}

/// Ordered slot classification within [`ContextSections`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextSlotKind {
    Slot0Anchor,
    StaticCoreCaps,
    SelfGuide,
    ValveSelfGuide,
    DynamicCaps,
    Procedures,
    SelfCard,
    SourceCards,
    Commitments,
    WorkingSet,
    Recalled,
    PlaceTail,
    Task,
    Temporal,
    Trigger,
}

/// Provenance-bearing, Spotlighted segment inside a [`crate::ports::ContextPlan`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextSegment {
    pub slot: ContextSlotKind,
    pub segment_kind: TrustSegmentKind,
    pub event_id: Option<EventId>,
    pub origin_actor: OriginActor,
    pub taint: Taint,
    pub timestamp_ms: Option<Time>,
    pub basis: Option<MemoryBasis>,
    pub recollection: Option<RecollectionConstraint>,
    pub is_gist: bool,
    pub superseded_by: Option<EventId>,
    pub attachments: Vec<AttachmentHandle>,
    pub estimated_bytes: usize,
    pub is_reference_only: bool,
    pub label: String,
}

/// Record of an item evicted from [`ContextSections`] due to byte/token budget constraints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedContextItem {
    pub slot: ContextSlotKind,
    pub event_id: Option<EventId>,
    pub estimated_bytes: usize,
    pub reason: String,
}

/// Four-pillar read-only `self_card` derived strictly from audited log events (`R-35`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct CoreSelfCard {
    /// Pillar 1: capability/delivery reliability Beta `(alpha_c, beta_c)`, latency, and degraded list.
    pub reliability_alpha: f64,
    pub reliability_beta: f64,
    pub mean_delivery_latency_ms: u64,
    pub degraded_capabilities: Vec<CapabilityId>,
    /// Pillar 2: `[0, 20]` confidence calibration sample count, Brier, ECE, AUROC_2, meta-d', and bias.
    pub calibration_samples: u64,
    pub calibration_brier: f64,
    pub calibration_ece: f64,
    pub calibration_auroc_2: f64,
    pub calibration_meta_d_prime: f64,
    pub calibration_bias: f64,
    /// Pillar 3: operational budget headroom `(1 - used / limit)` in `[0, 1]`.
    pub budget_headroom_ratio: f64,
    /// Pillar 4: claim/principle consistency Beta and recorded contradiction flags.
    pub principle_alpha: f64,
    pub principle_beta: f64,
    pub contradiction_flags: u64,
}

/// Compact source ledger card for participants present at wake time (`R-25`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceLedgerCard {
    pub actor_id: ActorId,
    pub reliability_alpha: f64,
    pub reliability_beta: f64,
    pub closeness: f64,
    pub trusted_origin: bool,
    pub witnessed_count: usize,
    pub open_requests_count: usize,
    pub ungrounded_told_count: usize,
    pub false_belief_count: usize,
}

/// Active commitment entry shown in the commitments slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitmentSlotItem {
    pub expectation_id: EventId,
    pub kind: ExpectKind,
    pub due_ms: Time,
    pub expiry_ms: Time,
    pub places: Vec<PlaceId>,
    pub people: Vec<ActorId>,
    pub is_fired_or_overdue: bool,
    #[serde(default)]
    pub condition: String,
    pub estimated_bytes: usize,
}

/// Capability catalog item held in Core for progressive `Tool RAG` disclosure (`R-38`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityCatalogEntry {
    pub descriptor: CapabilityDescriptor,
    pub is_static_core: bool,
    pub owner_signed: bool,
    pub active: bool,
    pub audience: Audience,
    pub embedding: Option<TaggedEmbedding>,
}

/// Consolidated procedure item held in Core for progressive `Tool RAG` disclosure (`R-38`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcedureEntry {
    pub procedure_id: EventId,
    pub name: String,
    pub steps: Vec<String>,
    pub required_cap_ids: Vec<CapabilityId>,
    pub capability_schema_hash: CapabilitySchemaHash,
    pub audience: Audience,
    pub domain: Option<DomainId>,
    pub active: bool,
    pub retired: bool,
    pub owner_signed: bool,
    pub embedding: Option<TaggedEmbedding>,
}

/// Selection budgets for progressive `Tool RAG` disclosure (`R-38`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSelectionBudget {
    pub max_static_core: usize,
    pub k_cap: usize,
    pub k_proc: usize,
}

impl ToolSelectionBudget {
    /// Builds selection budget from `config`.
    #[must_use]
    pub fn from_config(config: &CoreConfig, available_caps: usize) -> Self {
        Self {
            max_static_core: 4,
            k_cap: available_caps.clamp(1, config.max_recalled_memories.clamp(4, 8)),
            k_proc: 4,
        }
    }

    /// Exact neutral ablation budget (`k_cap = |C_avail|, k_proc = 0`).
    #[must_use]
    pub const fn neutral(available_capabilities: usize) -> Self {
        Self {
            max_static_core: 4,
            k_cap: available_capabilities,
            k_proc: 0,
        }
    }
}

/// Output of deterministic progressive `Tool RAG` selection for `ContextPlan` (`R-38`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SelectedToolSet {
    pub static_core: Vec<CapabilityDescriptor>,
    pub dynamic_caps: Vec<CapabilityDescriptor>,
    pub procedures: Vec<ProcedureEntry>,
    pub excluded_stale_procedures: usize,
    pub excluded_hidden_items: usize,
}

/// Selects up to `max_static_core` (`<= 4`) static `OwnerSigned` core tools, top-`k_cap` dynamic
/// capabilities, and top-`k_proc` schema-valid procedures filtered strictly by `VisibleScope ∩ ActiveCaps`.
#[must_use]
pub fn select_tools_for_context(
    query_terms: &[String],
    query_embeddings: &[TaggedEmbedding],
    capabilities: &[CapabilityCatalogEntry],
    procedures: &[ProcedureEntry],
    visible_scope: &VisibleScope,
    now: Time,
    budget: ToolSelectionBudget,
) -> SelectedToolSet {
    let mut active_cap_hashes: BTreeMap<&CapabilityId, &CapabilitySchemaHash> = BTreeMap::new();
    let mut static_candidates: Vec<CapabilityDescriptor> = Vec::new();
    let mut dynamic_candidates: Vec<&CapabilityCatalogEntry> = Vec::new();
    let mut excluded_hidden_items = 0usize;

    for cap in capabilities {
        if !cap.active {
            continue;
        }
        if !cap.audience.is_empty() && !visible_scope.allows(&cap.audience, None, now) {
            excluded_hidden_items += 1;
            continue;
        }
        active_cap_hashes.insert(&cap.descriptor.id, &cap.descriptor.capability_schema_hash);
        if cap.is_static_core && cap.owner_signed {
            static_candidates.push(cap.descriptor.clone());
        } else {
            dynamic_candidates.push(cap);
        }
    }

    sort_capabilities(&mut static_candidates);
    static_candidates.truncate(budget.max_static_core.min(4));

    let norm_terms = normalize_tokens(query_terms);
    let mut scored_caps: Vec<(f64, &CapabilityCatalogEntry)> = dynamic_candidates
        .iter()
        .map(|cap| {
            let text_score = bm25_like_score(
                &norm_terms,
                cap.descriptor.id.as_str(),
                &cap.descriptor.description,
            );
            let vec_score = cap
                .embedding
                .as_ref()
                .and_then(|emb| max_matching_cosine(query_embeddings, core::slice::from_ref(emb)))
                .map_or(0.0, |c| c.max(0.0));
            let score = 0.5 * text_score + 0.5 * vec_score;
            (score, *cap)
        })
        .collect();
    scored_caps.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| a.1.descriptor.id.cmp(&b.1.descriptor.id))
            .then_with(|| {
                a.1.descriptor
                    .capability_schema_hash
                    .cmp(&b.1.descriptor.capability_schema_hash)
            })
    });

    let mut dynamic_caps: Vec<CapabilityDescriptor> = scored_caps
        .into_iter()
        .take(budget.k_cap)
        .map(|(_, c)| c.descriptor.clone())
        .collect();
    // Maintain deterministic capability metadata canonicalization on the selected set?
    // Keep ranked order or sorted? Capability descriptors have sorted places.
    for d in &mut dynamic_caps {
        d.places.sort();
        d.places.dedup();
    }

    let mut excluded_stale_procedures = 0usize;
    let mut valid_procs: Vec<(f64, &ProcedureEntry)> = Vec::new();
    for proc_item in procedures {
        if !proc_item.active || proc_item.retired {
            continue;
        }
        if !visible_scope.allows(&proc_item.audience, Some(proc_item.procedure_id), now) {
            excluded_hidden_items += 1;
            continue;
        }
        let schema_ok = !proc_item.required_cap_ids.is_empty()
            && proc_item.required_cap_ids.iter().all(|req_id| {
                active_cap_hashes
                    .get(req_id)
                    .is_some_and(|h| **h == proc_item.capability_schema_hash)
            });
        if !schema_ok {
            excluded_stale_procedures += 1;
            continue;
        }
        let step_blob = proc_item.steps.join(" ");
        let text_score = bm25_like_score(&norm_terms, &proc_item.name, &step_blob);
        let vec_score = proc_item
            .embedding
            .as_ref()
            .and_then(|emb| max_matching_cosine(query_embeddings, core::slice::from_ref(emb)))
            .map_or(0.0, |c| c.max(0.0));
        valid_procs.push((0.5 * text_score + 0.5 * vec_score, proc_item));
    }
    valid_procs.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| a.1.procedure_id.cmp(&b.1.procedure_id))
    });
    let selected_procs: Vec<ProcedureEntry> = valid_procs
        .into_iter()
        .take(budget.k_proc)
        .map(|(_, p)| p.clone())
        .collect();

    SelectedToolSet {
        static_core: static_candidates,
        dynamic_caps,
        procedures: selected_procs,
        excluded_stale_procedures,
        excluded_hidden_items,
    }
}

fn normalize_tokens(terms: &[String]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for term in terms {
        for tok in term
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            out.insert(tok.to_ascii_lowercase());
        }
    }
    out
}

fn bm25_like_score(query_tokens: &BTreeSet<String>, id: &str, text: &str) -> f64 {
    if query_tokens.is_empty() {
        return 0.0;
    }
    let mut doc_tokens: BTreeMap<String, usize> = BTreeMap::new();
    let mut doc_len = 0usize;
    for chunk in [id, text] {
        for tok in chunk
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            *doc_tokens.entry(tok.to_ascii_lowercase()).or_insert(0) += 1;
            doc_len += 1;
        }
    }
    if doc_len == 0 {
        return 0.0;
    }
    let k1 = 1.2_f64;
    let b = 0.75_f64;
    let avgdl = 16.0_f64;
    let mut sum = 0.0_f64;
    for q in query_tokens {
        if let Some(&tf) = doc_tokens.get(q) {
            let tf_f = tf as f64;
            let norm = tf_f * (k1 + 1.0) / (tf_f + k1 * (1.0 - b + b * (doc_len as f64 / avgdl)));
            sum += norm;
        }
    }
    sum / (query_tokens.len() as f64)
}

/// Typed summary of active `VisibleScope` inside [`CoreSelfSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct VisibleScopeSummary {
    pub audience_count: usize,
    pub target_place: Option<PlaceId>,
    pub active_declass_tokens: usize,
}

/// Typed `auto_recall` trigger state inside [`CoreSelfSnapshot`].
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutoRecallTriggerState {
    pub active: bool,
    pub attempt: u8,
    pub max_attempts: u8,
    pub coarser_grain_allowed: bool,
    pub last_reason: Option<String>,
}

/// Typed read-only snapshot emitted by Core when `config.self_guide_enabled` is `true` (`R-48`).
///
/// Contains strictly typed flags, controller status, scope summaries, trigger states, and protocol
/// version — zero prose and zero tunable formula constants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreSelfSnapshot {
    pub protocol_version: ProtocolVersion,
    pub neural_active: bool,
    pub model_blake3_prefix: Option<String>,
    pub online_adaptation: bool,
    pub visible_scope_summary: VisibleScopeSummary,
    pub auto_recall_state: AutoRecallTriggerState,
    pub recent_receipt_cues: Vec<String>,
}

impl CoreSelfSnapshot {
    /// Builds a content-blind [`CoreSelfSnapshot`] from runtime config, neural state, `scope`, and `auto_recall`.
    #[must_use]
    pub fn build(
        protocol_version: ProtocolVersion,
        neural_blake3: Option<&[u8; 32]>,
        online_adaptation: bool,
        scope: &VisibleScope,
        auto_recall_state: AutoRecallTriggerState,
        recent_receipt_cues: Vec<String>,
    ) -> Self {
        let model_blake3_prefix = neural_blake3.map(|d| {
            let hex = blake3::to_hex(d);
            hex[..12.min(hex.len())].to_owned()
        });
        Self {
            protocol_version,
            neural_active: neural_blake3.is_some(),
            model_blake3_prefix,
            online_adaptation,
            visible_scope_summary: VisibleScopeSummary {
                audience_count: scope.audience.len(),
                target_place: scope.target_place.clone(),
                active_declass_tokens: scope.declass_tokens.len(),
            },
            auto_recall_state,
            recent_receipt_cues,
        }
    }
}

/// Opaque snapshot attached by `Valve` through `Doors::context` for `[SELF_GUIDE]` rendering (`7.6`).
///
/// Core never inspects or generates this struct; it preserves it as opaque data in a reserved
/// [`crate::ports::ContextPlan`] slot.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ValveSelfSnapshot {
    pub valve_mode: String,
    pub quiet_hours_active: bool,
    pub pending_approval_count: usize,
    pub daily_cap_remaining: Option<u32>,
    pub taint_gating_active: bool,
    pub opaque_tags: Vec<String>,
}

/// Typed extension-point row for V2 Self-Guide sections rendered by `crates/reason/src/render.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelfGuideTemplateEntry {
    pub stable_id: &'static str,
    pub template_key: &'static str,
    pub summary_tag: &'static str,
}

/// V2 deterministic self-guide sections rendered without tunable constants.
#[must_use]
pub fn self_guide_template_table() -> [SelfGuideTemplateEntry; 4] {
    [
        SelfGuideTemplateEntry {
            stable_id: "neural_controller",
            template_key: "guide.v2.neural_controller",
            summary_tag: "neural_cognitive_controller",
        },
        SelfGuideTemplateEntry {
            stable_id: "visible_scope",
            template_key: "guide.v2.visible_scope",
            summary_tag: "strict_visible_scope",
        },
        SelfGuideTemplateEntry {
            stable_id: "commitment_and_recall",
            template_key: "guide.v2.commitment_and_recall",
            summary_tag: "deterministic_commitments_and_recall",
        },
        SelfGuideTemplateEntry {
            stable_id: "agency_and_grounding",
            template_key: "guide.v2.agency_and_grounding",
            summary_tag: "agency_and_expectation_grounding",
        },
    ]
}

/// The question a `classify` or `consolidate` call answers, with the material it judges.
///
/// Core carries only structured data; `reason` renders each variant from a fixed template, so the
/// wording and the accepted labels never come from Core prose.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "task", rename_all = "snake_case")]
pub enum ModelTask {
    /// Does the trigger show that the condition (by its own wording) is now met?
    ConditionMet { condition: String },
    /// Does the trigger reach or advance the goal described by `condition`?
    GoalProgress { condition: String },
    /// Is the trigger, from the same sender, raising again the matter of `earlier`, the message
    /// the soul stayed silent on?
    SilenceReask { earlier: ContextSegment },
    /// Is the summary in the trigger faithful to `sources`?
    GistFaithful { sources: Vec<ContextSegment> },
    /// Summarize the episode `events` (in order) into one gist.
    ConsolidateEpisode { events: Vec<ContextSegment> },
}

impl ModelTask {
    /// Segments whose payloads must be resolved for rendering.
    #[must_use]
    pub fn segments(&self) -> &[ContextSegment] {
        match self {
            Self::ConditionMet { .. } | Self::GoalProgress { .. } => &[],
            Self::SilenceReask { earlier } => std::slice::from_ref(earlier),
            Self::GistFaithful { sources } => sources,
            Self::ConsolidateEpisode { events } => events,
        }
    }
}

/// Ordered, budgeted sections of [`crate::ports::ContextPlan`].
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ContextSections {
    // --- Stable cacheable prefix (Slot 0 .. SelfGuide) ---
    pub slot_0_anchor: Option<OwnerSigned<IdentityAnchor>>,
    pub slot_0_principles: Vec<OwnerSigned<PrincipleDescriptor>>,
    pub static_core_caps: Vec<CapabilityDescriptor>,
    pub self_guide_snapshot: Option<CoreSelfSnapshot>,

    // --- Dynamic disclosure & cards ---
    pub dynamic_caps: Vec<CapabilityDescriptor>,
    pub procedures: Vec<ProcedureEntry>,
    pub excluded_stale_procedures: usize,
    pub self_card: CoreSelfCard,
    pub source_cards: Vec<SourceLedgerCard>,

    // --- Commitments, working set, recall, place tail, temporal, trigger ---
    pub commitments: Vec<CommitmentSlotItem>,
    pub working_set: Vec<ContextSegment>,
    pub working_set_cues: Vec<String>,
    pub recalled: Vec<ContextSegment>,
    pub place_tail: Vec<ContextSegment>,
    pub now_ms: Time,
    pub knowledge_cutoff: Option<String>,
    pub anchor_check_due: bool,
    pub trigger: Option<ContextSegment>,
    /// What a non-reasoning call (`classify`, `consolidate`) is asked to judge or produce, with
    /// the material it needs. `None` for ordinary reasoning wakes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<ModelTask>,

    // --- Budget accounting ---
    pub budget_bytes: usize,
    pub total_estimated_bytes: usize,
}

/// Computes `P_rc = clamp(cost / (value + cost), certainty_bar, 0.99)` with `0/0 -> certainty_bar` (`A-01`).
///
/// Returns `0.0` when `!report_control_enabled` (Brain Map Row 17 reachable neutral value).
#[must_use]
pub fn compute_report_threshold(
    value: Option<f32>,
    cost: Option<f32>,
    certainty_bar: f64,
    report_control_enabled: bool,
) -> f64 {
    if !report_control_enabled {
        return 0.0;
    }
    let bar = if certainty_bar.is_finite() {
        certainty_bar.clamp(0.0, 0.99)
    } else {
        0.5
    };
    let v = value
        .map(f64::from)
        .filter(|x| x.is_finite() && *x >= 0.0)
        .unwrap_or(0.0);
    let c = cost
        .map(f64::from)
        .filter(|x| x.is_finite() && *x >= 0.0)
        .unwrap_or(0.0);
    let denom = v + c;
    if denom <= 0.0 || !denom.is_finite() {
        bar
    } else {
        (c / denom).clamp(bar, 0.99)
    }
}

/// Maps integer `[0, 20]` confidence to `c_raw = k / 20` and applies domain calibration offset (`A-10`).
#[must_use]
pub fn calibrate_confidence_0_20(confidence_0_20: u8, domain_offset: f64) -> f64 {
    let raw = f64::from(confidence_0_20.min(20)) / 20.0;
    let offset = if domain_offset.is_finite() {
        domain_offset
    } else {
        0.0
    };
    (raw + offset).clamp(0.0, 1.0)
}

/// Computes a deterministic BLAKE3 digest over the cacheable static prefix of [`ContextSections`]
/// and optional [`ValveSelfSnapshot`].
///
/// When `config.self_guide_enabled` is `false`, `self_guide_snapshot` is `None` and writes zero
/// bytes so baseline static prefixes stay bit-identical.
#[must_use]
pub fn compute_static_prefix_digest(
    sections: &ContextSections,
    valve_snapshot: Option<&ValveSelfSnapshot>,
) -> [u8; 32] {
    let mut buf = Vec::with_capacity(512);
    buf.extend_from_slice(b"monoagens.ctx.prefix.v1");

    match &sections.slot_0_anchor {
        Some(anchor) => {
            buf.push(1);
            push_str(&mut buf, &anchor.payload.anchor_id);
            buf.extend_from_slice(&anchor.payload.identity_blob_hash);
            buf.extend_from_slice(&anchor.payload.owner_sig_hash);
            push_str(&mut buf, anchor.signer.as_str());
            buf.extend_from_slice(&anchor.signed_at_ms.as_millis().to_le_bytes());
        }
        None => buf.push(0),
    }

    buf.extend_from_slice(&(sections.slot_0_principles.len() as u32).to_le_bytes());
    for p in &sections.slot_0_principles {
        push_str(&mut buf, &p.payload.id);
        buf.push(match p.payload.severity {
            PrincipleSeverity::HardLine => 1,
            PrincipleSeverity::StrongNorm => 2,
            PrincipleSeverity::Advisory => 3,
        });
        buf.extend_from_slice(&p.payload.owner_sig_hash);
        push_str(&mut buf, p.signer.as_str());
    }

    buf.extend_from_slice(&(sections.static_core_caps.len() as u32).to_le_bytes());
    for cap in &sections.static_core_caps {
        push_str(&mut buf, cap.id.as_str());
        push_str(&mut buf, cap.capability_schema_hash.as_str());
        push_str(&mut buf, &cap.description);
    }

    if let Some(guide) = &sections.self_guide_snapshot {
        buf.push(1);
        buf.extend_from_slice(&guide.protocol_version.major.to_le_bytes());
        buf.extend_from_slice(&guide.protocol_version.minor.to_le_bytes());
        buf.push(u8::from(guide.neural_active));
        buf.push(u8::from(guide.online_adaptation));
        if let Some(prefix) = &guide.model_blake3_prefix {
            buf.push(1);
            push_str(&mut buf, prefix);
        } else {
            buf.push(0);
        }
        buf.extend_from_slice(&(guide.visible_scope_summary.audience_count as u32).to_le_bytes());
        if let Some(place) = &guide.visible_scope_summary.target_place {
            buf.push(1);
            push_str(&mut buf, place.as_str());
        } else {
            buf.push(0);
        }
        buf.extend_from_slice(
            &(guide.visible_scope_summary.active_declass_tokens as u32).to_le_bytes(),
        );
        buf.push(u8::from(guide.auto_recall_state.active));
        buf.push(guide.auto_recall_state.attempt);
        buf.push(guide.auto_recall_state.max_attempts);
    }

    if let Some(valve) = valve_snapshot {
        buf.push(1);
        push_str(&mut buf, &valve.valve_mode);
        buf.push(u8::from(valve.quiet_hours_active));
        buf.extend_from_slice(&(valve.pending_approval_count as u32).to_le_bytes());
        buf.push(u8::from(valve.taint_gating_active));
    }

    blake3::hash(&buf)
}

fn push_str(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

/// Estimates segment byte footprint from payload length and metadata.
#[must_use]
pub fn estimate_record_bytes(rec: &EventRecord) -> usize {
    128 + rec.payload.len() + rec.attachments.len() * 48
}

/// Converts a recorded event into a Spotlighted [`ContextSegment`].
#[must_use]
pub fn segment_from_event(
    slot: ContextSlotKind,
    rec: &EventRecord,
    soul_id: &ActorId,
    basis: Option<MemoryBasis>,
    is_reference_only: bool,
    label: impl Into<String>,
) -> ContextSegment {
    let seg_kind = classify_trust_segment(&rec.actor, &rec.origin_actor, soul_id, rec.taint);
    let est = if is_reference_only {
        64
    } else {
        estimate_record_bytes(rec)
    };
    ContextSegment {
        slot,
        segment_kind: seg_kind,
        event_id: Some(rec.id),
        origin_actor: rec.origin_actor.clone(),
        taint: rec.taint,
        timestamp_ms: Some(rec.timestamp_ms),
        basis,
        recollection: None,
        is_gist: false,
        superseded_by: None,
        attachments: rec.attachments.clone(),
        estimated_bytes: est,
        is_reference_only,
        label: label.into(),
    }
}

/// Converts a recalled [`RecalledItem`] into a Spotlighted [`ContextSegment`].
#[must_use]
pub fn segment_from_recalled(item: &RecalledItem, soul_id: &ActorId) -> ContextSegment {
    let seg_kind = classify_trust_segment(&item.actor, &item.origin_actor, soul_id, item.taint);
    let is_gist = item.source_ids.len() > 1 || item.source_ids.first() != Some(&item.event_id);
    ContextSegment {
        slot: ContextSlotKind::Recalled,
        segment_kind: seg_kind,
        event_id: Some(item.event_id),
        origin_actor: item.origin_actor.clone(),
        taint: item.taint,
        timestamp_ms: Some(item.timestamp_ms),
        basis: Some(item.basis),
        recollection: if item.recollected {
            Some(RecollectionConstraint {
                people: vec![item.actor.clone()],
                place: Some(item.place.clone()),
                time_range: Some((item.timestamp_ms, item.timestamp_ms)),
            })
        } else {
            None
        },
        is_gist,
        superseded_by: item.superseded_by,
        attachments: item.attachments.clone(),
        estimated_bytes: 160 + item.attachments.len() * 48,
        is_reference_only: false,
        label: if is_gist {
            "recalled_gist".to_owned()
        } else {
            "recalled_event".to_owned()
        },
    }
}

/// Enforces `budget_bytes` over volatile sections of `sections`.
///
/// Drops lowest-ranked `recalled` items first, then oldest `place_tail` events, then compacts
/// older non-soul `working_set` observations to references. Never drops Slot 0 (`IdentityAnchor`,
/// `PrincipleDescriptor`s), `trigger`, or fired/overdue `commitments`.
pub fn enforce_context_budget(sections: &mut ContextSections) -> Vec<DroppedContextItem> {
    let mut dropped = Vec::new();
    let compute_total = |s: &ContextSections| -> usize {
        let ws: usize = s.working_set.iter().map(|x| x.estimated_bytes).sum();
        let rec: usize = s.recalled.iter().map(|x| x.estimated_bytes).sum();
        let tail: usize = s.place_tail.iter().map(|x| x.estimated_bytes).sum();
        let comm: usize = s.commitments.iter().map(|x| x.estimated_bytes).sum();
        let trig: usize = s.trigger.as_ref().map_or(0, |t| t.estimated_bytes);
        let cards: usize = s.source_cards.len() * 64 + 128;
        ws + rec + tail + comm + trig + cards
    };

    let mut total = compute_total(sections);
    // 1. Evict lowest-ranked recalled items from tail.
    while total > sections.budget_bytes && !sections.recalled.is_empty() {
        if let Some(evicted) = sections.recalled.pop() {
            total = total.saturating_sub(evicted.estimated_bytes);
            dropped.push(DroppedContextItem {
                slot: ContextSlotKind::Recalled,
                event_id: evicted.event_id,
                estimated_bytes: evicted.estimated_bytes,
                reason: "budget_exceeded".to_owned(),
            });
        }
    }

    // 2. Evict oldest place_tail items from head.
    while total > sections.budget_bytes && !sections.place_tail.is_empty() {
        let evicted = sections.place_tail.remove(0);
        total = total.saturating_sub(evicted.estimated_bytes);
        dropped.push(DroppedContextItem {
            slot: ContextSlotKind::PlaceTail,
            event_id: evicted.event_id,
            estimated_bytes: evicted.estimated_bytes,
            reason: "budget_exceeded".to_owned(),
        });
    }

    // 3. Compact older non-soul working_set observations into references, then drop if needed.
    if total > sections.budget_bytes {
        for seg in &mut sections.working_set {
            if seg.segment_kind == TrustSegmentKind::UntrustedQuoted && !seg.is_reference_only {
                let saved = seg.estimated_bytes.saturating_sub(64);
                seg.is_reference_only = true;
                seg.estimated_bytes = 64;
                total = total.saturating_sub(saved);
                if total <= sections.budget_bytes {
                    break;
                }
            }
        }
    }
    while total > sections.budget_bytes {
        let idx = sections
            .working_set
            .iter()
            .position(|s| s.segment_kind == TrustSegmentKind::UntrustedQuoted);
        let Some(pos) = idx else {
            break;
        };
        let evicted = sections.working_set.remove(pos);
        total = total.saturating_sub(evicted.estimated_bytes);
        dropped.push(DroppedContextItem {
            slot: ContextSlotKind::WorkingSet,
            event_id: evicted.event_id,
            estimated_bytes: evicted.estimated_bytes,
            reason: "budget_exceeded".to_owned(),
        });
    }

    // 4. Evict only non-fired/non-overdue passive commitments if still over budget.
    while total > sections.budget_bytes {
        let idx = sections
            .commitments
            .iter()
            .position(|c| !c.is_fired_or_overdue);
        let Some(pos) = idx else {
            break;
        };
        let evicted = sections.commitments.remove(pos);
        total = total.saturating_sub(evicted.estimated_bytes);
        dropped.push(DroppedContextItem {
            slot: ContextSlotKind::Commitments,
            event_id: Some(evicted.expectation_id),
            estimated_bytes: evicted.estimated_bytes,
            reason: "budget_exceeded".to_owned(),
        });
    }

    sections.total_estimated_bytes = compute_total(sections);
    dropped
}

//! Deterministic prompt renderer from [`ContextPlan`] with Spotlighting (`R-43`),
//! slot-0 `IdentityAnchor` (`R-48`), progressive `Tool RAG` rendering (`R-38`),
//! absolute UTC timestamps, and exhaustive zero-drift `[SELF_GUIDE]` synthesis (`7.6`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use monoagens_core::{
    AutoRecallTriggerState, CommitmentSlotItem, ContextPlan, ContextSections, ContextSegment,
    ContextSlotKind, CoreSelfCard, CoreSelfSnapshot, MechanismId, MemoryBasis, ModelPurpose,
    PrincipleSeverity, ProcedureEntry, SourceLedgerCard, Store, StoreError, TrustSegmentKind,
    ValveSelfSnapshot, self_guide_template_for, self_guide_template_table,
};
use monoagens_protocol::{
    CapabilityDescriptor, CapabilityId, CapabilitySchemaHash, EventId, ExpectKind, Receipt,
    ReceiptOutcome, Time, sort_capabilities,
};

use crate::attachments::ResolvedAttachment;
use crate::embed::estimate_tokens;
use crate::model::{MessageRole, ModelMessage, ModelRequest, ResponseSchemaMode};
use crate::parse::ParseOptions;
use crate::tools::{MAX_STATIC_CORE_CAPS, SelectedTools};

/// Hard token budget for the rendered `[SELF_GUIDE]` prefix (`openspec/.../reasoning/spec.md`, `7.6`).
pub const SELF_GUIDE_MAX_TOKENS: usize = 250;

/// Deterministic token-count estimator (`ceil(bytes / 4)`, minimum 1 for non-empty text).
///
/// Matches [`crate::embed::estimate_tokens`] and [`crate::model::ScriptedReply::text`] so
/// `[SELF_GUIDE]` budget checks are 100% deterministic and offline.
#[must_use]
pub fn estimate_prompt_tokens(text: &str) -> usize {
    estimate_tokens(text)
}

/// Formats an epoch-millisecond [`Time`] as an absolute UTC ISO-8601 / RFC-3339 timestamp
/// (`YYYY-MM-DDTHH:MM:SS.sssZ`), never as a relative offset.
#[must_use]
pub fn format_absolute_utc(time: Time) -> String {
    let ms = time.as_millis();
    let total_secs = ms / 1_000;
    let millis = ms % 1_000;
    let days = (total_secs / 86_400) as i64;
    let day_secs = total_secs % 86_400;
    let hour = day_secs / 3_600;
    let minute = (day_secs % 3_600) / 60;
    let second = day_secs % 60;

    // Howard Hinnant civil-from-days algorithm (exact over proleptic Gregorian calendar).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if month <= 2 { 1 } else { 0 };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Returns the opening and closing Spotlighting delimiters for `kind` (`R-43`).
#[must_use]
pub const fn spotlight_delimiters(kind: TrustSegmentKind) -> (&'static str, &'static str) {
    match kind {
        TrustSegmentKind::System => ("[SYSTEM]", "[/SYSTEM]"),
        TrustSegmentKind::Anchor => ("[ANCHOR]", "[/ANCHOR]"),
        TrustSegmentKind::Trusted => ("[TRUSTED]", "[/TRUSTED]"),
        TrustSegmentKind::UntrustedQuoted => ("[UNTRUSTED_QUOTED]", "[/UNTRUSTED_QUOTED]"),
    }
}

/// Escapes delimiter-like control tags in payload text and quotes untrusted lines with `> `.
///
/// Prevents instruction-looking payloads inside `[UNTRUSTED_QUOTED]` (or other segments) from
/// forging `[ANCHOR]`, `[SYSTEM]`, `[TRUSTED]`, or closing `[/UNTRUSTED_QUOTED]` boundaries.
#[must_use]
pub fn escape_spotlight_payload(kind: TrustSegmentKind, raw: &str) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let escaped = neutralize_control_tags(&normalized);
    match kind {
        TrustSegmentKind::UntrustedQuoted => {
            if escaped.is_empty() {
                "> ".to_owned()
            } else {
                escaped
                    .split('\n')
                    .map(|line| format!("> {line}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
        TrustSegmentKind::System | TrustSegmentKind::Anchor | TrustSegmentKind::Trusted => escaped,
    }
}

fn neutralize_control_tags(input: &str) -> String {
    const TAGS: &[&str] = &[
        "ANCHOR",
        "SYSTEM",
        "TRUSTED",
        "UNTRUSTED_QUOTED",
        "SELF_GUIDE",
        "ANCHOR_CHECK",
        "STATIC_CORE_CAPABILITIES",
        "DYNAMIC_CAPABILITIES",
        "PROCEDURES",
        "SELF_CARD",
        "SOURCE_CARDS",
        "COMMITMENTS",
        "WORKING_SET",
        "RECALLED",
        "PLACE_TAIL",
        "TEMPORAL",
        "TRIGGER",
    ];

    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(open_idx) = rest.find('[') {
        out.push_str(&rest[..open_idx]);
        let after_open = &rest[open_idx + 1..];
        if let Some(close_idx) = after_open.find(']') {
            let candidate = &after_open[..close_idx];
            let trimmed = candidate.trim();
            let bare = trimmed.strip_prefix('/').unwrap_or(trimmed).trim();
            if TAGS.iter().any(|t| bare.eq_ignore_ascii_case(t)) {
                out.push_str("[quoted:");
                out.push_str(trimmed);
                out.push(']');
                rest = &after_open[close_idx + 1..];
                continue;
            }
        }
        out.push('[');
        rest = after_open;
    }
    out.push_str(rest);
    out
}

fn sanitize_single_line(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            '\n' | '\r' | '\t' | '\0' => ' ',
            _ => c,
        })
        .collect();
    neutralize_control_tags(cleaned.trim())
}

/// Exhaustive compile-time switch -> `[SELF_GUIDE]` prose template mapping over all 22 [`MechanismId`]s.
///
/// Adding a mechanism to [`MechanismId`] without adding its template arm here is a compile error.
/// Contains zero tunable thresholds, formula symbols (`theta`, `P_rc`, `beta`), or policy internals.
#[must_use]
pub const fn self_guide_mechanism_line(id: MechanismId) -> &'static str {
    let _entry = self_guide_template_for(id);
    match id {
        MechanismId::Salience => "- m01 salience: signal wake gating",
        MechanismId::FloorCost => "- m02 floor: turn restraint",
        MechanismId::Habituation => "- m03 habituation: damp repeats",
        MechanismId::DriveThreshold => "- m04 drive: wake pacing",
        MechanismId::FamiliarityReserve => "- m05 reserve: new-place caution",
        MechanismId::StochasticGoNoGo => "- m06 go_nogo: seeded wake draw",
        MechanismId::CommitmentBreaker => "- m07 breaker: due-turn bypass",
        MechanismId::Patience => "- m08 patience: wait before reply",
        MechanismId::ReportBarOnThought => "- m09 bars: speak and certainty bars",
        MechanismId::ActivationRecall => "- m10 recall: activation ranking",
        MechanismId::VectorFade => "- m11 fade: stale vector release",
        MechanismId::PredictionAndGrounding => "- m12 grounding: outcome checks",
        MechanismId::ProspectiveMemory => "- m13 promises: track and CommitCheck",
        MechanismId::Credence => "- m14 credence: source reliability",
        MechanismId::BoundedLearning => "- m15 learning: outcome updates",
        MechanismId::MemoryTruth => "- m16 truth: basis, R/F, validity",
        MechanismId::ReportControl => "- m17 report: auto-recall or abstain",
        MechanismId::ConsolidationAndMerge => "- m18 sleep: gists and skills",
        MechanismId::PerceptionLedger => "- m19 ledger: self and actor cards",
        MechanismId::Agency => "- m20 agency: self-effect check",
        MechanismId::EventBoundaries => "- m21 boundaries: episode splits",
        MechanismId::BacklogChunking => "- m22 backlog: downtime batches",
    }
}

/// Compiles [`CoreSelfSnapshot`] and optional [`ValveSelfSnapshot`] into the cacheable
/// `[SELF_GUIDE]` prefix block (`<= 250` tokens).
///
/// Neutral mechanisms (`enabled == false`), disabled Valve modes (`"off"`/`"disabled"`/empty),
/// and inactive Valve rules (`quiet_hours_active == false`, `taint_gating_active == false`, etc.)
/// are omitted completely.
#[must_use]
pub fn render_self_guide(guide: &CoreSelfSnapshot, valve: Option<&ValveSelfSnapshot>) -> String {
    let mut out = String::with_capacity(512);
    out.push_str("[SELF_GUIDE]\n");
    let _ = writeln!(
        out,
        "protocol: v{}.{}",
        guide.protocol_version.major, guide.protocol_version.minor
    );

    let _ = write!(
        out,
        "scope: audience={}",
        guide.visible_scope_summary.audience_count
    );
    if let Some(place) = &guide.visible_scope_summary.target_place {
        let _ = write!(out, " place={}", sanitize_single_line(place.as_str()));
    }
    if guide.visible_scope_summary.active_declass_tokens > 0 {
        let _ = write!(
            out,
            " declass={}",
            guide.visible_scope_summary.active_declass_tokens
        );
    }
    out.push('\n');

    if let Some(v) = valve {
        let mode = v.valve_mode.trim();
        let valve_active = !mode.is_empty()
            && !mode.eq_ignore_ascii_case("off")
            && !mode.eq_ignore_ascii_case("disabled");
        if valve_active {
            let _ = write!(out, "valve: mode={}", sanitize_single_line(mode));
            if v.quiet_hours_active {
                out.push_str(" quiet_hours=active");
            }
            if v.pending_approval_count > 0 {
                let _ = write!(out, " pending_approval={}", v.pending_approval_count);
            }
            if let Some(rem) = v.daily_cap_remaining {
                let _ = write!(out, " daily_cap_remaining={rem}");
            }
            if v.taint_gating_active {
                out.push_str(" taint_gating=active");
            }
            let clean_tags: Vec<String> = v
                .opaque_tags
                .iter()
                .map(|t| sanitize_single_line(t))
                .filter(|t| !t.is_empty())
                .collect();
            if !clean_tags.is_empty() {
                let _ = write!(out, " tags={}", clean_tags.join(","));
            }
            out.push('\n');
        }
    }

    for entry in self_guide_template_table() {
        let enabled = guide
            .active_switches
            .iter()
            .find(|sw| sw.id == entry.id)
            .is_some_and(|sw| sw.enabled);
        if enabled {
            out.push_str(self_guide_mechanism_line(entry.id));
            out.push('\n');
        }
    }

    out.push_str("[/SELF_GUIDE]");
    out
}

/// Optional gateway-supplied enrichments for rendering a [`ContextPlan`].
///
/// All fields default to empty/`None`; calling [`render_context_plan`] without extras still
/// produces a complete, deterministic prompt from [`ContextPlan`] alone.
#[derive(Debug, Clone, Default)]
pub struct RenderExtras {
    /// Decoded UTF-8 payloads for `VisibleScope`-authorized `EventId`s in `plan`.
    pub event_payloads: BTreeMap<EventId, String>,
    /// Optional explicit credence `[0.0, 1.0]` for recalled items.
    pub item_credences: BTreeMap<EventId, f64>,
    /// Optional `Held`/`Refused` receipts carrying canonical Valve/Core `reason_code` tags.
    pub receipts: Vec<Receipt>,
    /// Optional `auto_recall` state override when `self_guide_snapshot` is absent.
    pub auto_recall_override: Option<AutoRecallTriggerState>,
    /// Optional full active capability schema hashes for defensive drift checking of procedures.
    pub known_capability_hashes: BTreeMap<CapabilityId, CapabilitySchemaHash>,
    /// Authorized attachments (resolved strictly after `VisibleScope` & door checks) for `ModelRequest`.
    pub attachments: Vec<ResolvedAttachment>,
}

impl RenderExtras {
    /// Populates `event_payloads` from `store` strictly for `EventId`s already present in `plan`.
    pub fn populate_payloads_from_store(
        &mut self,
        plan: &ContextPlan,
        store: &dyn Store,
    ) -> Result<(), StoreError> {
        let mut ids = BTreeSet::new();
        for seg in plan
            .sections
            .working_set
            .iter()
            .chain(plan.sections.recalled.iter())
            .chain(plan.sections.place_tail.iter())
            .chain(plan.sections.trigger.iter())
        {
            if !seg.is_reference_only
                && let Some(eid) = seg.event_id
            {
                ids.insert(eid);
            }
        }
        for eid in ids {
            if let Some(rec) = store.get(eid)?
                && !rec.payload.is_empty()
            {
                let text = String::from_utf8_lossy(rec.payload.as_bytes()).into_owned();
                self.event_payloads.insert(eid, text);
            }
        }
        Ok(())
    }
}

/// Populates [`ContextSections`] tool fields from [`SelectedTools`] (`7.5` consumer side).
pub fn apply_selected_tools(sections: &mut ContextSections, selected: &SelectedTools) {
    let mut static_caps = selected.static_core.clone();
    sort_capabilities(&mut static_caps);
    static_caps.truncate(MAX_STATIC_CORE_CAPS);
    sections.static_core_caps = static_caps;

    let mut dyn_caps = selected.dynamic_caps.clone();
    for cap in &mut dyn_caps {
        cap.places.sort();
        cap.places.dedup();
    }
    sections.dynamic_caps = dyn_caps;

    sections.procedures = selected
        .procedures
        .iter()
        .map(|p| monoagens_core::ProcedureEntry {
            procedure_id: p.procedure_id,
            name: p.name.clone(),
            steps: p.steps.clone(),
            required_cap_ids: p.required_cap_ids.clone(),
            capability_schema_hash: p.capability_schema_hash.clone(),
            audience: p.audience.clone(),
            domain: None,
            active: p.active,
            retired: p.retired,
            owner_signed: false,
            embedding: p.embedding.clone(),
        })
        .collect();
    sections.excluded_stale_procedures = selected.excluded_stale_procedures;
}

/// Deterministic rendered prompt output built from a [`ContextPlan`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedContext {
    /// Cacheable prefix (`[ANCHOR]`, `[STATIC_CORE_CAPABILITIES]`, optional `[SELF_GUIDE]`).
    pub static_prefix: String,
    /// Volatile per-wake sections (`[DYNAMIC_CAPABILITIES]` .. `[TRIGGER]`).
    pub dynamic_context: String,
    /// Complete rendered prompt in exact [`ContextPlan`] slot order.
    pub full_prompt: String,
    /// Rendered `[SELF_GUIDE]` block when `self_mechanics_guide` is enabled.
    pub self_guide: Option<String>,
    /// Measured token count of `self_guide` via [`estimate_prompt_tokens`].
    pub self_guide_tokens: usize,
    /// Just-in-time working-set cue lines emitted (`auto_recall`, held/refused receipts).
    pub working_set_cues: Vec<String>,
    /// Number of schema-drifted/inactive procedures excluded (including render-time checks).
    pub excluded_stale_procedures: usize,
    /// Plan's static prefix digest carried from Core.
    pub static_prefix_digest: [u8; 32],
    /// Structural sequence of slots rendered, in exact output order.
    pub slots_in_order: Vec<ContextSlotKind>,
}

impl RenderedContext {
    /// Packages the rendered context into a stateless [`ModelRequest`] ready for any [`crate::ModelProvider`].
    #[must_use]
    pub fn into_model_request(
        &self,
        request_id: EventId,
        purpose: ModelPurpose,
        model: impl Into<String>,
        attachments: Vec<ResolvedAttachment>,
    ) -> ModelRequest {
        let response_schema = match purpose {
            ModelPurpose::Reason | ModelPurpose::Consolidate => {
                ResponseSchemaMode::ThoughtBatchStrict
            }
            ModelPurpose::Classify => ResponseSchemaMode::ClassificationVerdictStrict,
        };
        let user_msg = if attachments.is_empty() {
            ModelMessage::text(MessageRole::User, &self.dynamic_context)
        } else {
            ModelMessage::with_attachments(MessageRole::User, &self.dynamic_context, attachments)
        };
        ModelRequest {
            request_id,
            purpose,
            model: model.into(),
            messages: vec![
                ModelMessage::text(MessageRole::System, &self.static_prefix),
                user_msg,
            ],
            response_schema,
            parse_options: ParseOptions::default(),
            timeout_ms: 30_000,
            max_output_tokens: None,
            temperature: None,
        }
    }
}

/// Renders a [`ContextPlan`] deterministically with default empty [`RenderExtras`].
#[must_use]
pub fn render_context_plan(plan: &ContextPlan) -> RenderedContext {
    render_context_plan_with(plan, &RenderExtras::default())
}

/// Renders a [`ContextPlan`] + [`RenderExtras`] into a deterministic prompt following exact plan order.
#[must_use]
pub fn render_context_plan_with(plan: &ContextPlan, extras: &RenderExtras) -> RenderedContext {
    let sections = &plan.sections;
    let mut slots_in_order = Vec::with_capacity(14);

    // --- 1. Cacheable static prefix: Slot 0 [ANCHOR], Static Core Capabilities, optional [SELF_GUIDE] ---
    let mut static_prefix = String::with_capacity(1024);

    // Slot 0: IdentityAnchor + OwnerSigned PrincipleDescriptors (never dropped)
    slots_in_order.push(ContextSlotKind::Slot0Anchor);
    static_prefix.push_str(&render_slot_0_anchor(sections));

    // Static Core Capabilities (<= 4 OwnerSigned tools, deterministically sorted)
    slots_in_order.push(ContextSlotKind::StaticCoreCaps);
    static_prefix.push('\n');
    static_prefix.push_str(&render_static_core_caps(&sections.static_core_caps));

    // Optional [SELF_GUIDE] (only when CoreSelfSnapshot is present, i.e. self_mechanics_guide = true)
    let (self_guide, self_guide_tokens) = if let Some(guide) = &sections.self_guide_snapshot {
        slots_in_order.push(ContextSlotKind::SelfGuide);
        if plan.valve_self_snapshot.is_some() {
            slots_in_order.push(ContextSlotKind::ValveSelfGuide);
        }
        let rendered_guide = render_self_guide(guide, plan.valve_self_snapshot.as_ref());
        let tok_count = estimate_prompt_tokens(&rendered_guide);
        static_prefix.push('\n');
        static_prefix.push_str(&rendered_guide);
        (Some(rendered_guide), tok_count)
    } else {
        (None, 0)
    };

    // --- 2. Dynamic disclosure, cards, commitments, working set, recall, place tail, temporal, trigger ---
    let mut dynamic_context = String::with_capacity(2048);

    // Dynamic Capabilities
    slots_in_order.push(ContextSlotKind::DynamicCaps);
    dynamic_context.push_str(&render_dynamic_caps(&sections.dynamic_caps));

    // Procedures (with defensive schema-hash validation)
    slots_in_order.push(ContextSlotKind::Procedures);
    let (proc_text, extra_stale) = render_procedures(sections, extras);
    dynamic_context.push('\n');
    dynamic_context.push_str(&proc_text);

    // 4-pillar self_card
    slots_in_order.push(ContextSlotKind::SelfCard);
    dynamic_context.push('\n');
    dynamic_context.push_str(&render_self_card(&sections.self_card));

    // Source ledger cards
    slots_in_order.push(ContextSlotKind::SourceCards);
    dynamic_context.push('\n');
    dynamic_context.push_str(&render_source_cards(&sections.source_cards));

    // Commitments
    slots_in_order.push(ContextSlotKind::Commitments);
    dynamic_context.push('\n');
    dynamic_context.push_str(&render_commitments(&sections.commitments));

    // Working set + just-in-time working-set cues
    slots_in_order.push(ContextSlotKind::WorkingSet);
    let (ws_text, working_set_cues) = render_working_set(sections, extras);
    dynamic_context.push('\n');
    dynamic_context.push_str(&ws_text);

    // Recalled items
    slots_in_order.push(ContextSlotKind::Recalled);
    dynamic_context.push('\n');
    dynamic_context.push_str(&render_recalled(sections, extras));

    // Place tail
    slots_in_order.push(ContextSlotKind::PlaceTail);
    dynamic_context.push('\n');
    dynamic_context.push_str(&render_place_tail(sections, extras));

    // Temporal (<now> + knowledge cutoff)
    slots_in_order.push(ContextSlotKind::Temporal);
    dynamic_context.push('\n');
    dynamic_context.push_str(&render_temporal(sections));

    // Triggering event LAST
    slots_in_order.push(ContextSlotKind::Trigger);
    dynamic_context.push('\n');
    dynamic_context.push_str(&render_trigger(sections, extras));

    let full_prompt = format!("{static_prefix}\n{dynamic_context}");

    RenderedContext {
        static_prefix,
        dynamic_context,
        full_prompt,
        self_guide,
        self_guide_tokens,
        working_set_cues,
        excluded_stale_procedures: sections.excluded_stale_procedures + extra_stale,
        static_prefix_digest: plan.static_prefix_digest,
        slots_in_order,
    }
}

/// Renders `plan` + `extras` and constructs a ready-to-send [`ModelRequest`].
#[must_use]
pub fn build_model_request_from_plan(
    plan: &ContextPlan,
    extras: &RenderExtras,
    request_id: EventId,
    purpose: ModelPurpose,
    model: impl Into<String>,
) -> ModelRequest {
    let rendered = render_context_plan_with(plan, extras);
    rendered.into_model_request(request_id, purpose, model, extras.attachments.clone())
}

fn render_slot_0_anchor(sections: &ContextSections) -> String {
    let mut out = String::new();
    out.push_str("[ANCHOR]\n");
    match &sections.slot_0_anchor {
        Some(signed) => {
            let _ = writeln!(
                out,
                "anchor_id: {} signer={} signed_at={} blob_hash={}",
                sanitize_single_line(&signed.payload.anchor_id),
                sanitize_single_line(signed.signer.as_str()),
                format_absolute_utc(signed.signed_at_ms),
                encode_hex(&signed.payload.identity_blob_hash),
            );
            let raw_payload = String::from_utf8_lossy(signed.payload.payload.as_bytes());
            let clean_payload = escape_spotlight_payload(TrustSegmentKind::Anchor, &raw_payload);
            out.push_str(&clean_payload);
            if !clean_payload.ends_with('\n') {
                out.push('\n');
            }
        }
        None => {
            out.push_str("anchor: none\n");
        }
    }

    if !sections.slot_0_principles.is_empty() {
        out.push_str("principles:\n");
        for p in &sections.slot_0_principles {
            let sev = match p.payload.severity {
                PrincipleSeverity::HardLine => "hard_line",
                PrincipleSeverity::StrongNorm => "strong_norm",
                PrincipleSeverity::Advisory => "advisory",
            };
            let blocked = if p.payload.blocked_capabilities.is_empty() {
                "none".to_owned()
            } else {
                p.payload
                    .blocked_capabilities
                    .iter()
                    .map(|c| sanitize_single_line(c.as_str()))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let _ = writeln!(
                out,
                "- id={} severity={} forbid_identity_change={} forbid_untrusted_taint={} blocked_caps=[{}] signer={}",
                sanitize_single_line(&p.payload.id),
                sev,
                p.payload.forbid_identity_change,
                p.payload.forbid_untrusted_taint,
                blocked,
                sanitize_single_line(p.signer.as_str()),
            );
        }
    }
    out.push_str("[/ANCHOR]");
    out
}

fn render_static_core_caps(caps: &[CapabilityDescriptor]) -> String {
    let mut sorted = caps.to_vec();
    sort_capabilities(&mut sorted);
    sorted.truncate(MAX_STATIC_CORE_CAPS);

    let mut out = String::new();
    out.push_str("[STATIC_CORE_CAPABILITIES]\n");
    if sorted.is_empty() {
        out.push_str("- none\n");
    } else {
        for cap in &sorted {
            write_capability_line(&mut out, cap);
        }
    }
    out.push_str("[/STATIC_CORE_CAPABILITIES]");
    out
}

fn render_dynamic_caps(caps: &[CapabilityDescriptor]) -> String {
    let mut out = String::new();
    out.push_str("[DYNAMIC_CAPABILITIES]\n");
    if caps.is_empty() {
        out.push_str("- none\n");
    } else {
        for cap in caps {
            write_capability_line(&mut out, cap);
        }
    }
    out.push_str("[/DYNAMIC_CAPABILITIES]");
    out
}

fn write_capability_line(out: &mut String, cap: &CapabilityDescriptor) {
    let mut places: Vec<String> = cap
        .places
        .iter()
        .map(|p| sanitize_single_line(p.as_str()))
        .collect();
    places.sort();
    places.dedup();
    let _ = writeln!(
        out,
        "- {} schema_hash={} places=[{}] comm={} id_chg={} irrev={} latency_ms={} desc={}",
        sanitize_single_line(cap.id.as_str()),
        sanitize_single_line(cap.capability_schema_hash.as_str()),
        places.join(","),
        cap.flags.communicative,
        cap.flags.identity_changing,
        cap.flags.irreversible,
        cap.typical_latency_ms,
        sanitize_single_line(&cap.description),
    );
}

fn render_procedures(sections: &ContextSections, extras: &RenderExtras) -> (String, usize) {
    let mut active_hashes: BTreeMap<&str, &str> = BTreeMap::new();
    for (id, hash) in &extras.known_capability_hashes {
        active_hashes.insert(id.as_str(), hash.as_str());
    }
    for cap in sections
        .static_core_caps
        .iter()
        .chain(sections.dynamic_caps.iter())
    {
        active_hashes.insert(cap.id.as_str(), cap.capability_schema_hash.as_str());
    }

    let mut out = String::new();
    out.push_str("[PROCEDURES]\n");
    let mut rendered_count = 0usize;
    let mut extra_stale = 0usize;

    for proc_item in &sections.procedures {
        if !procedure_schema_valid(
            proc_item,
            &active_hashes,
            !extras.known_capability_hashes.is_empty(),
        ) {
            extra_stale += 1;
            continue;
        }
        let reqs: Vec<String> = proc_item
            .required_cap_ids
            .iter()
            .map(|c| sanitize_single_line(c.as_str()))
            .collect();
        let steps: Vec<String> = proc_item
            .steps
            .iter()
            .map(|s| sanitize_single_line(s))
            .collect();
        let _ = writeln!(
            out,
            "- id={} name={} schema_hash={} requires=[{}] steps=[{}]",
            proc_item.procedure_id,
            sanitize_single_line(&proc_item.name),
            sanitize_single_line(proc_item.capability_schema_hash.as_str()),
            reqs.join(","),
            steps.join(" -> "),
        );
        rendered_count += 1;
    }

    if rendered_count == 0 {
        out.push_str("- none\n");
    }
    out.push_str("[/PROCEDURES]");
    (out, extra_stale)
}

fn procedure_schema_valid(
    proc_item: &ProcedureEntry,
    active_hashes: &BTreeMap<&str, &str>,
    require_known: bool,
) -> bool {
    if !proc_item.active || proc_item.retired || proc_item.required_cap_ids.is_empty() {
        return false;
    }
    proc_item
        .required_cap_ids
        .iter()
        .all(|req_id| match active_hashes.get(req_id.as_str()) {
            Some(&active_hash) => active_hash == proc_item.capability_schema_hash.as_str(),
            None => !require_known,
        })
}

fn render_self_card(card: &CoreSelfCard) -> String {
    let mut out = String::new();
    out.push_str("[SELF_CARD]\n");
    let _ = writeln!(
        out,
        "pillar_1_reliability: alpha={:.3} beta={:.3} mean_delivery_latency_ms={}",
        card.reliability_alpha, card.reliability_beta, card.mean_delivery_latency_ms
    );
    let _ = writeln!(
        out,
        "pillar_2_calibration: samples={} brier={:.4}",
        card.calibration_samples, card.calibration_brier
    );
    let _ = writeln!(
        out,
        "pillar_3_budget: headroom_ratio={:.4}",
        card.budget_headroom_ratio
    );
    let _ = writeln!(
        out,
        "pillar_4_principles: alpha={:.3} beta={:.3} contradiction_flags={}",
        card.principle_alpha, card.principle_beta, card.contradiction_flags
    );
    out.push_str("[/SELF_CARD]");
    out
}

fn render_source_cards(cards: &[SourceLedgerCard]) -> String {
    let mut out = String::new();
    out.push_str("[SOURCE_CARDS]\n");
    if cards.is_empty() {
        out.push_str("- none\n");
    } else {
        for c in cards {
            let _ = writeln!(
                out,
                "- actor={} alpha={:.3} beta={:.3} closeness={:.4} trusted_origin={}",
                sanitize_single_line(c.actor_id.as_str()),
                c.reliability_alpha,
                c.reliability_beta,
                c.closeness,
                c.trusted_origin,
            );
        }
    }
    out.push_str("[/SOURCE_CARDS]");
    out
}

fn render_commitments(commitments: &[CommitmentSlotItem]) -> String {
    let mut out = String::new();
    out.push_str("[COMMITMENTS]\n");
    if commitments.is_empty() {
        out.push_str("- none\n");
    } else {
        for c in commitments {
            let kind = match c.kind {
                ExpectKind::Response => "response",
                ExpectKind::Effect => "effect",
                ExpectKind::Promise => "promise",
                ExpectKind::Goal => "goal",
                ExpectKind::Monitor => "monitor",
                ExpectKind::Wait => "wait",
                ExpectKind::PendingApproval => "pending_approval",
            };
            let places: Vec<String> = c
                .places
                .iter()
                .map(|p| sanitize_single_line(p.as_str()))
                .collect();
            let people: Vec<String> = c
                .people
                .iter()
                .map(|a| sanitize_single_line(a.as_str()))
                .collect();
            let _ = writeln!(
                out,
                "- id={} kind={} status={} due={} expiry={} places=[{}] people=[{}]",
                c.expectation_id,
                kind,
                if c.is_fired_or_overdue {
                    "overdue_or_due"
                } else {
                    "active"
                },
                format_absolute_utc(c.due_ms),
                format_absolute_utc(c.expiry_ms),
                places.join(","),
                people.join(","),
            );
        }
    }
    out.push_str("[/COMMITMENTS]");
    out
}

fn render_working_set(sections: &ContextSections, extras: &RenderExtras) -> (String, Vec<String>) {
    let mut out = String::new();
    out.push_str("[WORKING_SET]\n");

    let mut cues: Vec<String> = Vec::new();
    let auto_recall = sections
        .self_guide_snapshot
        .as_ref()
        .map(|g| &g.auto_recall_state)
        .or(extras.auto_recall_override.as_ref());
    if let Some(ar) = auto_recall
        && ar.active
    {
        let mut line = format!(
            "- cue: auto_recall active (attempt {}/{}",
            ar.attempt, ar.max_attempts
        );
        if let Some(reason) = &ar.last_reason {
            let clean_r = sanitize_single_line(reason);
            if !clean_r.is_empty() {
                let _ = write!(line, ", reason={clean_r}");
            }
        }
        if ar.coarser_grain_allowed {
            line.push_str(", coarser_grain_allowed=true");
        }
        line.push(')');
        cues.push(line);
    }

    let mut seen_raw_cues: BTreeSet<String> = BTreeSet::new();
    let plan_cues = sections.working_set_cues.iter().chain(
        sections
            .self_guide_snapshot
            .iter()
            .flat_map(|g| g.recent_receipt_cues.iter()),
    );
    for raw_cue in plan_cues {
        let clean = sanitize_single_line(raw_cue);
        if !clean.is_empty() && seen_raw_cues.insert(clean.clone()) {
            cues.push(format!("- cue: receipt {clean}"));
        }
    }

    for receipt in &extras.receipts {
        let outcome_str = match receipt.outcome {
            ReceiptOutcome::Held => "held",
            ReceiptOutcome::Refused => "refused",
            _ => continue,
        };
        if let Some(tag) = &receipt.reason_code {
            let clean_tag = sanitize_single_line(tag);
            if !clean_tag.is_empty() {
                let line = format!(
                    "- cue: receipt outcome={outcome_str} capability={} action={} reason={clean_tag}",
                    sanitize_single_line(receipt.capability_id.as_str()),
                    sanitize_single_line(receipt.action_id.as_str()),
                );
                if seen_raw_cues.insert(line.clone()) {
                    cues.push(line);
                }
            }
        }
    }

    for cue in &cues {
        out.push_str(cue);
        out.push('\n');
    }

    if sections.working_set.is_empty() && cues.is_empty() {
        out.push_str("- none\n");
    } else {
        for seg in &sections.working_set {
            out.push_str(&render_segment(seg, extras, None));
            out.push('\n');
        }
    }

    out.push_str("[/WORKING_SET]");
    (out, cues)
}

fn render_recalled(sections: &ContextSections, extras: &RenderExtras) -> String {
    let mut out = String::new();
    out.push_str("[RECALLED]\n");
    if sections.recalled.is_empty() {
        out.push_str("- none\n");
    } else {
        for seg in &sections.recalled {
            let credence = resolve_recalled_credence(seg, &sections.source_cards, extras);
            out.push_str(&render_segment(seg, extras, credence));
            out.push('\n');
        }
    }
    out.push_str("[/RECALLED]");
    out
}

fn resolve_recalled_credence(
    seg: &ContextSegment,
    source_cards: &[SourceLedgerCard],
    extras: &RenderExtras,
) -> Option<f64> {
    if let Some(eid) = seg.event_id
        && let Some(&c) = extras.item_credences.get(&eid)
        && c.is_finite()
    {
        return Some(c.clamp(0.0, 1.0));
    }
    if seg.basis == Some(MemoryBasis::Told) {
        let origin = seg.origin_actor.actor_id();
        if let Some(card) = source_cards.iter().find(|c| &c.actor_id == origin) {
            let denom = card.reliability_alpha + card.reliability_beta;
            if denom > 0.0 && denom.is_finite() {
                return Some((card.reliability_alpha / denom).clamp(0.0, 1.0));
            }
        }
    }
    None
}

fn render_place_tail(sections: &ContextSections, extras: &RenderExtras) -> String {
    let mut out = String::new();
    out.push_str("[PLACE_TAIL]\n");
    if sections.place_tail.is_empty() {
        out.push_str("- none\n");
    } else {
        for seg in &sections.place_tail {
            out.push_str(&render_segment(seg, extras, None));
            out.push('\n');
        }
    }
    out.push_str("[/PLACE_TAIL]");
    out
}

fn render_temporal(sections: &ContextSections) -> String {
    let mut out = String::new();
    out.push_str("[TEMPORAL]\n");
    let _ = writeln!(out, "<now>{}</now>", format_absolute_utc(sections.now_ms));
    match &sections.knowledge_cutoff {
        Some(cutoff) if !cutoff.trim().is_empty() => {
            let _ = writeln!(out, "knowledge_cutoff: {}", sanitize_single_line(cutoff));
        }
        _ => {
            out.push_str("knowledge_cutoff: none\n");
        }
    }
    if sections.anchor_check_due {
        out.push_str("anchor_check_due: true\n");
    }
    out.push_str("[/TEMPORAL]");
    out
}

fn render_trigger(sections: &ContextSections, extras: &RenderExtras) -> String {
    let mut out = String::new();
    out.push_str("[TRIGGER]\n");
    match &sections.trigger {
        Some(seg) => {
            out.push_str(&render_segment(seg, extras, None));
            out.push('\n');
        }
        None => {
            out.push_str("trigger: none\n");
        }
    }
    out.push_str("[/TRIGGER]");
    out
}

/// Renders a single provenance-bearing [`ContextSegment`] wrapped in its Spotlighting delimiters.
#[must_use]
pub fn render_segment(
    seg: &ContextSegment,
    extras: &RenderExtras,
    credence: Option<f64>,
) -> String {
    let (open, close) = spotlight_delimiters(seg.segment_kind);
    let mut out = String::new();
    out.push_str(open);
    out.push('\n');

    let id_str = seg
        .event_id
        .map_or_else(|| "none".to_owned(), |id| id.to_string());
    let time_str = seg
        .timestamp_ms
        .map_or_else(|| "unknown".to_owned(), format_absolute_utc);
    let basis_str = match seg.basis {
        Some(MemoryBasis::Observed) => "observed",
        Some(MemoryBasis::Told) => "told",
        Some(MemoryBasis::Inferred) => "inferred",
        Some(MemoryBasis::SelfCaused) => "self_caused",
        None => "unspecified",
    };
    let taint_str = match seg.taint {
        monoagens_protocol::Taint::Untrusted => "untrusted",
        monoagens_protocol::Taint::Provisional => "provisional",
        monoagens_protocol::Taint::Trusted => "trusted",
    };

    let _ = write!(
        out,
        "id={id_str} by={} taint={taint_str} time={time_str} basis={basis_str}",
        sanitize_single_line(seg.origin_actor.actor_id().as_str()),
    );

    if seg.slot == monoagens_core::ContextSlotKind::Recalled {
        let memory_mode = if seg.recollection.is_some() {
            "recollected"
        } else {
            "familiar"
        };
        let form = if seg.is_gist { "gist" } else { "verbatim" };
        let validity = match seg.superseded_by {
            Some(sup_id) => format!("superseded_by:{sup_id}"),
            None => "current".to_owned(),
        };
        let _ = write!(out, " memory={memory_mode} form={form} validity={validity}");
        if let Some(c) = credence {
            let _ = write!(out, " credence={c:.3}");
        }
    } else if seg.is_reference_only {
        out.push_str(" ref_only=true");
    }

    if !seg.attachments.is_empty() {
        let att_meta: Vec<String> = seg
            .attachments
            .iter()
            .map(|a| {
                format!(
                    "{}:{:?}:{}B",
                    sanitize_single_line(&a.handle_id),
                    a.modality_slot,
                    a.byte_len
                )
            })
            .collect();
        let _ = write!(out, " attachments=[{}]", att_meta.join(","));
    }
    out.push('\n');

    let body = if seg.is_reference_only {
        format!("ref:{}", sanitize_single_line(&seg.label))
    } else if let Some(eid) = seg.event_id
        && let Some(payload) = extras.event_payloads.get(&eid)
    {
        payload.clone()
    } else {
        seg.label.clone()
    };

    let escaped_body = escape_spotlight_payload(seg.segment_kind, &body);
    out.push_str(&escaped_body);
    if !escaped_body.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(close);
    out
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

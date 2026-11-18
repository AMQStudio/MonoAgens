//! Strict `R | null` schema generation, output parsing, and rubric decoding.
//!
//! Enforces strict nullable JSON schemas for `think`, `act`, `expect`, `resolve`,
//! `revise`, and `consolidate` (`gist`, `lesson`, `procedure`), decodes `[0, 20]`
//! confidence (`c_raw = k / 20`) and balanced `1..=5` motivation, falls back to a
//! single `think` act on act-less plain text, and returns diagnostic [`FailedWake`]
//! errors naming the offending field on any schema violation.

use core::fmt;

use monoagens_core::Input;
use monoagens_protocol::schema::generate_json_schema;
use monoagens_protocol::{
    CURRENT_PROTOCOL_VERSION, ClassificationVerdict, ConsolidateOutput, EventId, ProtocolVersion,
    ThoughtAct, ThoughtBatch, check_version,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Balanced `1..=5` motivation level decoded from `think` / `act` acts (`A-10`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MotivationLevel {
    /// `1`: strong preference to stay silent.
    DoNotSpeak = 1,
    /// `2`: mild preference to stay silent.
    PreferSilence = 2,
    /// `3`: balanced / neutral.
    Neutral = 3,
    /// `4`: mild preference to speak / act.
    PreferSpeak = 4,
    /// `5`: strong necessity to speak / act.
    MustSpeak = 5,
}

impl MotivationLevel {
    /// Decodes a `1..=5` integer into [`MotivationLevel`].
    pub fn from_u8(value: u8) -> Result<Self, FailedWake> {
        match value {
            1 => Ok(Self::DoNotSpeak),
            2 => Ok(Self::PreferSilence),
            3 => Ok(Self::Neutral),
            4 => Ok(Self::PreferSpeak),
            5 => Ok(Self::MustSpeak),
            _ => Err(FailedWake::new(
                "motivation",
                format!("motivation {value} must be an integer in 1..=5"),
            )),
        }
    }
}

/// Decoded rubric values for a `think` or `act` step (`A-10`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DecodedRubric {
    pub motivation: u8,
    pub motivation_level: MotivationLevel,
    pub confidence_0_20: u8,
    /// Raw uncalibrated confidence `k / 20.0` in `[0.0, 1.0]`.
    pub c_raw: f64,
}

/// Decodes and validates `[1, 5]` motivation and `[0, 20]` confidence into `c_raw = k / 20`.
pub fn decode_rubric(motivation: u8, confidence_0_20: u8) -> Result<DecodedRubric, FailedWake> {
    let motivation_level = MotivationLevel::from_u8(motivation)?;
    if confidence_0_20 > 20 {
        return Err(FailedWake::new(
            "confidence_0_20",
            format!("confidence_0_20 {confidence_0_20} must be an integer in 0..=20"),
        ));
    }
    Ok(DecodedRubric {
        motivation,
        motivation_level,
        confidence_0_20,
        c_raw: f64::from(confidence_0_20) / 20.0,
    })
}

/// Diagnostic error returned when model output violates the strict `R | null` contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedWake {
    pub field: String,
    pub reason: String,
    pub act_index: Option<usize>,
    pub act_kind: Option<String>,
}

impl FailedWake {
    /// Constructs a `FailedWake` error naming the wrong `field`.
    #[must_use]
    pub fn new(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
            act_index: None,
            act_kind: None,
        }
    }

    /// Attaches the offending act index and act kind for diagnostics.
    #[must_use]
    pub fn with_act(mut self, index: usize, kind: Option<String>) -> Self {
        self.act_index = Some(index);
        self.act_kind = kind;
        self
    }

    /// Converts this failure into a `Core` [`Input::ModelFailure`] so the cognitive loop stays alive.
    #[must_use]
    pub fn to_model_failure_input(&self, request_id: EventId) -> Input {
        Input::ModelFailure {
            request_id,
            reason: self.to_string(),
        }
    }
}

impl fmt::Display for FailedWake {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "failed_wake on field '{}': {}", self.field, self.reason)
    }
}

impl std::error::Error for FailedWake {}

/// Options controlling strict `R | null` output decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ParseOptions {
    /// When `true`, every optional `R | null` field must be explicitly present (either `R` or `null`).
    pub require_explicit_nulls: bool,
    /// Protocol version supported by this caller (defaults to `CURRENT_PROTOCOL_VERSION`).
    pub supported_version: Option<ProtocolVersion>,
}

impl ParseOptions {
    /// Strict mode requiring explicit `null` for all optional `R | null` keys.
    #[must_use]
    pub const fn strict_explicit_nulls() -> Self {
        Self {
            require_explicit_nulls: true,
            supported_version: None,
        }
    }
}

/// Decoded output of a reasoning or consolidation model call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedModelActs {
    pub protocol_version: ProtocolVersion,
    pub acts: Vec<ThoughtAct>,
    pub rubrics: Vec<Option<DecodedRubric>>,
    /// `true` when the model returned act-less plain text and fell back to one `think` act.
    pub fallback_plain_text: bool,
}

/// Generates the strict JSON Schema Draft 2020-12 for [`ThoughtBatch`] with
/// `additionalProperties: false` and all properties listed in `required` (`R | null`).
#[must_use]
pub fn strict_thought_batch_schema() -> Value {
    let mut schema = generate_json_schema::<ThoughtBatch>();
    tighten_object_schemas(&mut schema);
    tighten_thought_batch_constraints(&mut schema);
    schema
}

/// Generates the strict JSON Schema Draft 2020-12 for [`ClassificationVerdict`].
#[must_use]
pub fn strict_classification_verdict_schema() -> Value {
    let mut schema = generate_json_schema::<ClassificationVerdict>();
    tighten_object_schemas(&mut schema);
    tighten_classification_verdict_constraints(&mut schema);
    schema
}

fn tighten_thought_batch_constraints(val: &mut Value) {
    match val {
        Value::Object(map) => {
            if let Some(Value::Object(props)) = map.get_mut("properties") {
                if let Some(Value::Object(mot)) = props.get_mut("motivation") {
                    mot.insert("minimum".to_owned(), Value::from(1));
                    mot.insert("maximum".to_owned(), Value::from(5));
                }
                if let Some(Value::Object(conf)) = props.get_mut("confidence_0_20") {
                    conf.insert("minimum".to_owned(), Value::from(0));
                    conf.insert("maximum".to_owned(), Value::from(20));
                }
                if let Some(Value::Object(cap)) = props.get_mut("capability_id") {
                    cap.insert("minLength".to_owned(), Value::from(1));
                }
                let is_act_variant = props.get("act").is_some_and(|act_prop| {
                    act_prop.get("const").and_then(Value::as_str) == Some("act")
                        || act_prop
                            .get("enum")
                            .and_then(Value::as_array)
                            .is_some_and(|arr| arr.len() == 1 && arr[0].as_str() == Some("act"))
                });
                if is_act_variant && let Some(Value::Object(places)) = props.get_mut("places") {
                    places.insert("minItems".to_owned(), Value::from(1));
                }
            }
            for child in map.values_mut() {
                tighten_thought_batch_constraints(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                tighten_thought_batch_constraints(item);
            }
        }
        _ => {}
    }
}

fn tighten_classification_verdict_constraints(val: &mut Value) {
    if let Value::Object(map) = val
        && let Some(Value::Object(props)) = map.get_mut("properties")
        && let Some(Value::Object(label)) = props.get_mut("label")
    {
        label.insert("minLength".to_owned(), Value::from(1));
    }
}

fn tighten_object_schemas(val: &mut Value) {
    match val {
        Value::Object(map) => {
            if let Some(Value::Object(props)) = map.get("properties") {
                let mut req_keys: Vec<Value> =
                    props.keys().map(|k| Value::String(k.clone())).collect();
                req_keys.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
                map.insert("additionalProperties".to_owned(), Value::Bool(false));
                map.insert("required".to_owned(), Value::Array(req_keys));
            }
            for child in map.values_mut() {
                tighten_object_schemas(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                tighten_object_schemas(item);
            }
        }
        _ => {}
    }
}

/// Parses raw model output using default [`ParseOptions`].
pub fn parse_thought_output(raw: &str) -> Result<ParsedModelActs, FailedWake> {
    parse_thought_output_with_options(raw, ParseOptions::default())
}

/// Parses raw model output into ordered [`ThoughtAct`]s or a diagnostic [`FailedWake`].
///
/// - Non-JSON, non-empty plain text falls back to a single `ThoughtAct::Think`.
/// - Empty text returns `FailedWake` naming `"output"`.
/// - JSON starting with `{` or `[` is validated strictly against the `R | null` schemas,
///   preserving model act order and rejecting malformed or unknown fields with the field name.
pub fn parse_thought_output_with_options(
    raw: &str,
    opts: ParseOptions,
) -> Result<ParsedModelActs, FailedWake> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FailedWake::new("output", "model returned empty output"));
    }

    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        let rubric = decode_rubric(3, 0)?;
        let think = ThoughtAct::Think {
            thought: trimmed.to_owned(),
            motivation: rubric.motivation,
            confidence_0_20: rubric.confidence_0_20,
            used_ids: Vec::new(),
            importance: None,
            basis: None,
            value: None,
            cost: None,
            domain: None,
        };
        return Ok(ParsedModelActs {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            acts: vec![think],
            rubrics: vec![Some(rubric)],
            fallback_plain_text: true,
        });
    }

    let json_val: Value = serde_json::from_str(trimmed)
        .map_err(|err| FailedWake::new("json", format!("malformed JSON output: {err}")))?;

    let supported = opts.supported_version.unwrap_or(CURRENT_PROTOCOL_VERSION);

    match json_val {
        Value::Array(items) => parse_act_array(&items, CURRENT_PROTOCOL_VERSION, false, opts),
        Value::Object(obj) => {
            if obj.contains_key("acts") {
                for key in obj.keys() {
                    if !matches!(key.as_str(), "acts" | "protocol_version") {
                        return Err(FailedWake::new(
                            key.as_str(),
                            format!("unknown top-level field '{key}'"),
                        ));
                    }
                }
                let protocol_version = match obj.get("protocol_version") {
                    Some(v) => {
                        let pv: ProtocolVersion =
                            serde_json::from_value(v.clone()).map_err(|err| {
                                FailedWake::new(
                                    "protocol_version",
                                    format!("invalid protocol_version: {err}"),
                                )
                            })?;
                        check_version(pv, supported)
                            .map_err(|err| FailedWake::new("protocol_version", err.to_string()))?;
                        pv
                    }
                    None => CURRENT_PROTOCOL_VERSION,
                };
                let acts_val = obj
                    .get("acts")
                    .and_then(Value::as_array)
                    .ok_or_else(|| FailedWake::new("acts", "field 'acts' must be an array"))?;
                parse_act_array(acts_val, protocol_version, false, opts)
            } else if obj.contains_key("act") {
                let items = vec![Value::Object(obj)];
                parse_act_array(&items, CURRENT_PROTOCOL_VERSION, false, opts)
            } else {
                Err(FailedWake::new(
                    "acts",
                    "missing required top-level field 'acts'",
                ))
            }
        }
        _ => Err(FailedWake::new(
            "json",
            "top-level JSON must be a ThoughtBatch object or act array",
        )),
    }
}

fn parse_act_array(
    items: &[Value],
    protocol_version: ProtocolVersion,
    fallback_plain_text: bool,
    opts: ParseOptions,
) -> Result<ParsedModelActs, FailedWake> {
    let mut acts = Vec::with_capacity(items.len());
    let mut rubrics = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let (act, rubric) = parse_single_act(idx, item, opts)?;
        acts.push(act);
        rubrics.push(rubric);
    }
    Ok(ParsedModelActs {
        protocol_version,
        acts,
        rubrics,
        fallback_plain_text,
    })
}

fn parse_single_act(
    idx: usize,
    val: &Value,
    opts: ParseOptions,
) -> Result<(ThoughtAct, Option<DecodedRubric>), FailedWake> {
    let prefix = format!("acts[{idx}]");
    let obj = val.as_object().ok_or_else(|| {
        FailedWake::new(&prefix, "each act entry must be a JSON object").with_act(idx, None)
    })?;

    let act_kind_val = obj.get("act").ok_or_else(|| {
        FailedWake::new(format!("{prefix}.act"), "missing required field 'act'").with_act(idx, None)
    })?;
    let act_kind = act_kind_val.as_str().ok_or_else(|| {
        FailedWake::new(format!("{prefix}.act"), "field 'act' must be a string").with_act(idx, None)
    })?;

    let (allowed, required, optional): (&[&str], &[&str], &[&str]) = match act_kind {
        "think" => (
            &[
                "act",
                "thought",
                "motivation",
                "confidence_0_20",
                "used_ids",
                "importance",
                "basis",
                "value",
                "cost",
                "domain",
            ],
            &[
                "act",
                "thought",
                "motivation",
                "confidence_0_20",
                "used_ids",
            ],
            &["importance", "basis", "value", "cost", "domain"],
        ),
        "act" => (
            &[
                "act",
                "capability_id",
                "places",
                "people",
                "payload",
                "motivation",
                "confidence_0_20",
                "used_ids",
                "expected",
                "domain",
            ],
            &[
                "act",
                "capability_id",
                "places",
                "people",
                "payload",
                "motivation",
                "confidence_0_20",
                "used_ids",
            ],
            &["expected", "domain"],
        ),
        "expect" => (
            &[
                "act",
                "expect_kind",
                "about_ids",
                "places",
                "people",
                "condition",
                "due_ms",
                "expiry_ms",
                "recurrence",
                "promise_meta",
                "domain",
            ],
            &[
                "act",
                "expect_kind",
                "about_ids",
                "places",
                "people",
                "condition",
                "expiry_ms",
            ],
            &["due_ms", "recurrence", "promise_meta", "domain"],
        ),
        "resolve" => (
            &[
                "act",
                "expectation_id",
                "outcome",
                "note",
                "places",
                "people",
                "domain",
            ],
            &["act", "expectation_id", "outcome", "places", "people"],
            &["note", "domain"],
        ),
        "revise" => (
            &[
                "act",
                "target_id",
                "new_version",
                "condition",
                "due_ms",
                "expiry_ms",
                "places",
                "people",
                "domain",
            ],
            &["act", "target_id", "new_version", "places", "people"],
            &["condition", "due_ms", "expiry_ms", "domain"],
        ),
        "consolidate" => (&["act", "output"], &["act", "output"], &[]),
        other => {
            return Err(FailedWake::new(
                format!("{prefix}.act"),
                format!("unknown act kind '{other}'"),
            )
            .with_act(idx, Some(other.to_owned())));
        }
    };

    check_object_fields(
        obj,
        &prefix,
        allowed,
        required,
        optional,
        opts.require_explicit_nulls,
        Some((idx, act_kind)),
    )?;

    // Validate field types and semantic invariants with field-level diagnostics.
    match act_kind {
        "think" => {
            require_string(obj, &prefix, "thought", idx, act_kind)?;
            let rubric = check_rubric_fields(obj, &prefix, idx, act_kind)?;
            check_u64_array(obj, &prefix, "used_ids", idx, act_kind)?;
            check_opt_finite_f32(obj, &prefix, "importance", idx, act_kind)?;
            check_opt_string(obj, &prefix, "basis", idx, act_kind)?;
            check_opt_finite_f32(obj, &prefix, "value", idx, act_kind)?;
            check_opt_finite_f32(obj, &prefix, "cost", idx, act_kind)?;
            check_opt_string(obj, &prefix, "domain", idx, act_kind)?;
            let act: ThoughtAct = serde_json::from_value(val.clone()).map_err(|err| {
                FailedWake::new(&prefix, format!("failed to decode think act: {err}"))
                    .with_act(idx, Some(act_kind.to_owned()))
            })?;
            act.validate().map_err(|err| {
                FailedWake::new(format!("{prefix}.act"), err.to_string())
                    .with_act(idx, Some(act_kind.to_owned()))
            })?;
            Ok((act, Some(rubric)))
        }
        "act" => {
            let cap = require_string(obj, &prefix, "capability_id", idx, act_kind)?;
            if cap.trim().is_empty() {
                return Err(FailedWake::new(
                    format!("{prefix}.capability_id"),
                    "capability_id must not be empty",
                )
                .with_act(idx, Some(act_kind.to_owned())));
            }
            let places = check_string_array(obj, &prefix, "places", idx, act_kind)?;
            if places.is_empty() {
                return Err(FailedWake::new(
                    format!("{prefix}.places"),
                    "act must specify at least one place",
                )
                .with_act(idx, Some(act_kind.to_owned())));
            }
            check_string_array(obj, &prefix, "people", idx, act_kind)?;
            check_u8_array(obj, &prefix, "payload", idx, act_kind)?;
            let rubric = check_rubric_fields(obj, &prefix, idx, act_kind)?;
            check_u64_array(obj, &prefix, "used_ids", idx, act_kind)?;
            if let Some(exp) = obj.get("expected").filter(|v| !v.is_null()) {
                validate_typed_subfield::<monoagens_protocol::SensedEffect>(
                    exp,
                    &format!("{prefix}.expected"),
                    idx,
                    act_kind,
                )?;
            }
            check_opt_string(obj, &prefix, "domain", idx, act_kind)?;
            let act: ThoughtAct = serde_json::from_value(val.clone()).map_err(|err| {
                FailedWake::new(&prefix, format!("failed to decode act: {err}"))
                    .with_act(idx, Some(act_kind.to_owned()))
            })?;
            act.validate().map_err(|err| {
                FailedWake::new(format!("{prefix}.act"), err.to_string())
                    .with_act(idx, Some(act_kind.to_owned()))
            })?;
            Ok((act, Some(rubric)))
        }
        "expect" => {
            validate_typed_subfield::<monoagens_protocol::ExpectKind>(
                &obj["expect_kind"],
                &format!("{prefix}.expect_kind"),
                idx,
                act_kind,
            )?;
            check_u64_array(obj, &prefix, "about_ids", idx, act_kind)?;
            check_string_array(obj, &prefix, "places", idx, act_kind)?;
            check_string_array(obj, &prefix, "people", idx, act_kind)?;
            require_string(obj, &prefix, "condition", idx, act_kind)?;
            check_opt_u64(obj, &prefix, "due_ms", idx, act_kind)?;
            require_u64(obj, &prefix, "expiry_ms", idx, act_kind)?;
            if let Some(rec) = obj.get("recurrence").filter(|v| !v.is_null()) {
                validate_typed_subfield::<monoagens_protocol::RecurrenceSpec>(
                    rec,
                    &format!("{prefix}.recurrence"),
                    idx,
                    act_kind,
                )?;
            }
            if let Some(pm) = obj.get("promise_meta").filter(|v| !v.is_null()) {
                validate_typed_subfield::<monoagens_protocol::PromiseMeta>(
                    pm,
                    &format!("{prefix}.promise_meta"),
                    idx,
                    act_kind,
                )?;
            }
            check_opt_string(obj, &prefix, "domain", idx, act_kind)?;
            let act: ThoughtAct = serde_json::from_value(val.clone()).map_err(|err| {
                FailedWake::new(&prefix, format!("failed to decode expect act: {err}"))
                    .with_act(idx, Some(act_kind.to_owned()))
            })?;
            Ok((act, None))
        }
        "resolve" => {
            require_u64(obj, &prefix, "expectation_id", idx, act_kind)?;
            validate_typed_subfield::<monoagens_protocol::ResolveOutcome>(
                &obj["outcome"],
                &format!("{prefix}.outcome"),
                idx,
                act_kind,
            )?;
            check_opt_string(obj, &prefix, "note", idx, act_kind)?;
            check_string_array(obj, &prefix, "places", idx, act_kind)?;
            check_string_array(obj, &prefix, "people", idx, act_kind)?;
            check_opt_string(obj, &prefix, "domain", idx, act_kind)?;
            let act: ThoughtAct = serde_json::from_value(val.clone()).map_err(|err| {
                FailedWake::new(&prefix, format!("failed to decode resolve act: {err}"))
                    .with_act(idx, Some(act_kind.to_owned()))
            })?;
            Ok((act, None))
        }
        "revise" => {
            require_u64(obj, &prefix, "target_id", idx, act_kind)?;
            let new_ver = require_u64(obj, &prefix, "new_version", idx, act_kind)?;
            if u32::try_from(new_ver).is_err() {
                return Err(FailedWake::new(
                    format!("{prefix}.new_version"),
                    format!("new_version {new_ver} exceeds u32::MAX"),
                )
                .with_act(idx, Some(act_kind.to_owned())));
            }
            check_opt_string(obj, &prefix, "condition", idx, act_kind)?;
            check_opt_u64(obj, &prefix, "due_ms", idx, act_kind)?;
            check_opt_u64(obj, &prefix, "expiry_ms", idx, act_kind)?;
            check_string_array(obj, &prefix, "places", idx, act_kind)?;
            check_string_array(obj, &prefix, "people", idx, act_kind)?;
            check_opt_string(obj, &prefix, "domain", idx, act_kind)?;
            let act: ThoughtAct = serde_json::from_value(val.clone()).map_err(|err| {
                FailedWake::new(&prefix, format!("failed to decode revise act: {err}"))
                    .with_act(idx, Some(act_kind.to_owned()))
            })?;
            Ok((act, None))
        }
        "consolidate" => {
            let output_val = &obj["output"];
            let out_prefix = format!("{prefix}.output");
            validate_consolidate_output(output_val, &out_prefix, idx, opts)?;
            let output: ConsolidateOutput =
                serde_json::from_value(output_val.clone()).map_err(|err| {
                    FailedWake::new(
                        &out_prefix,
                        format!("failed to decode consolidate output: {err}"),
                    )
                    .with_act(idx, Some("consolidate".to_owned()))
                })?;
            Ok((ThoughtAct::Consolidate { output }, None))
        }
        _ => unreachable!(),
    }
}

fn validate_consolidate_output(
    val: &Value,
    prefix: &str,
    idx: usize,
    opts: ParseOptions,
) -> Result<(), FailedWake> {
    let act_kind = "consolidate";
    let obj = val.as_object().ok_or_else(|| {
        FailedWake::new(prefix, "consolidate output must be a JSON object")
            .with_act(idx, Some(act_kind.to_owned()))
    })?;
    let kind = obj
        .get("consolidate_kind")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            FailedWake::new(
                format!("{prefix}.consolidate_kind"),
                "missing or non-string field 'consolidate_kind'",
            )
            .with_act(idx, Some(act_kind.to_owned()))
        })?;

    let (allowed, required, optional): (&[&str], &[&str], &[&str]) = match kind {
        "gist" => (
            &[
                "consolidate_kind",
                "summary",
                "about_ids",
                "places",
                "people",
                "domain",
            ],
            &[
                "consolidate_kind",
                "summary",
                "about_ids",
                "places",
                "people",
            ],
            &["domain"],
        ),
        "lesson" => (
            &[
                "consolidate_kind",
                "pattern",
                "failure_signature",
                "contrastive_success_ids",
                "failure_ids",
                "domain",
            ],
            &[
                "consolidate_kind",
                "pattern",
                "failure_signature",
                "contrastive_success_ids",
                "failure_ids",
            ],
            &["domain"],
        ),
        "procedure" => (
            &[
                "consolidate_kind",
                "name",
                "steps",
                "required_cap_ids",
                "capability_schema_hash",
                "domain",
            ],
            &[
                "consolidate_kind",
                "name",
                "steps",
                "required_cap_ids",
                "capability_schema_hash",
            ],
            &["domain"],
        ),
        other => {
            return Err(FailedWake::new(
                format!("{prefix}.consolidate_kind"),
                format!("unknown consolidate_kind '{other}'"),
            )
            .with_act(idx, Some(act_kind.to_owned())));
        }
    };

    check_object_fields(
        obj,
        prefix,
        allowed,
        required,
        optional,
        opts.require_explicit_nulls,
        Some((idx, act_kind)),
    )?;

    match kind {
        "gist" => {
            require_string(obj, prefix, "summary", idx, act_kind)?;
            check_u64_array(obj, prefix, "about_ids", idx, act_kind)?;
            check_string_array(obj, prefix, "places", idx, act_kind)?;
            check_string_array(obj, prefix, "people", idx, act_kind)?;
            check_opt_string(obj, prefix, "domain", idx, act_kind)?;
        }
        "lesson" => {
            require_string(obj, prefix, "pattern", idx, act_kind)?;
            require_string(obj, prefix, "failure_signature", idx, act_kind)?;
            check_u64_array(obj, prefix, "contrastive_success_ids", idx, act_kind)?;
            check_u64_array(obj, prefix, "failure_ids", idx, act_kind)?;
            check_opt_string(obj, prefix, "domain", idx, act_kind)?;
        }
        "procedure" => {
            require_string(obj, prefix, "name", idx, act_kind)?;
            check_string_array(obj, prefix, "steps", idx, act_kind)?;
            check_string_array(obj, prefix, "required_cap_ids", idx, act_kind)?;
            let hash = require_string(obj, prefix, "capability_schema_hash", idx, act_kind)?;
            if hash.trim().is_empty() {
                return Err(FailedWake::new(
                    format!("{prefix}.capability_schema_hash"),
                    "capability_schema_hash must not be empty",
                )
                .with_act(idx, Some(act_kind.to_owned())));
            }
            check_opt_string(obj, prefix, "domain", idx, act_kind)?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// Parses a `ModelPurpose::Classify` response into [`ClassificationVerdict`].
pub fn parse_classification_verdict(raw: &str) -> Result<ClassificationVerdict, FailedWake> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(FailedWake::new("output", "classification output is empty"));
    }
    let val: Value = serde_json::from_str(trimmed)
        .map_err(|err| FailedWake::new("json", format!("malformed classification JSON: {err}")))?;
    let obj = val
        .as_object()
        .ok_or_else(|| FailedWake::new("json", "classification verdict must be a JSON object"))?;
    check_object_fields(
        obj,
        "$",
        &["protocol_version", "label", "score", "rationale"],
        &["label"],
        &["score", "rationale"],
        false,
        None,
    )?;
    let label = match obj.get("label").and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            return Err(FailedWake::new(
                "label",
                "classification 'label' must be a non-empty string",
            ));
        }
    };
    if let Some(score_val) = obj.get("score").filter(|v| !v.is_null()) {
        let Some(score) = score_val.as_f64() else {
            return Err(FailedWake::new(
                "score",
                "classification 'score' must be a number or null",
            ));
        };
        if !score.is_finite() {
            return Err(FailedWake::new(
                "score",
                "classification 'score' must be finite",
            ));
        }
    }
    if let Some(rat_val) = obj.get("rationale").filter(|v| !v.is_null())
        && !rat_val.is_string()
    {
        return Err(FailedWake::new(
            "rationale",
            "classification 'rationale' must be a string or null",
        ));
    }
    let _ = label;
    let verdict: ClassificationVerdict = serde_json::from_value(val).map_err(|err| {
        FailedWake::new(
            "classification_verdict",
            format!("invalid ClassificationVerdict: {err}"),
        )
    })?;
    check_version(verdict.protocol_version, CURRENT_PROTOCOL_VERSION)
        .map_err(|err| FailedWake::new("protocol_version", err.to_string()))?;
    Ok(verdict)
}

fn check_object_fields(
    obj: &Map<String, Value>,
    prefix: &str,
    allowed: &[&str],
    required: &[&str],
    optional: &[&str],
    require_explicit_nulls: bool,
    act_ctx: Option<(usize, &str)>,
) -> Result<(), FailedWake> {
    let wrap = |fw: FailedWake| match act_ctx {
        Some((idx, kind)) => fw.with_act(idx, Some(kind.to_owned())),
        None => fw,
    };
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(wrap(FailedWake::new(
                format!("{prefix}.{key}"),
                format!("unknown field '{key}'"),
            )));
        }
    }
    for req in required {
        match obj.get(*req) {
            None => {
                return Err(wrap(FailedWake::new(
                    format!("{prefix}.{req}"),
                    format!("missing required field '{req}'"),
                )));
            }
            Some(Value::Null) => {
                return Err(wrap(FailedWake::new(
                    format!("{prefix}.{req}"),
                    format!("non-nullable required field '{req}' cannot be null"),
                )));
            }
            Some(_) => {}
        }
    }
    if require_explicit_nulls {
        for opt in optional {
            if !obj.contains_key(*opt) {
                return Err(wrap(FailedWake::new(
                    format!("{prefix}.{opt}"),
                    format!("strict R | null schema requires explicit key '{opt}' (value or null)"),
                )));
            }
        }
    }
    Ok(())
}

fn check_rubric_fields(
    obj: &Map<String, Value>,
    prefix: &str,
    idx: usize,
    act_kind: &str,
) -> Result<DecodedRubric, FailedWake> {
    let mot_u64 = require_u64(obj, prefix, "motivation", idx, act_kind)?;
    let mot = u8::try_from(mot_u64).map_err(|_| {
        FailedWake::new(
            format!("{prefix}.motivation"),
            format!("motivation {mot_u64} must be in 1..=5"),
        )
        .with_act(idx, Some(act_kind.to_owned()))
    })?;
    let conf_u64 = require_u64(obj, prefix, "confidence_0_20", idx, act_kind)?;
    let conf = u8::try_from(conf_u64).map_err(|_| {
        FailedWake::new(
            format!("{prefix}.confidence_0_20"),
            format!("confidence_0_20 {conf_u64} must be in 0..=20"),
        )
        .with_act(idx, Some(act_kind.to_owned()))
    })?;
    decode_rubric(mot, conf).map_err(|err| {
        FailedWake::new(format!("{prefix}.{}", err.field), err.reason)
            .with_act(idx, Some(act_kind.to_owned()))
    })
}

fn require_string<'a>(
    obj: &'a Map<String, Value>,
    prefix: &str,
    field: &str,
    idx: usize,
    act_kind: &str,
) -> Result<&'a str, FailedWake> {
    obj.get(field).and_then(Value::as_str).ok_or_else(|| {
        FailedWake::new(
            format!("{prefix}.{field}"),
            format!("field '{field}' must be a string"),
        )
        .with_act(idx, Some(act_kind.to_owned()))
    })
}

fn check_opt_string(
    obj: &Map<String, Value>,
    prefix: &str,
    field: &str,
    idx: usize,
    act_kind: &str,
) -> Result<(), FailedWake> {
    if let Some(v) = obj.get(field)
        && !v.is_null()
        && !v.is_string()
    {
        return Err(FailedWake::new(
            format!("{prefix}.{field}"),
            format!("field '{field}' must be a string or null"),
        )
        .with_act(idx, Some(act_kind.to_owned())));
    }
    Ok(())
}

fn require_u64(
    obj: &Map<String, Value>,
    prefix: &str,
    field: &str,
    idx: usize,
    act_kind: &str,
) -> Result<u64, FailedWake> {
    obj.get(field).and_then(Value::as_u64).ok_or_else(|| {
        FailedWake::new(
            format!("{prefix}.{field}"),
            format!("field '{field}' must be a non-negative integer"),
        )
        .with_act(idx, Some(act_kind.to_owned()))
    })
}

fn check_opt_u64(
    obj: &Map<String, Value>,
    prefix: &str,
    field: &str,
    idx: usize,
    act_kind: &str,
) -> Result<(), FailedWake> {
    if let Some(v) = obj.get(field)
        && !v.is_null()
        && v.as_u64().is_none()
    {
        return Err(FailedWake::new(
            format!("{prefix}.{field}"),
            format!("field '{field}' must be a non-negative integer or null"),
        )
        .with_act(idx, Some(act_kind.to_owned())));
    }
    Ok(())
}

fn check_opt_finite_f32(
    obj: &Map<String, Value>,
    prefix: &str,
    field: &str,
    idx: usize,
    act_kind: &str,
) -> Result<(), FailedWake> {
    if let Some(v) = obj.get(field)
        && !v.is_null()
    {
        let Some(num) = v.as_f64() else {
            return Err(FailedWake::new(
                format!("{prefix}.{field}"),
                format!("field '{field}' must be a finite number or null"),
            )
            .with_act(idx, Some(act_kind.to_owned())));
        };
        if !num.is_finite() || !(num as f32).is_finite() {
            return Err(FailedWake::new(
                format!("{prefix}.{field}"),
                format!("field '{field}' must be finite"),
            )
            .with_act(idx, Some(act_kind.to_owned())));
        }
    }
    Ok(())
}

fn check_u64_array(
    obj: &Map<String, Value>,
    prefix: &str,
    field: &str,
    idx: usize,
    act_kind: &str,
) -> Result<(), FailedWake> {
    let arr = obj.get(field).and_then(Value::as_array).ok_or_else(|| {
        FailedWake::new(
            format!("{prefix}.{field}"),
            format!("field '{field}' must be an array of integer IDs"),
        )
        .with_act(idx, Some(act_kind.to_owned()))
    })?;
    for (elem_idx, elem) in arr.iter().enumerate() {
        if elem.as_u64().is_none() {
            return Err(FailedWake::new(
                format!("{prefix}.{field}[{elem_idx}]"),
                format!("element {elem_idx} of '{field}' must be a non-negative integer"),
            )
            .with_act(idx, Some(act_kind.to_owned())));
        }
    }
    Ok(())
}

fn check_u8_array(
    obj: &Map<String, Value>,
    prefix: &str,
    field: &str,
    idx: usize,
    act_kind: &str,
) -> Result<(), FailedWake> {
    let arr = obj.get(field).and_then(Value::as_array).ok_or_else(|| {
        FailedWake::new(
            format!("{prefix}.{field}"),
            format!("field '{field}' must be a byte array"),
        )
        .with_act(idx, Some(act_kind.to_owned()))
    })?;
    for (elem_idx, elem) in arr.iter().enumerate() {
        let Some(n) = elem.as_u64() else {
            return Err(FailedWake::new(
                format!("{prefix}.{field}[{elem_idx}]"),
                format!("byte {elem_idx} of '{field}' must be an integer in 0..=255"),
            )
            .with_act(idx, Some(act_kind.to_owned())));
        };
        if n > 255 {
            return Err(FailedWake::new(
                format!("{prefix}.{field}[{elem_idx}]"),
                format!("byte {elem_idx} of '{field}' exceeds 255"),
            )
            .with_act(idx, Some(act_kind.to_owned())));
        }
    }
    Ok(())
}

fn check_string_array<'a>(
    obj: &'a Map<String, Value>,
    prefix: &str,
    field: &str,
    idx: usize,
    act_kind: &str,
) -> Result<Vec<&'a str>, FailedWake> {
    let arr = obj.get(field).and_then(Value::as_array).ok_or_else(|| {
        FailedWake::new(
            format!("{prefix}.{field}"),
            format!("field '{field}' must be an array of strings"),
        )
        .with_act(idx, Some(act_kind.to_owned()))
    })?;
    let mut out = Vec::with_capacity(arr.len());
    for (elem_idx, elem) in arr.iter().enumerate() {
        let Some(s) = elem.as_str() else {
            return Err(FailedWake::new(
                format!("{prefix}.{field}[{elem_idx}]"),
                format!("element {elem_idx} of '{field}' must be a string"),
            )
            .with_act(idx, Some(act_kind.to_owned())));
        };
        out.push(s);
    }
    Ok(out)
}

fn validate_typed_subfield<T: serde::de::DeserializeOwned>(
    val: &Value,
    field_path: &str,
    idx: usize,
    act_kind: &str,
) -> Result<(), FailedWake> {
    serde_json::from_value::<T>(val.clone())
        .map(|_| ())
        .map_err(|err| {
            FailedWake::new(field_path, format!("invalid '{field_path}': {err}"))
                .with_act(idx, Some(act_kind.to_owned()))
        })
}

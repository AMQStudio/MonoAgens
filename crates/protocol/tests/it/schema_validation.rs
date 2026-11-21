#![cfg(feature = "schema")]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use monoagens_protocol::schema::{
    JSON_SCHEMA_DRAFT_2020_12, OPENRPC_VERSION, write_generated_artifacts,
};
use monoagens_protocol::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult, ActionId,
    ActionReceiptParams, ActionReceiptResult, ActorId, ApprovalResolveParams,
    ApprovalResolveResult, Audience, BlobGetParams, BlobGetResult, BlobPutParams, BlobPutResult,
    BodyDiscoverParams, BodyDiscoverResult, BodyHeartbeatParams, BodyHeartbeatResult, BodyId,
    BodyShutdownParams, BodyShutdownResult, BodyStatus, CapabilityId, HAct, MetaValue,
    OpaquePayload, Percept, PerceptEmitParams, PerceptEmitResult, PlaceId, ProtocolVersion,
    QuestionAnswerParams, QuestionAnswerResult, Receipt, ReceiptOutcome, StateQueryParams,
    StateQueryResult, StateQueryStatus, TerminationReason, Time, Waist2Headers,
};
use serde::Serialize;
use serde_json::{Value, json};

const EXPECTED_METHODS: [&str; 12] = [
    "body/discover",
    "body/heartbeat",
    "body/shutdown",
    "percept/emit",
    "action/execute",
    "action/cancel",
    "action/receipt",
    "approval/resolve",
    "question/answer",
    "state/query",
    "blob/put",
    "blob/get",
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_schema(name: &str) -> Value {
    let path = manifest_dir()
        .join("jsonschema")
        .join(format!("{name}.schema.json"));
    let text = fs::read_to_string(&path).expect("read schema file");
    serde_json::from_str(&text).expect("parse schema json")
}

fn collect_relative_files(base: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    for subdir in ["openrpc", "jsonschema"] {
        let dir = base.join(subdir);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_file() {
                let rel = path
                    .strip_prefix(base)
                    .expect("relative path")
                    .to_path_buf();
                let bytes = fs::read(&path).expect("read artifact");
                files.insert(rel, bytes);
            }
        }
    }
    files
}

#[test]
fn regenerated_artifacts_match_committed_files_byte_for_byte() {
    let committed_root = manifest_dir();
    let temp_root = std::env::temp_dir().join(format!(
        "monoagens-openrpc-drift-check-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&temp_root);
    write_generated_artifacts(&temp_root).expect("regenerate artifacts");

    let expected = collect_relative_files(&temp_root);
    let actual = collect_relative_files(&committed_root);
    let _ = fs::remove_dir_all(&temp_root);

    let expected_keys: BTreeSet<_> = expected.keys().cloned().collect();
    let actual_keys: BTreeSet<_> = actual.keys().cloned().collect();
    assert_eq!(
        actual_keys, expected_keys,
        "generated artifact file set drifted"
    );

    for (rel, expected_bytes) in &expected {
        let actual_bytes = &actual[rel];
        assert_eq!(
            actual_bytes,
            expected_bytes,
            "byte drift detected in {}",
            rel.display()
        );
    }
}

#[test]
fn all_json_schemas_validate_against_draft_2020_12_metaschema() {
    let dir = manifest_dir().join("jsonschema");
    let mut count = 0usize;
    for entry in fs::read_dir(&dir).expect("read jsonschema dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path).expect("read schema");
        let schema: Value = serde_json::from_str(&raw).expect("parse schema");
        assert_eq!(
            schema.get("$schema").and_then(Value::as_str),
            Some(JSON_SCHEMA_DRAFT_2020_12),
            "{} must declare Draft 2020-12 $schema",
            path.display()
        );
        jsonschema::draft202012::meta::validate(&schema).unwrap_or_else(|err| {
            panic!(
                "{} failed Draft 2020-12 metaschema validation: {err}",
                path.display()
            )
        });
        jsonschema::validator_for(&schema)
            .unwrap_or_else(|err| panic!("{} failed to compile validator: {err}", path.display()));
        count += 1;
    }
    assert!(count >= 24, "expected all domain and envelope schemas");
}

#[test]
fn openrpc_document_contains_all_twelve_methods_and_six_errors() {
    let path = manifest_dir()
        .join("openrpc")
        .join("monoagens.body.v1.openrpc.json");
    let raw = fs::read_to_string(&path).expect("read openrpc doc");
    let doc: Value = serde_json::from_str(&raw).expect("parse openrpc doc");

    assert_eq!(
        doc.get("openrpc").and_then(Value::as_str),
        Some(OPENRPC_VERSION)
    );
    assert_eq!(
        doc.pointer("/info/title").and_then(Value::as_str),
        Some("monoagens.body.v1")
    );
    assert!(
        doc.pointer("/info/version")
            .and_then(Value::as_str)
            .is_some_and(|v| !v.is_empty())
    );

    let methods = doc
        .get("methods")
        .and_then(Value::as_array)
        .expect("methods array");
    assert_eq!(methods.len(), 12);

    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for m in methods {
        let name = m.get("name").and_then(Value::as_str).expect("method name");
        *counts.entry(name).or_insert(0) += 1;
        assert!(
            m.get("params").and_then(Value::as_array).is_some(),
            "method {name} missing params array"
        );
        assert!(
            m.pointer("/result/schema/$ref")
                .and_then(Value::as_str)
                .is_some_and(|r| r.starts_with("#/components/schemas/")),
            "method {name} result must reference components/schemas"
        );
    }
    for expected in EXPECTED_METHODS {
        assert_eq!(
            counts.get(expected).copied(),
            Some(1),
            "method {expected} must appear exactly once"
        );
    }

    let errors = doc
        .pointer("/components/errors")
        .and_then(Value::as_object)
        .expect("components.errors");
    let mut codes = BTreeSet::new();
    for err_obj in errors.values() {
        let code = err_obj.get("code").and_then(Value::as_i64).expect("code");
        codes.insert(code);
    }
    for expected_code in -32006..=-32001 {
        assert!(
            codes.contains(&i64::from(expected_code)),
            "missing error code {expected_code}"
        );
    }

    // Every component schema inside OpenRPC must also validate under Draft 2020-12.
    let schemas = doc
        .pointer("/components/schemas")
        .and_then(Value::as_object)
        .expect("components.schemas");
    for (name, schema) in schemas {
        let wrapped = json!({
            "$schema": JSON_SCHEMA_DRAFT_2020_12,
            "components": { "schemas": Value::Object(schemas.clone()) },
            "$ref": format!("#/components/schemas/{name}")
        });
        jsonschema::draft202012::meta::validate(schema).unwrap_or_else(|err| {
            panic!("OpenRPC component schema {name} invalid under Draft 2020-12: {err}")
        });
        jsonschema::validator_for(&wrapped).unwrap_or_else(|err| {
            panic!("OpenRPC component schema {name} failed validator compile: {err}")
        });
    }
}

#[test]
fn schema_rejects_missing_audience_and_newer_version_and_accepts_unknown_meta() {
    let percept_emit_validator =
        jsonschema::validator_for(&load_schema("PerceptEmitParams")).expect("compile");
    let percept_validator = jsonschema::validator_for(&load_schema("Percept")).expect("compile");
    let headers_validator =
        jsonschema::validator_for(&load_schema("Waist2Headers")).expect("compile");

    // 1. Valid payload with unknown reverse-DNS _meta keys passes.
    let valid_emit = json!({
        "headers": {
            "idempotency_key": "idem-meta-1",
            "protocol_version": { "major": 1, "minor": 0 },
            "_meta": {
                "com.example.vendor": { "feature": true, "count": 2 },
                "org.monoagens.ext": "ok"
            },
            "future_additive_header": 42
        },
        "actor": "per_alice",
        "place": "cli:0",
        "audience": ["per_alice"],
        "timestamp_ms": 1_700_000_000_000u64,
        "payload": [104, 105]
    });
    assert!(percept_emit_validator.validate(&valid_emit).is_ok());
    assert!(headers_validator.validate(&valid_emit["headers"]).is_ok());

    // 2. Payload missing `audience` fails validation (both PerceptEmitParams and Percept).
    let missing_aud_emit = json!({
        "headers": {
            "idempotency_key": "idem-no-aud",
            "protocol_version": { "major": 1, "minor": 0 }
        },
        "actor": "per_alice",
        "place": "cli:0",
        "timestamp_ms": 1_700_000_000_000u64,
        "payload": [104, 105]
    });
    assert!(percept_emit_validator.validate(&missing_aud_emit).is_err());

    let missing_aud_percept = json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "actor": "per_alice",
        "place": "cli:0",
        "timestamp_ms": 1_700_000_000_000u64,
        "payload": [104, 105]
    });
    assert!(percept_validator.validate(&missing_aud_percept).is_err());

    let empty_aud_emit = json!({
        "headers": {
            "idempotency_key": "idem-empty-aud",
            "protocol_version": { "major": 1, "minor": 0 }
        },
        "actor": "per_alice",
        "place": "cli:0",
        "audience": [],
        "timestamp_ms": 1_700_000_000_000u64,
        "payload": [104, 105]
    });
    assert!(percept_emit_validator.validate(&empty_aud_emit).is_err());

    // 3. Newer protocol_version (both newer minor 1.1 and newer major 2.0) fails validation.
    let newer_minor = json!({
        "headers": {
            "idempotency_key": "idem-v11",
            "protocol_version": { "major": 1, "minor": 1 }
        },
        "actor": "per_alice",
        "place": "cli:0",
        "audience": ["per_alice"],
        "timestamp_ms": 1_700_000_000_000u64,
        "payload": [104, 105]
    });
    assert!(percept_emit_validator.validate(&newer_minor).is_err());
    assert!(headers_validator.validate(&newer_minor["headers"]).is_err());

    let newer_major = json!({
        "idempotency_key": "idem-v20",
        "protocol_version": { "major": 2, "minor": 0 }
    });
    assert!(headers_validator.validate(&newer_major).is_err());
}

fn assert_validates_against<T: Serialize>(schema_name: &str, value: &T) {
    let validator = jsonschema::validator_for(&load_schema(schema_name)).expect("compile");
    let json_val = serde_json::to_value(value).expect("serialize");
    validator
        .validate(&json_val)
        .unwrap_or_else(|err| panic!("{schema_name} rejected valid Rust value: {err}"));
}

#[test]
fn all_twelve_method_envelopes_validate_against_their_generated_schemas() {
    let mut headers = Waist2Headers::new("idem-all-12");
    headers
        .meta
        .insert("org.monoagens.test".to_owned(), MetaValue::Bool(true));

    let actor = ActorId::new("per_alice");
    let place = PlaceId::new("cli:0");
    let audience = Audience::new([actor.clone()]);
    let action_id = ActionId::new("act-1");
    let cap_id = CapabilityId::new("chat.speak");
    let body_id = BodyId::new("body-cli");
    let receipt = Receipt {
        action_id: action_id.clone(),
        capability_id: cap_id.clone(),
        outcome: ReceiptOutcome::Done,
        reason_code: None,
        detail: None,
        termination_reason: Some(TerminationReason::Normal),
        progress: Some(1.0),
        delivered_units: Some(1),
        h_act: None,
        timestamp_ms: Time::from_millis(1_700_000_000_500),
    };

    assert_validates_against(
        "BodyDiscoverParams",
        &BodyDiscoverParams {
            headers: headers.clone(),
            body_id: Some(body_id.clone()),
        },
    );
    assert_validates_against(
        "BodyDiscoverResult",
        &BodyDiscoverResult {
            body_id: body_id.clone(),
            protocol_version: ProtocolVersion::new(1, 0),
            capabilities: Vec::new(),
            places: vec![place.clone()],
        },
    );
    assert_validates_against(
        "BodyHeartbeatParams",
        &BodyHeartbeatParams {
            headers: headers.clone(),
            body_id: body_id.clone(),
            status: BodyStatus::Ready,
            timestamp_ms: Time::from_millis(100),
        },
    );
    assert_validates_against(
        "BodyHeartbeatResult",
        &BodyHeartbeatResult {
            acknowledged_at_ms: Time::from_millis(101),
        },
    );
    assert_validates_against(
        "BodyShutdownParams",
        &BodyShutdownParams {
            headers: headers.clone(),
            body_id,
            reason: Some("normal".to_owned()),
        },
    );
    assert_validates_against("BodyShutdownResult", &BodyShutdownResult { accepted: true });
    assert_validates_against(
        "PerceptEmitParams",
        &PerceptEmitParams::from_percept(
            headers.clone(),
            Percept::minimal(
                actor.clone(),
                place.clone(),
                audience.clone(),
                Time::from_millis(200),
                OpaquePayload::from_bytes(b"hi".as_slice()),
            ),
        ),
    );
    assert_validates_against("PerceptEmitResult", &PerceptEmitResult { accepted: true });
    assert_validates_against(
        "ActionExecuteParams",
        &ActionExecuteParams {
            headers: headers.clone(),
            action_id: action_id.clone(),
            capability_id: cap_id,
            places: vec![place.clone()],
            audience: audience.clone(),
            payload: OpaquePayload::from_bytes(b"say".as_slice()),
            timeout_ms: Some(5_000),
            h_act: Some(HAct::new("blake3:01")),
        },
    );
    assert_validates_against(
        "ActionExecuteResult",
        &ActionExecuteResult {
            accepted: true,
            receipt: Some(receipt.clone()),
        },
    );
    assert_validates_against(
        "ActionCancelParams",
        &ActionCancelParams {
            headers: headers.clone(),
            action_id: action_id.clone(),
            reason: Some("barge_in".to_owned()),
        },
    );
    assert_validates_against(
        "ActionCancelResult",
        &ActionCancelResult {
            cancelled: true,
            receipt: Some(receipt.clone()),
        },
    );
    assert_validates_against(
        "ActionReceiptParams",
        &ActionReceiptParams {
            headers: headers.clone(),
            receipt: receipt.clone(),
        },
    );
    assert_validates_against(
        "ActionReceiptResult",
        &ActionReceiptResult { recorded: true },
    );
    assert_validates_against(
        "ApprovalResolveParams",
        &ApprovalResolveParams {
            headers: headers.clone(),
            action_id: action_id.clone(),
            h_act: HAct::new("blake3:01"),
            state_epoch: 1,
            approved: true,
            resolver: actor.clone(),
            resolved_at_ms: Time::from_millis(300),
            signature: None,
            out_of_band_channel: false,
        },
    );
    assert_validates_against(
        "ApprovalResolveResult",
        &ApprovalResolveResult { applied: true },
    );
    assert_validates_against(
        "QuestionAnswerParams",
        &QuestionAnswerParams {
            headers: headers.clone(),
            question_id: "q-1".to_owned(),
            actor,
            place,
            audience,
            answer: OpaquePayload::from_bytes(b"yes".as_slice()),
            timestamp_ms: Time::from_millis(400),
        },
    );
    assert_validates_against(
        "QuestionAnswerResult",
        &QuestionAnswerResult { accepted: true },
    );
    assert_validates_against(
        "StateQueryParams",
        &StateQueryParams {
            headers: headers.clone(),
            action_id: action_id.clone(),
        },
    );
    assert_validates_against(
        "StateQueryResult",
        &StateQueryResult {
            action_id,
            status: StateQueryStatus::Terminal { receipt },
        },
    );
    assert_validates_against(
        "BlobPutParams",
        &BlobPutParams {
            headers: headers.clone(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned(),
            byte_len: 0,
            inline_bytes: Some(OpaquePayload::default()),
        },
    );
    assert_validates_against(
        "BlobPutResult",
        &BlobPutResult {
            blob_ref: "blob://1".to_owned(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned(),
            expires_at_ms: Time::from_millis(60_000),
        },
    );
    assert_validates_against(
        "BlobGetParams",
        &BlobGetParams {
            headers,
            blob_ref: "blob://1".to_owned(),
            expected_sha256: None,
        },
    );
    assert_validates_against(
        "BlobGetResult",
        &BlobGetResult {
            blob_ref: "blob://1".to_owned(),
            sha256: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".to_owned(),
            byte_len: 3,
            bytes: Some(OpaquePayload::from_bytes(b"xyz".as_slice())),
        },
    );
}

//! OpenRPC 1.4.1 and JSON Schema Draft 2020-12 generation for Waist 1 & Waist 2 contracts.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use schemars::generate::{SchemaGenerator, SchemaSettings};
use schemars::{JsonSchema, Schema};
use serde_json::{Map, Value, json};

use crate::facts::{
    ActionCanonicalInput, Audience, CapabilityDescriptor, CapabilityFlags, CatchupPolicy,
    DeclassToken, DomainFactKind, HAct, OriginActor, PendingApproval, PrincipalLink,
    PrincipalLinkStatus, PromiseMeta, Receipt, ReceiptOutcome, RecurrenceSchedule, RecurrenceSpec,
    SensedEffect, Taint, TerminationReason, VisibleScope,
};
use crate::ids::{
    ActionId, ActorId, BodyId, CapabilityId, CapabilitySchemaHash, ChannelId, CorrelationId,
    DomainId, EventId, PlaceId, Time,
};
use crate::message::{
    Action, ActorFacts, AttachmentHandle, BodyState, BodyStatus, ChannelPrediction, EmbeddingTag,
    FModal, ModalitySlot, OpaquePayload, OrientScore, Percept, TaggedEmbedding,
};
use crate::thought::{
    ClassificationVerdict, ConsolidateOutput, ExpectKind, ResolveOutcome, ThoughtAct, ThoughtBatch,
};
use crate::version::{CURRENT_PROTOCOL_VERSION, ProtocolVersion};
use crate::waist2::{
    ActionCancelParams, ActionCancelResult, ActionExecuteParams, ActionExecuteResult,
    ActionReceiptParams, ActionReceiptResult, ApprovalResolveParams, ApprovalResolveResult,
    BlobGetParams, BlobGetResult, BlobPutParams, BlobPutResult, BodyDiscoverParams,
    BodyDiscoverResult, BodyHeartbeatParams, BodyHeartbeatResult, BodyShutdownParams,
    BodyShutdownResult, JSONRPC_VERSION, MetaValue, PerceptEmitParams, PerceptEmitResult,
    QuestionAnswerParams, QuestionAnswerResult, RpcId, StateQueryParams, StateQueryResult,
    StateQueryStatus, Waist2Call, Waist2Error, Waist2ErrorCode, Waist2ErrorData, Waist2Headers,
    Waist2Method, Waist2Request, Waist2Response,
};

/// Canonical JSON Schema Draft 2020-12 metaschema URI.
pub const JSON_SCHEMA_DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// Canonical OpenRPC specification version emitted by `monoagens-openrpc-gen`.
pub const OPENRPC_VERSION: &str = "1.4.1";

/// Protocol version bounds transform: enforces `(M_msg == M_gw) && (m_msg <= m_gw)`.
pub(crate) fn transform_protocol_version(schema: &mut Schema) {
    let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    if let Some(major) = props.get_mut("major").and_then(Value::as_object_mut) {
        major.insert("minimum".to_owned(), json!(CURRENT_PROTOCOL_VERSION.major));
        major.insert("maximum".to_owned(), json!(CURRENT_PROTOCOL_VERSION.major));
    }
    if let Some(minor) = props.get_mut("minor").and_then(Value::as_object_mut) {
        minor.insert("minimum".to_owned(), json!(0));
        minor.insert("maximum".to_owned(), json!(CURRENT_PROTOCOL_VERSION.minor));
    }
}

/// Ensures `jsonrpc` envelope field is fixed to `"2.0"`.
pub(crate) fn transform_jsonrpc_const(schema: &mut Schema) {
    if let Some(obj) = schema.as_object_mut() {
        obj.insert("const".to_owned(), json!(JSONRPC_VERSION));
    }
}

/// Ensures `Audience` fields on emitted messages require at least one actor.
pub(crate) fn transform_non_empty_array(schema: &mut Schema) {
    if let Some(obj) = schema.as_object_mut() {
        obj.insert("minItems".to_owned(), json!(1));
    }
}

/// Marks `audience` required and non-empty in the wire JSON Schema for `percept/emit`.
///
/// The Rust struct retains `Option<Audience>` so serde deserialization can route
/// missing audiences to `-32006 ERR_UNDECLARED_AUDIENCE`.
pub(crate) fn transform_percept_emit_params(schema: &mut Schema) {
    if let Some(props) = schema.get_mut("properties").and_then(Value::as_object_mut)
        && let Some(aud) = props.get_mut("audience")
    {
        if let Some(ref_val) = aud
            .get("anyOf")
            .and_then(Value::as_array)
            .and_then(|arr| arr.iter().find(|v| v.get("$ref").is_some()).cloned())
        {
            *aud = ref_val;
        }
        if let Some(obj) = aud.as_object_mut() {
            obj.insert("minItems".to_owned(), json!(1));
        }
    }

    if let Some(req) = schema.get_mut("required").and_then(Value::as_array_mut)
        && !req.iter().any(|v| v.as_str() == Some("audience"))
    {
        req.push(json!("audience"));
        req.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
    }
}

/// Returns a `SchemaGenerator` configured for JSON Schema Draft 2020-12.
#[must_use]
pub fn draft2020_12_generator() -> SchemaGenerator {
    SchemaSettings::draft2020_12().into_generator()
}

/// Generates a deterministic JSON Schema Draft 2020-12 document for `T`.
#[must_use]
pub fn generate_json_schema<T: JsonSchema + ?Sized>() -> Value {
    let generator = draft2020_12_generator();
    let mut val = serde_json::to_value(generator.into_root_schema_for::<T>())
        .expect("schema serializes to JSON");
    normalize_schema_value(&mut val, None);
    val
}

fn generate_named_schema<T: JsonSchema + ?Sized>(override_title: Option<&str>) -> Value {
    let generator = draft2020_12_generator();
    let mut val = serde_json::to_value(generator.into_root_schema_for::<T>())
        .expect("schema serializes to JSON");
    normalize_schema_value(&mut val, override_title);
    val
}

fn normalize_schema_value(val: &mut Value, override_title: Option<&str>) {
    if let Some(obj) = val.as_object_mut() {
        obj.insert("$schema".to_owned(), json!(JSON_SCHEMA_DRAFT_2020_12));
        if let Some(title) = override_title {
            obj.insert("title".to_owned(), json!(title));
        }
    }
    val.sort_all_objects();
}

/// Generates all individual Draft 2020-12 JSON Schema documents keyed by type name.
#[must_use]
pub fn generate_all_json_schemas() -> BTreeMap<&'static str, Value> {
    let mut map = BTreeMap::new();
    macro_rules! insert_schema {
        ($ty:ty, $name:literal) => {
            map.insert($name, generate_named_schema::<$ty>(None));
        };
    }

    // Waist-1 IDs & version
    insert_schema!(ActionId, "ActionId");
    insert_schema!(ActorId, "ActorId");
    insert_schema!(BodyId, "BodyId");
    insert_schema!(CapabilityId, "CapabilityId");
    insert_schema!(CapabilitySchemaHash, "CapabilitySchemaHash");
    insert_schema!(ChannelId, "ChannelId");
    insert_schema!(CorrelationId, "CorrelationId");
    insert_schema!(DomainId, "DomainId");
    insert_schema!(EventId, "EventId");
    insert_schema!(PlaceId, "PlaceId");
    insert_schema!(Time, "Time");
    insert_schema!(ProtocolVersion, "ProtocolVersion");

    // Waist-1 domain facts & governance
    insert_schema!(ActionCanonicalInput, "ActionCanonicalInput");
    insert_schema!(Audience, "Audience");
    insert_schema!(CapabilityDescriptor, "CapabilityDescriptor");
    insert_schema!(CapabilityFlags, "CapabilityFlags");
    insert_schema!(CatchupPolicy, "CatchupPolicy");
    insert_schema!(DeclassToken, "DeclassToken");
    insert_schema!(DomainFactKind, "DomainFactKind");
    insert_schema!(HAct, "HAct");
    insert_schema!(OriginActor, "OriginActor");
    insert_schema!(PendingApproval, "PendingApproval");
    insert_schema!(PrincipalLink, "PrincipalLink");
    insert_schema!(PrincipalLinkStatus, "PrincipalLinkStatus");
    insert_schema!(PromiseMeta, "PromiseMeta");
    insert_schema!(Receipt, "Receipt");
    insert_schema!(ReceiptOutcome, "ReceiptOutcome");
    insert_schema!(RecurrenceSchedule, "RecurrenceSchedule");
    insert_schema!(RecurrenceSpec, "RecurrenceSpec");
    insert_schema!(SensedEffect, "SensedEffect");
    insert_schema!(Taint, "Taint");
    insert_schema!(TerminationReason, "TerminationReason");
    insert_schema!(VisibleScope, "VisibleScope");

    // Waist-1 messages & thought acts
    insert_schema!(Action, "Action");
    insert_schema!(ActorFacts, "ActorFacts");
    insert_schema!(AttachmentHandle, "AttachmentHandle");
    insert_schema!(BodyState, "BodyState");
    insert_schema!(BodyStatus, "BodyStatus");
    insert_schema!(ChannelPrediction, "ChannelPrediction");
    insert_schema!(ClassificationVerdict, "ClassificationVerdict");
    insert_schema!(ConsolidateOutput, "ConsolidateOutput");
    insert_schema!(EmbeddingTag, "EmbeddingTag");
    insert_schema!(ExpectKind, "ExpectKind");
    insert_schema!(FModal, "FModal");
    insert_schema!(ModalitySlot, "ModalitySlot");
    insert_schema!(OpaquePayload, "OpaquePayload");
    insert_schema!(OrientScore, "OrientScore");
    insert_schema!(Percept, "Percept");
    insert_schema!(ResolveOutcome, "ResolveOutcome");
    insert_schema!(TaggedEmbedding, "TaggedEmbedding");
    insert_schema!(ThoughtAct, "ThoughtAct");
    insert_schema!(ThoughtBatch, "ThoughtBatch");

    // Waist-2 envelopes & twelve methods
    insert_schema!(ActionCancelParams, "ActionCancelParams");
    insert_schema!(ActionCancelResult, "ActionCancelResult");
    insert_schema!(ActionExecuteParams, "ActionExecuteParams");
    insert_schema!(ActionExecuteResult, "ActionExecuteResult");
    insert_schema!(ActionReceiptParams, "ActionReceiptParams");
    insert_schema!(ActionReceiptResult, "ActionReceiptResult");
    insert_schema!(ApprovalResolveParams, "ApprovalResolveParams");
    insert_schema!(ApprovalResolveResult, "ApprovalResolveResult");
    insert_schema!(BlobGetParams, "BlobGetParams");
    insert_schema!(BlobGetResult, "BlobGetResult");
    insert_schema!(BlobPutParams, "BlobPutParams");
    insert_schema!(BlobPutResult, "BlobPutResult");
    insert_schema!(BodyDiscoverParams, "BodyDiscoverParams");
    insert_schema!(BodyDiscoverResult, "BodyDiscoverResult");
    insert_schema!(BodyHeartbeatParams, "BodyHeartbeatParams");
    insert_schema!(BodyHeartbeatResult, "BodyHeartbeatResult");
    insert_schema!(BodyShutdownParams, "BodyShutdownParams");
    insert_schema!(BodyShutdownResult, "BodyShutdownResult");
    insert_schema!(MetaValue, "MetaValue");
    insert_schema!(PerceptEmitParams, "PerceptEmitParams");
    insert_schema!(PerceptEmitResult, "PerceptEmitResult");
    insert_schema!(QuestionAnswerParams, "QuestionAnswerParams");
    insert_schema!(QuestionAnswerResult, "QuestionAnswerResult");
    insert_schema!(RpcId, "RpcId");
    insert_schema!(StateQueryParams, "StateQueryParams");
    insert_schema!(StateQueryResult, "StateQueryResult");
    insert_schema!(StateQueryStatus, "StateQueryStatus");
    insert_schema!(Waist2Call, "Waist2Call");
    insert_schema!(Waist2Error, "Waist2Error");
    insert_schema!(Waist2ErrorData, "Waist2ErrorData");
    insert_schema!(Waist2Headers, "Waist2Headers");
    insert_schema!(Waist2Method, "Waist2Method");
    map.insert(
        "Waist2Request",
        generate_named_schema::<Waist2Request<Value>>(Some("Waist2Request")),
    );
    map.insert(
        "Waist2Response",
        generate_named_schema::<Waist2Response<Value>>(Some("Waist2Response")),
    );

    map
}

struct MethodSpec {
    method: Waist2Method,
    summary: &'static str,
    params_schema_name: &'static str,
    result_schema_name: &'static str,
    errors: &'static [Waist2ErrorCode],
}

const METHOD_SPECS: [MethodSpec; 12] = [
    MethodSpec {
        method: Waist2Method::BodyDiscover,
        summary: "Advertise deterministic body capabilities, places, and supported protocol version.",
        params_schema_name: "BodyDiscoverParams",
        result_schema_name: "BodyDiscoverResult",
        errors: &[Waist2ErrorCode::UnsupportedVersion],
    },
    MethodSpec {
        method: Waist2Method::BodyHeartbeat,
        summary: "Report body liveness and operational status to the gateway.",
        params_schema_name: "BodyHeartbeatParams",
        result_schema_name: "BodyHeartbeatResult",
        errors: &[Waist2ErrorCode::UnsupportedVersion],
    },
    MethodSpec {
        method: Waist2Method::BodyShutdown,
        summary: "Request graceful shutdown of a connected body adapter.",
        params_schema_name: "BodyShutdownParams",
        result_schema_name: "BodyShutdownResult",
        errors: &[Waist2ErrorCode::UnsupportedVersion],
    },
    MethodSpec {
        method: Waist2Method::PerceptEmit,
        summary: "Emit a body-sensed percept carrying six domain facts and transport headers.",
        params_schema_name: "PerceptEmitParams",
        result_schema_name: "PerceptEmitResult",
        errors: &[
            Waist2ErrorCode::DuplicateIdempotency,
            Waist2ErrorCode::UnsupportedVersion,
            Waist2ErrorCode::UndeclaredAudience,
        ],
    },
    MethodSpec {
        method: Waist2Method::ActionExecute,
        summary: "Dispatch an action to a declared body capability with idempotency_key.",
        params_schema_name: "ActionExecuteParams",
        result_schema_name: "ActionExecuteResult",
        errors: &[
            Waist2ErrorCode::UnknownCapability,
            Waist2ErrorCode::DuplicateIdempotency,
            Waist2ErrorCode::UnsupportedVersion,
            Waist2ErrorCode::UndeclaredAudience,
        ],
    },
    MethodSpec {
        method: Waist2Method::ActionCancel,
        summary: "Cancel or preempt an in-flight action, triggering an Option A receipt.",
        params_schema_name: "ActionCancelParams",
        result_schema_name: "ActionCancelResult",
        errors: &[
            Waist2ErrorCode::InvalidReceiptTransition,
            Waist2ErrorCode::UnsupportedVersion,
        ],
    },
    MethodSpec {
        method: Waist2Method::ActionReceipt,
        summary: "Deliver a single terminal or held Option A receipt for an executed action.",
        params_schema_name: "ActionReceiptParams",
        result_schema_name: "ActionReceiptResult",
        errors: &[
            Waist2ErrorCode::DuplicateIdempotency,
            Waist2ErrorCode::InvalidReceiptTransition,
            Waist2ErrorCode::UnsupportedVersion,
        ],
    },
    MethodSpec {
        method: Waist2Method::ApprovalResolve,
        summary: "Resolve a held PendingApproval bound to canonical action hash H_act.",
        params_schema_name: "ApprovalResolveParams",
        result_schema_name: "ApprovalResolveResult",
        errors: &[
            Waist2ErrorCode::InvalidReceiptTransition,
            Waist2ErrorCode::UnsupportedVersion,
        ],
    },
    MethodSpec {
        method: Waist2Method::QuestionAnswer,
        summary: "Deliver a user answer to an outstanding gateway/body question.",
        params_schema_name: "QuestionAnswerParams",
        result_schema_name: "QuestionAnswerResult",
        errors: &[
            Waist2ErrorCode::UnsupportedVersion,
            Waist2ErrorCode::UndeclaredAudience,
        ],
    },
    MethodSpec {
        method: Waist2Method::StateQuery,
        summary: "Query crash-recovery status (in_flight, terminal receipt, or unknown) for an action.",
        params_schema_name: "StateQueryParams",
        result_schema_name: "StateQueryResult",
        errors: &[Waist2ErrorCode::UnsupportedVersion],
    },
    MethodSpec {
        method: Waist2Method::BlobPut,
        summary: "Stage an out-of-band binary attachment (>256 KiB, 60 s TTL) with SHA-256 verification.",
        params_schema_name: "BlobPutParams",
        result_schema_name: "BlobPutResult",
        errors: &[
            Waist2ErrorCode::DuplicateIdempotency,
            Waist2ErrorCode::UnsupportedVersion,
            Waist2ErrorCode::HashMismatch,
        ],
    },
    MethodSpec {
        method: Waist2Method::BlobGet,
        summary: "Fetch a staged binary attachment by blob_ref with optional SHA-256 verification.",
        params_schema_name: "BlobGetParams",
        result_schema_name: "BlobGetResult",
        errors: &[
            Waist2ErrorCode::UnsupportedVersion,
            Waist2ErrorCode::HashMismatch,
        ],
    },
];

const ALL_ERROR_CODES: [Waist2ErrorCode; 6] = [
    Waist2ErrorCode::UnknownCapability,
    Waist2ErrorCode::DuplicateIdempotency,
    Waist2ErrorCode::InvalidReceiptTransition,
    Waist2ErrorCode::UnsupportedVersion,
    Waist2ErrorCode::HashMismatch,
    Waist2ErrorCode::UndeclaredAudience,
];

fn error_message(code: Waist2ErrorCode) -> &'static str {
    match code {
        Waist2ErrorCode::UnknownCapability => "ERR_UNKNOWN_CAPABILITY: unknown capability",
        Waist2ErrorCode::DuplicateIdempotency => {
            "ERR_DUPLICATE_IDEMPOTENCY: duplicate idempotency_key"
        }
        Waist2ErrorCode::InvalidReceiptTransition => {
            "ERR_INVALID_RECEIPT_TRANSITION: invalid receipt transition"
        }
        Waist2ErrorCode::UnsupportedVersion => {
            "ERR_UNSUPPORTED_VERSION: unsupported protocol version"
        }
        Waist2ErrorCode::HashMismatch => "ERR_HASH_MISMATCH: SHA-256 digest mismatch",
        Waist2ErrorCode::UndeclaredAudience => {
            "ERR_UNDECLARED_AUDIENCE: percept must declare a non-empty audience"
        }
    }
}

fn params_descriptors_from_schema(params_schema: &Value) -> Vec<Value> {
    let required: BTreeSet<&str> = params_schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();

    let Some(props) = params_schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };

    let mut req_names: Vec<&str> = Vec::new();
    let mut opt_names: Vec<&str> = Vec::new();
    for name in props.keys() {
        if required.contains(name.as_str()) {
            req_names.push(name.as_str());
        } else {
            opt_names.push(name.as_str());
        }
    }
    req_names.sort_unstable();
    opt_names.sort_unstable();

    req_names
        .into_iter()
        .chain(opt_names)
        .map(|name| {
            json!({
                "name": name,
                "required": required.contains(name),
                "schema": props[name].clone(),
            })
        })
        .collect()
}

/// Generates the canonical OpenRPC 1.4.1 document for `monoagens.body.v1`.
#[must_use]
pub fn generate_openrpc_document() -> Value {
    let mut settings = SchemaSettings::draft2020_12();
    settings.definitions_path = "#/components/schemas".into();
    let mut generator = settings.into_generator();

    // Register all twelve method parameter/result pairs and shared envelope types.
    let _ = generator.subschema_for::<BodyDiscoverParams>();
    let _ = generator.subschema_for::<BodyDiscoverResult>();
    let _ = generator.subschema_for::<BodyHeartbeatParams>();
    let _ = generator.subschema_for::<BodyHeartbeatResult>();
    let _ = generator.subschema_for::<BodyShutdownParams>();
    let _ = generator.subschema_for::<BodyShutdownResult>();
    let _ = generator.subschema_for::<PerceptEmitParams>();
    let _ = generator.subschema_for::<PerceptEmitResult>();
    let _ = generator.subschema_for::<ActionExecuteParams>();
    let _ = generator.subschema_for::<ActionExecuteResult>();
    let _ = generator.subschema_for::<ActionCancelParams>();
    let _ = generator.subschema_for::<ActionCancelResult>();
    let _ = generator.subschema_for::<ActionReceiptParams>();
    let _ = generator.subschema_for::<ActionReceiptResult>();
    let _ = generator.subschema_for::<ApprovalResolveParams>();
    let _ = generator.subschema_for::<ApprovalResolveResult>();
    let _ = generator.subschema_for::<QuestionAnswerParams>();
    let _ = generator.subschema_for::<QuestionAnswerResult>();
    let _ = generator.subschema_for::<StateQueryParams>();
    let _ = generator.subschema_for::<StateQueryResult>();
    let _ = generator.subschema_for::<BlobPutParams>();
    let _ = generator.subschema_for::<BlobPutResult>();
    let _ = generator.subschema_for::<BlobGetParams>();
    let _ = generator.subschema_for::<BlobGetResult>();
    let _ = generator.subschema_for::<Waist2Error>();
    let _ = generator.subschema_for::<Waist2Method>();
    let _ = generator.subschema_for::<Waist2Call>();
    let _ = generator.subschema_for::<Percept>();
    let _ = generator.subschema_for::<Action>();
    let _ = generator.subschema_for::<BodyState>();

    let defs_map: Map<String, Value> = serde_json::from_value(
        serde_json::to_value(generator.take_definitions(true)).expect("definitions serialize"),
    )
    .expect("definitions map");

    let methods: Vec<Value> = METHOD_SPECS
        .iter()
        .map(|spec| {
            let params_schema = defs_map
                .get(spec.params_schema_name)
                .expect("registered params schema");
            let params = params_descriptors_from_schema(params_schema);
            let errors: Vec<Value> = spec
                .errors
                .iter()
                .map(|err| json!({ "$ref": format!("#/components/errors/{}", err.symbol()) }))
                .collect();
            json!({
                "name": spec.method.as_str(),
                "summary": spec.summary,
                "paramStructure": "by-name",
                "params": params,
                "result": {
                    "name": "result",
                    "schema": {
                        "$ref": format!("#/components/schemas/{}", spec.result_schema_name)
                    }
                },
                "errors": errors,
            })
        })
        .collect();

    let mut errors_map = Map::new();
    for code in ALL_ERROR_CODES {
        errors_map.insert(
            code.symbol().to_owned(),
            json!({
                "code": code.code(),
                "message": error_message(code),
            }),
        );
    }

    let mut doc = json!({
        "openrpc": OPENRPC_VERSION,
        "info": {
            "title": "monoagens.body.v1",
            "version": format!("{}.{}.0", CURRENT_PROTOCOL_VERSION.major, CURRENT_PROTOCOL_VERSION.minor),
            "description": "Waist 2 stateless JSON-RPC 2.0 wire contract for MonoAgens bodies.",
            "license": {
                "name": "Apache-2.0"
            }
        },
        "methods": methods,
        "components": {
            "schemas": Value::Object(defs_map),
            "errors": Value::Object(errors_map),
        }
    });
    doc.sort_all_objects();
    doc
}

/// Writes OpenRPC 1.4.1 and Draft 2020-12 JSON Schema files deterministically into `crate_dir`.
pub fn write_generated_artifacts(crate_dir: &Path) -> io::Result<Vec<PathBuf>> {
    let openrpc_dir = crate_dir.join("openrpc");
    let jsonschema_dir = crate_dir.join("jsonschema");
    fs::create_dir_all(&openrpc_dir)?;
    fs::create_dir_all(&jsonschema_dir)?;

    let mut written = Vec::new();

    let openrpc_path = openrpc_dir.join("monoagens.body.v1.openrpc.json");
    let openrpc_doc = generate_openrpc_document();
    write_pretty_json(&openrpc_path, &openrpc_doc)?;
    written.push(openrpc_path);

    for (name, schema) in generate_all_json_schemas() {
        let path = jsonschema_dir.join(format!("{name}.schema.json"));
        write_pretty_json(&path, &schema)?;
        written.push(path);
    }

    Ok(written)
}

fn write_pretty_json(path: &Path, value: &Value) -> io::Result<()> {
    let mut rendered = serde_json::to_string_pretty(value)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    rendered.push('\n');
    fs::write(path, rendered)
}

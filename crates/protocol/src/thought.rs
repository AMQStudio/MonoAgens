//! Reasoning thought acts (`think`, `act`, `expect`, `resolve`, `revise`, `consolidate`)
//! and classification verdicts under strict nullable schemas (`R | null`).

use serde::{Deserialize, Serialize};

use crate::facts::{DomainFactKind, PromiseMeta, ProtocolError, RecurrenceSpec, SensedEffect};
use crate::ids::{ActorId, CapabilityId, CapabilitySchemaHash, DomainId, EventId, PlaceId, Time};
use crate::message::OpaquePayload;
use crate::version::{ProtocolVersion, check_version};

/// Expectation kinds supported by `expect` acts and Core's expectation book.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ExpectKind {
    Effect,
    Response,
    Wait,
    Promise,
    Goal,
    Monitor,
    PendingApproval,
}

/// Terminal or transitional outcomes supported by `resolve`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum ResolveOutcome {
    Kept,
    KeptLate,
    Renegotiated,
    Released,
    Failed,
    Cancelled,
    Answered,
    Reached,
}

/// Typed outputs emitted during sleep consolidation (`consolidate`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "consolidate_kind", rename_all = "snake_case")]
pub enum ConsolidateOutput {
    Gist {
        summary: String,
        about_ids: Vec<EventId>,
        places: Vec<PlaceId>,
        people: Vec<ActorId>,
        domain: Option<DomainId>,
    },
    Lesson {
        pattern: String,
        failure_signature: String,
        contrastive_success_ids: Vec<EventId>,
        failure_ids: Vec<EventId>,
        domain: Option<DomainId>,
    },
    Procedure {
        name: String,
        steps: Vec<String>,
        required_cap_ids: Vec<CapabilityId>,
        capability_schema_hash: CapabilitySchemaHash,
        domain: Option<DomainId>,
    },
}

/// Strict nullable schema contract for reasoning and consolidation acts (`A-10`, `A-11`).
///
/// Optional fields intentionally omit `skip_serializing_if` so they serialize explicitly
/// as `null` (`R | null`) for strict structured-output JSON schemas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(tag = "act", rename_all = "snake_case")]
pub enum ThoughtAct {
    Think {
        thought: String,
        motivation: u8,
        confidence_0_20: u8,
        used_ids: Vec<EventId>,
        importance: Option<f32>,
        basis: Option<String>,
        value: Option<f32>,
        cost: Option<f32>,
        domain: Option<DomainId>,
    },
    Act {
        capability_id: CapabilityId,
        places: Vec<PlaceId>,
        people: Vec<ActorId>,
        payload: OpaquePayload,
        motivation: u8,
        confidence_0_20: u8,
        used_ids: Vec<EventId>,
        expected: Option<SensedEffect>,
        domain: Option<DomainId>,
    },
    Expect {
        expect_kind: ExpectKind,
        about_ids: Vec<EventId>,
        places: Vec<PlaceId>,
        people: Vec<ActorId>,
        condition: String,
        due_ms: Option<Time>,
        expiry_ms: Time,
        recurrence: Option<RecurrenceSpec>,
        promise_meta: Option<PromiseMeta>,
        domain: Option<DomainId>,
    },
    Resolve {
        expectation_id: EventId,
        outcome: ResolveOutcome,
        note: Option<String>,
        places: Vec<PlaceId>,
        people: Vec<ActorId>,
        domain: Option<DomainId>,
    },
    Revise {
        target_id: EventId,
        new_version: u32,
        condition: Option<String>,
        due_ms: Option<Time>,
        expiry_ms: Option<Time>,
        places: Vec<PlaceId>,
        people: Vec<ActorId>,
        domain: Option<DomainId>,
    },
    Consolidate {
        output: ConsolidateOutput,
    },
}

impl ThoughtAct {
    /// Validates bounded rubric invariants (`motivation` in `1..=5`, `confidence_0_20 <= 20`).
    pub fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Think {
                motivation,
                confidence_0_20,
                ..
            } => validate_rubric(*motivation, *confidence_0_20),
            Self::Act {
                places,
                motivation,
                confidence_0_20,
                ..
            } => {
                validate_rubric(*motivation, *confidence_0_20)?;
                if places.is_empty() {
                    return Err(ProtocolError::MissingDomainFact {
                        fact: DomainFactKind::Place,
                        detail: "act must specify at least one place".to_owned(),
                    });
                }
                Ok(())
            }
            Self::Expect { .. }
            | Self::Resolve { .. }
            | Self::Revise { .. }
            | Self::Consolidate { .. } => Ok(()),
        }
    }
}

fn validate_rubric(motivation: u8, confidence_0_20: u8) -> Result<(), ProtocolError> {
    if !(1..=5).contains(&motivation) {
        return Err(ProtocolError::InvalidField {
            field: "motivation",
            detail: format!("motivation {motivation} must be in 1..=5"),
        });
    }
    if confidence_0_20 > 20 {
        return Err(ProtocolError::InvalidField {
            field: "confidence_0_20",
            detail: format!("confidence_0_20 {confidence_0_20} must be in 0..=20"),
        });
    }
    Ok(())
}

/// Versioned batch of thought acts returned by a model provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ThoughtBatch {
    #[serde(default)]
    pub protocol_version: ProtocolVersion,
    pub acts: Vec<ThoughtAct>,
}

impl ThoughtBatch {
    /// Validates protocol version and every contained [`ThoughtAct`].
    pub fn validate(&self, supported_version: ProtocolVersion) -> Result<(), ProtocolError> {
        check_version(self.protocol_version, supported_version)?;
        for act in &self.acts {
            act.validate()?;
        }
        Ok(())
    }
}

/// Verdict returned by a `Classify` model call (e.g., judged-silence or learning outcome evaluation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct ClassificationVerdict {
    #[serde(default)]
    pub protocol_version: ProtocolVersion,
    pub label: String,
    pub score: Option<f32>,
    pub rationale: Option<String>,
}

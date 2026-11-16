//! Compiled Cedar policy bundle with `cedar-policy-symcc` (`LocalSolver::cvc5()`)
//! static verification on policy load.

use core::fmt;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;

use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityId, EntityTypeName, EntityUid,
    PolicySet, Request, RestrictedExpression, Schema, ValidationMode, Validator,
};
use cedar_policy_symcc::{CedarSymCompiler, CompiledPolicySet, solver::LocalSolver};
use monoagens_protocol::{ActorId, CapabilityFlags, CapabilityId, PlaceId};
use serde::{Deserialize, Serialize};

use crate::roles::AttestedRoles;

/// Compiled-in default Cedar schema asset.
pub const DEFAULT_CEDAR_SCHEMA: &str = include_str!("../policies/default.cedarschema");
/// Compiled-in default Cedar policy bundle asset.
pub const DEFAULT_CEDAR_POLICIES: &str = include_str!("../policies/default.cedar");

/// Canonical Cedar entity identifier reserved exclusively for the verified owner.
const OWNER_CEDAR_EID: &str = "owner";

/// Reference owner-only specification for `SelfModify` used by `symcc` verification.
const SPEC_SELF_MODIFY_OWNER_ONLY: &str = r#"
permit(
    principal == Actor::"owner",
    action == Action::"SelfModify",
    resource
);
"#;

/// Reference owner-only specification for `InvokeCapability` (`identity_changing` paths).
const SPEC_INVOKE_CAPABILITY_OWNER_ONLY: &str = r#"
permit(
    principal == Actor::"owner",
    action == Action::"InvokeCapability",
    resource
);
permit(
    principal,
    action == Action::"InvokeCapability",
    resource
) when {
    !resource.identity_changing
};
"#;

/// Static proof status from `cedar-policy-symcc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    Unsat,
}

/// Structured evidence of `cedar-policy-symcc` verification recorded on policy activation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymccProofReport {
    pub status: ProofStatus,
    pub solver: String,
    pub checked_envs: Vec<String>,
    pub counterexamples: usize,
}

/// Fail-closed policy compilation or `symcc` verification errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    SchemaParse(String),
    PolicyParse(String),
    ValidationFailed(String),
    SolverUnavailable(String),
    SymccCompile(String),
    SymccVerificationFailed {
        action: String,
        counterexample: String,
    },
    RuntimeBuild(String),
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SchemaParse(e) => write!(f, "cedar schema parse failed: {e}"),
            Self::PolicyParse(e) => write!(f, "cedar policy parse failed: {e}"),
            Self::ValidationFailed(e) => write!(f, "cedar policy validation failed: {e}"),
            Self::SolverUnavailable(e) => write!(f, "symcc cvc5 solver unavailable: {e}"),
            Self::SymccCompile(e) => write!(f, "symcc compilation failed: {e}"),
            Self::SymccVerificationFailed {
                action,
                counterexample,
            } => write!(
                f,
                "symcc verification failed for '{action}': non-owner path permitted ({counterexample})"
            ),
            Self::RuntimeBuild(e) => write!(f, "symcc runtime initialization failed: {e}"),
        }
    }
}

impl std::error::Error for PolicyError {}

/// Memory-write attributes evaluated by [`PolicyBundle::eval_memory_write`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryWritePolicyAttrs<'a> {
    pub item_id: &'a str,
    pub is_procedure: bool,
    pub owner_signed: bool,
    pub trusted: bool,
    pub tainted: bool,
}

/// Verified Cedar policy bundle + reusable `Authorizer` and `Schema`.
#[derive(Debug, Clone)]
pub struct PolicyBundle {
    schema: Schema,
    policy_set: PolicySet,
    authorizer: Authorizer,
    proof_report: SymccProofReport,
    actor_type: EntityTypeName,
    place_type: EntityTypeName,
    capability_type: EntityTypeName,
    memory_item_type: EntityTypeName,
    action_inbound: EntityUid,
    action_context: EntityUid,
    action_invoke: EntityUid,
    action_self_modify: EntityUid,
    action_write_memory: EntityUid,
    role_known: EntityUid,
    role_room: EntityUid,
    role_physical: EntityUid,
}

impl PolicyBundle {
    /// Loads and statically verifies the compiled-in default policy bundle using `LocalSolver::cvc5()`.
    pub fn default_verified() -> Result<Self, PolicyError> {
        Self::from_cedar_str(DEFAULT_CEDAR_SCHEMA, DEFAULT_CEDAR_POLICIES)
    }

    /// Parses, validates, and `symcc`-verifies a Cedar schema and policy set using `LocalSolver::cvc5()`.
    pub fn from_cedar_str(schema_src: &str, policy_src: &str) -> Result<Self, PolicyError> {
        Self::from_cedar_str_with_solver(schema_src, policy_src, LocalSolver::cvc5)
    }

    /// Parses, validates, and `symcc`-verifies using an explicit solver factory (used to test
    /// fail-closed behavior when `cvc5` is unavailable).
    pub fn from_cedar_str_with_solver<F>(
        schema_src: &str,
        policy_src: &str,
        make_solver: F,
    ) -> Result<Self, PolicyError>
    where
        F: FnOnce() -> Result<LocalSolver, cedar_policy_symcc::solver::SolverError>,
    {
        let (schema, _warnings) = Schema::from_cedarschema_str(schema_src)
            .map_err(|e| PolicyError::SchemaParse(e.to_string()))?;
        let policy_set =
            PolicySet::from_str(policy_src).map_err(|e| PolicyError::PolicyParse(e.to_string()))?;

        let validator = Validator::new(schema.clone());
        let validation = validator.validate(&policy_set, ValidationMode::Strict);
        if !validation.validation_passed() {
            let errs: Vec<String> = validation
                .validation_errors()
                .map(|e| e.to_string())
                .collect();
            return Err(PolicyError::ValidationFailed(errs.join("; ")));
        }

        let proof_report =
            verify_owner_only_identity_invariants(&schema, &policy_set, make_solver)?;

        let actor_type = EntityTypeName::from_str("Actor")
            .map_err(|e| PolicyError::SchemaParse(e.to_string()))?;
        let role_type = EntityTypeName::from_str("Role")
            .map_err(|e| PolicyError::SchemaParse(e.to_string()))?;
        let place_type = EntityTypeName::from_str("Place")
            .map_err(|e| PolicyError::SchemaParse(e.to_string()))?;
        let capability_type = EntityTypeName::from_str("Capability")
            .map_err(|e| PolicyError::SchemaParse(e.to_string()))?;
        let memory_item_type = EntityTypeName::from_str("MemoryItem")
            .map_err(|e| PolicyError::SchemaParse(e.to_string()))?;

        Ok(Self {
            schema,
            policy_set,
            authorizer: Authorizer::new(),
            proof_report,
            actor_type,
            place_type,
            capability_type,
            memory_item_type,
            action_inbound: EntityUid::from_str(r#"Action::"InboundPercept""#)
                .map_err(|e| PolicyError::SchemaParse(e.to_string()))?,
            action_context: EntityUid::from_str(r#"Action::"ContextAssemble""#)
                .map_err(|e| PolicyError::SchemaParse(e.to_string()))?,
            action_invoke: EntityUid::from_str(r#"Action::"InvokeCapability""#)
                .map_err(|e| PolicyError::SchemaParse(e.to_string()))?,
            action_self_modify: EntityUid::from_str(r#"Action::"SelfModify""#)
                .map_err(|e| PolicyError::SchemaParse(e.to_string()))?,
            action_write_memory: EntityUid::from_str(r#"Action::"WriteMemory""#)
                .map_err(|e| PolicyError::SchemaParse(e.to_string()))?,
            role_known: EntityUid::from_type_name_and_id(
                role_type.clone(),
                EntityId::new("known_actor"),
            ),
            role_room: EntityUid::from_type_name_and_id(
                role_type.clone(),
                EntityId::new("room_present"),
            ),
            role_physical: EntityUid::from_type_name_and_id(
                role_type,
                EntityId::new("physically_present"),
            ),
        })
    }

    /// Returns the `cedar-policy-symcc` UNSAT proof report captured at load time.
    #[must_use]
    pub fn proof_report(&self) -> &SymccProofReport {
        &self.proof_report
    }

    /// Returns the parsed Cedar [`Schema`].
    #[must_use]
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Evaluates the Inbound door policy deterministically over snapshot facts.
    #[must_use]
    pub fn eval_inbound(
        &self,
        actor: &ActorId,
        roles: AttestedRoles,
        place: &PlaceId,
        has_audience: bool,
    ) -> Decision {
        let principal = self.principal_uid(actor, roles.is_owner);
        let resource = EntityUid::from_type_name_and_id(
            self.place_type.clone(),
            EntityId::new(place.as_str()),
        );
        let Ok(context) = Context::from_pairs([
            (
                "has_audience".to_owned(),
                RestrictedExpression::new_bool(has_audience),
            ),
            (
                "place_present".to_owned(),
                RestrictedExpression::new_bool(roles.room_present),
            ),
            (
                "physical_present".to_owned(),
                RestrictedExpression::new_bool(roles.physically_present),
            ),
        ]) else {
            return Decision::Deny;
        };

        let Ok(request) = Request::new(
            principal.clone(),
            self.action_inbound.clone(),
            resource.clone(),
            context,
            None,
        ) else {
            return Decision::Deny;
        };

        let Some(entities) = self.build_entities(
            principal,
            roles,
            Entity::new_no_attrs(resource, HashSet::new()),
        ) else {
            return Decision::Deny;
        };
        self.authorizer
            .is_authorized(&request, &self.policy_set, &entities)
            .decision()
    }

    /// Evaluates the Context door policy deterministically.
    #[must_use]
    pub fn eval_context(
        &self,
        actor: &ActorId,
        roles: AttestedRoles,
        place: Option<&PlaceId>,
        has_scope: bool,
    ) -> Decision {
        let principal = self.principal_uid(actor, roles.is_owner);
        let place_name = place.map_or("context:global", PlaceId::as_str);
        let resource =
            EntityUid::from_type_name_and_id(self.place_type.clone(), EntityId::new(place_name));
        let Ok(context) = Context::from_pairs([(
            "has_scope".to_owned(),
            RestrictedExpression::new_bool(has_scope),
        )]) else {
            return Decision::Deny;
        };
        let Ok(request) = Request::new(
            principal.clone(),
            self.action_context.clone(),
            resource.clone(),
            context,
            None,
        ) else {
            return Decision::Deny;
        };
        let Some(entities) = self.build_entities(
            principal,
            roles,
            Entity::new_no_attrs(resource, HashSet::new()),
        ) else {
            return Decision::Deny;
        };
        self.authorizer
            .is_authorized(&request, &self.policy_set, &entities)
            .decision()
    }

    /// Evaluates the Action door policy (`SelfModify` or `InvokeCapability`) deterministically.
    #[must_use]
    pub fn eval_action(
        &self,
        actor: &ActorId,
        roles: AttestedRoles,
        capability_id: &CapabilityId,
        flags: CapabilityFlags,
    ) -> Decision {
        let principal = self.principal_uid(actor, roles.is_owner);
        let resource = EntityUid::from_type_name_and_id(
            self.capability_type.clone(),
            EntityId::new(capability_id.as_str()),
        );
        let mut attrs = HashMap::with_capacity(3);
        attrs.insert(
            "identity_changing".to_owned(),
            RestrictedExpression::new_bool(flags.identity_changing),
        );
        attrs.insert(
            "irreversible".to_owned(),
            RestrictedExpression::new_bool(flags.irreversible),
        );
        attrs.insert(
            "communicative".to_owned(),
            RestrictedExpression::new_bool(flags.communicative),
        );
        let Ok(cap_entity) = Entity::new(resource.clone(), attrs, HashSet::new()) else {
            return Decision::Deny;
        };
        let Some(entities) = self.build_entities(principal.clone(), roles, cap_entity) else {
            return Decision::Deny;
        };

        if flags.identity_changing {
            let Ok(self_mod_req) = Request::new(
                principal.clone(),
                self.action_self_modify.clone(),
                resource.clone(),
                Context::empty(),
                None,
            ) else {
                return Decision::Deny;
            };
            if self
                .authorizer
                .is_authorized(&self_mod_req, &self.policy_set, &entities)
                .decision()
                != Decision::Allow
            {
                return Decision::Deny;
            }
        }

        let Ok(context) = Context::from_pairs([
            (
                "place_present".to_owned(),
                RestrictedExpression::new_bool(roles.room_present),
            ),
            (
                "physical_present".to_owned(),
                RestrictedExpression::new_bool(roles.physically_present),
            ),
        ]) else {
            return Decision::Deny;
        };
        let Ok(invoke_req) = Request::new(
            principal,
            self.action_invoke.clone(),
            resource,
            context,
            None,
        ) else {
            return Decision::Deny;
        };
        self.authorizer
            .is_authorized(&invoke_req, &self.policy_set, &entities)
            .decision()
    }

    /// Evaluates the Memory-write door policy deterministically.
    #[must_use]
    pub fn eval_memory_write(
        &self,
        actor: &ActorId,
        roles: AttestedRoles,
        attrs_in: MemoryWritePolicyAttrs<'_>,
    ) -> Decision {
        let principal = self.principal_uid(actor, roles.is_owner);
        let resource = EntityUid::from_type_name_and_id(
            self.memory_item_type.clone(),
            EntityId::new(attrs_in.item_id),
        );
        let mut attrs = HashMap::with_capacity(3);
        attrs.insert(
            "is_procedure".to_owned(),
            RestrictedExpression::new_bool(attrs_in.is_procedure),
        );
        attrs.insert(
            "owner_signed".to_owned(),
            RestrictedExpression::new_bool(attrs_in.owner_signed),
        );
        attrs.insert(
            "trusted".to_owned(),
            RestrictedExpression::new_bool(attrs_in.trusted),
        );
        let Ok(mem_entity) = Entity::new(resource.clone(), attrs, HashSet::new()) else {
            return Decision::Deny;
        };
        let Some(entities) = self.build_entities(principal.clone(), roles, mem_entity) else {
            return Decision::Deny;
        };
        let Ok(context) = Context::from_pairs([(
            "tainted".to_owned(),
            RestrictedExpression::new_bool(attrs_in.tainted),
        )]) else {
            return Decision::Deny;
        };
        let Ok(request) = Request::new(
            principal,
            self.action_write_memory.clone(),
            resource,
            context,
            None,
        ) else {
            return Decision::Deny;
        };
        self.authorizer
            .is_authorized(&request, &self.policy_set, &entities)
            .decision()
    }

    fn principal_uid(&self, actor: &ActorId, is_owner: bool) -> EntityUid {
        if is_owner {
            EntityUid::from_type_name_and_id(
                self.actor_type.clone(),
                EntityId::new(OWNER_CEDAR_EID),
            )
        } else {
            // Prefix non-owner identifiers so no actor string like `"owner"` can ever alias Actor::"owner".
            let safe_id = format!("peer:{}", actor.as_str());
            EntityUid::from_type_name_and_id(self.actor_type.clone(), EntityId::new(safe_id))
        }
    }

    fn build_entities(
        &self,
        principal: EntityUid,
        roles: AttestedRoles,
        resource_entity: Entity,
    ) -> Option<Entities> {
        let mut parents = HashSet::new();
        if roles.known_actor {
            parents.insert(self.role_known.clone());
        }
        if roles.room_present {
            parents.insert(self.role_room.clone());
        }
        if roles.physically_present {
            parents.insert(self.role_physical.clone());
        }

        let actor_entity = Entity::new_no_attrs(principal, parents);
        let role_known_e = Entity::new_no_attrs(self.role_known.clone(), HashSet::new());
        let role_room_e = Entity::new_no_attrs(self.role_room.clone(), HashSet::new());
        let role_phys_e = Entity::new_no_attrs(self.role_physical.clone(), HashSet::new());

        Entities::from_entities(
            [
                actor_entity,
                role_known_e,
                role_room_e,
                role_phys_e,
                resource_entity,
            ],
            None,
        )
        .ok()
    }
}

fn verify_owner_only_identity_invariants<F>(
    schema: &Schema,
    policy_set: &PolicySet,
    make_solver: F,
) -> Result<SymccProofReport, PolicyError>
where
    F: FnOnce() -> Result<LocalSolver, cedar_policy_symcc::solver::SolverError>,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(|e| PolicyError::RuntimeBuild(e.to_string()))?;
    let _guard = rt.enter();

    let solver = make_solver().map_err(|e| PolicyError::SolverUnavailable(e.to_string()))?;
    let mut compiler =
        CedarSymCompiler::new(solver).map_err(|e| PolicyError::SymccCompile(e.to_string()))?;

    let spec_self_mod = PolicySet::from_str(SPEC_SELF_MODIFY_OWNER_ONLY)
        .map_err(|e| PolicyError::SymccCompile(e.to_string()))?;
    let spec_invoke = PolicySet::from_str(SPEC_INVOKE_CAPABILITY_OWNER_ONLY)
        .map_err(|e| PolicyError::SymccCompile(e.to_string()))?;

    let mut checked_envs = Vec::new();
    for req_env in schema.request_envs() {
        let action_str = req_env.action().to_string();
        let spec_opt = if action_str.contains("SelfModify") || action_str.contains("IdentityChange")
        {
            Some(&spec_self_mod)
        } else if action_str.contains("InvokeCapability") {
            Some(&spec_invoke)
        } else {
            None
        };

        let Some(spec_policies) = spec_opt else {
            continue;
        };

        let compiled_bundle = CompiledPolicySet::compile(policy_set, &req_env, schema)
            .map_err(|e| PolicyError::SymccCompile(e.to_string()))?;
        let compiled_spec = CompiledPolicySet::compile(spec_policies, &req_env, schema)
            .map_err(|e| PolicyError::SymccCompile(e.to_string()))?;

        let cex = rt
            .block_on(
                compiler.check_implies_with_counterexample_opt(&compiled_bundle, &compiled_spec),
            )
            .map_err(|e| PolicyError::SolverUnavailable(e.to_string()))?;

        if let Some(env) = cex {
            let p = env
                .request
                .principal()
                .map_or_else(|| "<unknown>".to_owned(), ToString::to_string);
            let a = env
                .request
                .action()
                .map_or_else(|| "<unknown>".to_owned(), ToString::to_string);
            let r = env
                .request
                .resource()
                .map_or_else(|| "<unknown>".to_owned(), ToString::to_string);
            return Err(PolicyError::SymccVerificationFailed {
                action: action_str,
                counterexample: format!("principal={p}, action={a}, resource={r}"),
            });
        }
        checked_envs.push(action_str);
    }

    if checked_envs.is_empty() {
        return Err(PolicyError::SymccCompile(
            "schema defines no SelfModify or InvokeCapability request environments to verify"
                .to_owned(),
        ));
    }

    Ok(SymccProofReport {
        status: ProofStatus::Unsat,
        solver: "cvc5".to_owned(),
        checked_envs,
        counterexamples: 0,
    })
}

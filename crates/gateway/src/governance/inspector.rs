//! Policy inspector data: the single source behind `monoagens inspector` and `policy.get`.

use monoagens_valve::{DEFAULT_CEDAR_POLICIES, PolicyBundle, SymccProofReport, ValveReasonTag};
use serde::Serialize;

use crate::config::GatewayConfig;

use super::approvals::build_valve_config;

/// One Cedar rule: its stable CLI identifier and source text.
#[derive(Debug, Clone, Serialize)]
pub struct CedarRule {
    /// `default_cedar_rule_NN_permit|forbid`, the identifier `monoagens inspector` prints.
    pub id: String,
    pub kind: String,
    /// Source text of the statement.
    pub source: String,
}

/// Rule ids as printed by the CLI inspector.
#[must_use]
pub fn extract_cedar_rule_summaries(policy_src: &str) -> Vec<String> {
    extract_cedar_rules(policy_src).into_iter().map(|r| r.id).collect()
}

/// Splits the policy text into rules: a rule starts at a `permit(`/`forbid(` line and runs up
/// to the line before the next rule (comments between rules attach to the rule that follows
/// them only through the preceding blank/comment lines, which are kept with the previous rule
/// otherwise dropped).
#[must_use]
pub fn extract_cedar_rules(policy_src: &str) -> Vec<CedarRule> {
    let mut rules: Vec<CedarRule> = Vec::new();
    let mut idx = 0usize;
    let mut pending_comments: Vec<&str> = Vec::new();
    for raw_line in policy_src.lines() {
        let trimmed = raw_line.trim();
        if trimmed.starts_with("permit(") || trimmed.starts_with("forbid(") {
            idx += 1;
            let kind = if trimmed.starts_with("permit") { "permit" } else { "forbid" };
            let mut source = String::new();
            for c in pending_comments.drain(..) {
                source.push_str(c);
                source.push('\n');
            }
            source.push_str(raw_line);
            rules.push(CedarRule {
                id: format!("default_cedar_rule_{idx:02}_{kind}"),
                kind: kind.to_owned(),
                source,
            });
        } else if let Some(last) = rules.last_mut().filter(|_| !trimmed.is_empty() && !trimmed.starts_with("//")) {
            last.source.push('\n');
            last.source.push_str(raw_line);
        } else if trimmed.starts_with("//") {
            pending_comments.push(raw_line);
        } else {
            pending_comments.clear();
        }
    }
    rules
}

/// Inspector payload (`monoagens inspector --json` fields plus rule sources and RBAC summary).
#[derive(Debug, Clone, Serialize)]
pub struct PolicyReport {
    pub valve_mode: String,
    pub symcc_proof: SymccProofReport,
    /// Identifiers, identical to the CLI's `active_rule_names`.
    pub active_rule_names: Vec<String>,
    pub canonical_reason_tags: Vec<String>,
    pub rules: Vec<CedarRule>,
    pub rbac: RbacSummary,
    /// The two absolute lines (`ARCHITECTURE.md`), fixed text.
    pub absolute_lines: Vec<String>,
}

/// Read-only RBAC summary.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RbacSummary {
    pub fallback_role: Option<String>,
    pub roles: Vec<String>,
    pub actor_roles: Vec<(String, Vec<String>)>,
    pub place_rules: usize,
}

/// Builds the report for `cfg`.
///
/// # Errors
/// Returns the policy-verification error text.
pub fn policy_report(cfg: &GatewayConfig) -> Result<PolicyReport, String> {
    let valve_cfg = build_valve_config(cfg);
    // The SMT proof of the default bundle is expensive and independent of the config: compute
    // it once per process. (`default_verified` blocks on its own runtime, so callers inside
    // a Tokio runtime must run this on a blocking thread.)
    static PROOF: std::sync::OnceLock<Result<SymccProofReport, String>> = std::sync::OnceLock::new();
    let proof = PROOF
        .get_or_init(|| {
            PolicyBundle::default_verified()
                .map(|b| b.proof_report().clone())
                .map_err(|e| format!("cedar policy verification failed: {e}"))
        })
        .clone()?;
    let rules = extract_cedar_rules(DEFAULT_CEDAR_POLICIES);
    let rbac = cfg.rbac.as_ref().map_or_else(RbacSummary::default, |r| RbacSummary {
        fallback_role: r.fallback_role.clone(),
        roles: r.roles.keys().cloned().collect(),
        actor_roles: r
            .actor_roles
            .iter()
            .map(|(a, roles)| (a.as_str().to_owned(), roles.clone()))
            .collect(),
        place_rules: r.places.len(),
    });
    Ok(PolicyReport {
        valve_mode: valve_cfg.mode.as_str().to_owned(),
        symcc_proof: proof,
        active_rule_names: rules.iter().map(|r| r.id.clone()).collect(),
        canonical_reason_tags: ValveReasonTag::ALL.iter().map(|t| t.as_str().to_owned()).collect(),
        rules,
        rbac,
        absolute_lines: vec![
            "Only the owner changes who the soul is.".to_owned(),
            "Private content never reaches anyone who was not part of it.".to_owned(),
        ],
    })
}

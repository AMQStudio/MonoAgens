//! People and account links: the JSON shapes `monoagens link --json` prints, produced here so
//! the CLI and the admin methods (`people.list`, `links.*`) cannot drift apart.

use monoagens_protocol::BodyId;
use serde_json::{Value, json};

use crate::config::GatewayConfig;
use crate::identity::{IdentityError, IdentityResolver};
use monoagens_protocol::Time;

/// File inside the data dir holding the identity link events.
pub const IDENTITY_LOG_FILENAME: &str = "identity_events.jsonl";

/// Builds the resolver exactly as `monoagens link` does (state lives in the jsonl log).
///
/// # Errors
/// Returns the resolver initialisation error text.
pub fn open_resolver(cfg: &GatewayConfig, data_dir: &std::path::Path) -> Result<IdentityResolver, String> {
    IdentityResolver::from_gateway_config(cfg, Some(data_dir.join(IDENTITY_LOG_FILENAME)))
        .map_err(|e| format!("failed to initialize identity resolver: {e}"))
}

/// `link list --json`: `{configured_persons, active_links}`.
#[must_use]
pub fn link_list_json(cfg: &GatewayConfig, resolver: &IdentityResolver) -> Value {
    let active: Vec<Value> = resolver
        .active_links()
        .into_iter()
        .map(|l| {
            json!({
                "body_id": l.body_id.as_str(),
                "account_id": l.account_id,
                "from_actor": l.from_actor.as_str(),
                "to_actor": l.to_actor.as_str(),
                "linked_at_ms": l.linked_at_ms.as_millis(),
                "retroactive_merge": l.retroactive_merge,
                "owner_confirmed": l.owner_confirmed,
            })
        })
        .collect();
    let configured: Vec<Value> = cfg
        .persons
        .iter()
        .map(|p| {
            json!({
                "actor_id": p.actor_id.as_str(),
                "is_owner": p.is_owner,
                "accounts": p.accounts.iter().map(|a| json!({
                    "body_id": a.body_id.as_str(),
                    "account_id": a.account_id,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "configured_persons": configured, "active_links": active })
}

/// `link initiate --json`.
///
/// # Errors
/// Returns the identity error text.
pub fn link_initiate_json(
    resolver: &IdentityResolver,
    primary: (&str, &str),
    secondary: (&str, &str),
    emit_nonces: bool,
    now: Time,
) -> Result<Value, String> {
    let issued = resolver
        .initiate_bilateral_challenge(
            &BodyId::new(primary.0),
            primary.1,
            &BodyId::new(secondary.0),
            secondary.1,
            now,
        )
        .map_err(|e: IdentityError| e.to_string())?;
    let mut payload = json!({
        "challenge_id": issued.challenge_id,
        "primary_actor": issued.primary_actor.as_str(),
        "secondary_actor": issued.secondary_actor.as_str(),
        "expires_at_ms": issued.expires_at_ms.as_millis(),
    });
    if emit_nonces {
        payload["nonce_1"] = json!(issued.nonce_1);
        payload["nonce_2"] = json!(issued.nonce_2);
    }
    Ok(payload)
}

/// `link verify --json`.
///
/// # Errors
/// Returns the identity error text.
pub fn link_verify_json(
    resolver: &IdentityResolver,
    challenge: &str,
    nonce1: &str,
    nonce2: &str,
    owner_confirmed: bool,
    now: Time,
) -> Result<Value, String> {
    let linked = resolver
        .verify_bilateral_challenge(challenge, nonce1, nonce2, owner_confirmed, now)
        .map_err(|e| e.to_string())?;
    Ok(json!({ "status": "linked", "challenge_id": challenge, "actor_id": linked.as_str() }))
}

/// `link unlink --json`.
///
/// # Errors
/// Returns the identity error text.
pub fn link_unlink_json(
    resolver: &IdentityResolver,
    body: &str,
    account: &str,
    reason: &str,
    now: Time,
) -> Result<Value, String> {
    resolver
        .unlink_actor(&BodyId::new(body), account, reason, now)
        .map_err(|e| e.to_string())?;
    Ok(json!({ "status": "unlinked", "body_id": body, "account_id": account }))
}

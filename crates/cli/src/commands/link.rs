use std::io::Write;
use std::path::Path;

use monoagens_gateway::{Clock, IdentityResolver, SystemClock};
use monoagens_protocol::BodyId;
use serde_json::json;

use crate::common::{load_paths, write_json_pretty};
use crate::{LinkArgs, LinkSubcommand};

pub(crate) fn cmd_link(
    config_path: Option<&Path>,
    json_mode: bool,
    args: LinkArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let (cfg_opt, paths) = load_paths(config_path)?;
    let cfg = cfg_opt.unwrap_or_default();
    let log_path = paths.data_dir.join("identity_events.jsonl");
    let resolver = IdentityResolver::from_gateway_config(&cfg, Some(log_path))
        .map_err(|e| format!("failed to initialize identity resolver: {e}"))?;
    let now = SystemClock::new().now();

    match args.action {
        LinkSubcommand::List => {
            let active: Vec<serde_json::Value> = resolver
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
            let configured: Vec<serde_json::Value> = cfg
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
            if json_mode {
                write_json_pretty(
                    out,
                    &json!({
                        "configured_persons": configured,
                        "active_links": active,
                    }),
                )?;
            } else {
                writeln!(
                    out,
                    "configured_persons={} active_links={}",
                    configured.len(),
                    active.len()
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
        LinkSubcommand::Resolve { body, account } => {
            let actor = resolver.resolve(&BodyId::new(body.clone()), &account);
            if json_mode {
                write_json_pretty(
                    out,
                    &json!({
                        "body_id": body,
                        "account_id": account,
                        "actor_id": actor.as_str(),
                        "heuristic_merge_enabled": resolver.heuristic_merge_enabled(),
                    }),
                )?;
            } else {
                writeln!(out, "{}", actor.as_str()).map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
        LinkSubcommand::Initiate {
            primary_body,
            primary_account,
            secondary_body,
            secondary_account,
            emit_nonces,
        } => {
            let issued = resolver
                .initiate_bilateral_challenge(
                    &BodyId::new(primary_body),
                    &primary_account,
                    &BodyId::new(secondary_body),
                    &secondary_account,
                    now,
                )
                .map_err(|e| e.to_string())?;
            if json_mode {
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
                write_json_pretty(out, &payload)?;
            } else {
                writeln!(
                    out,
                    "challenge_id={} primary={} secondary={} expires_at_ms={}",
                    issued.challenge_id,
                    issued.primary_actor.as_str(),
                    issued.secondary_actor.as_str(),
                    issued.expires_at_ms.as_millis()
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
        LinkSubcommand::Verify {
            challenge,
            nonce1,
            nonce2,
            owner_confirmed,
        } => {
            let linked = resolver
                .verify_bilateral_challenge(&challenge, &nonce1, &nonce2, owner_confirmed, now)
                .map_err(|e| e.to_string())?;
            if json_mode {
                write_json_pretty(
                    out,
                    &json!({
                        "status": "linked",
                        "challenge_id": challenge,
                        "actor_id": linked.as_str(),
                    }),
                )?;
            } else {
                writeln!(out, "linked actor={}", linked.as_str()).map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
        LinkSubcommand::Unlink {
            body,
            account,
            reason,
        } => {
            resolver
                .unlink_actor(&BodyId::new(body.clone()), &account, reason, now)
                .map_err(|e| e.to_string())?;
            if json_mode {
                write_json_pretty(
                    out,
                    &json!({
                        "status": "unlinked",
                        "body_id": body,
                        "account_id": account,
                    }),
                )?;
            } else {
                writeln!(out, "unlinked ({body}, {account})").map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
    }
}

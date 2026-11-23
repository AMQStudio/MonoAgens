use std::io::Write;
use std::path::Path;

use monoagens_gateway::governance::links::{
    link_initiate_json, link_list_json, link_unlink_json, link_verify_json, open_resolver,
};
use monoagens_gateway::{Clock, SystemClock};
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
    let resolver = open_resolver(&cfg, &paths.data_dir)?;
    let now = SystemClock::new().now();

    match args.action {
        LinkSubcommand::List => {
            let list = link_list_json(&cfg, &resolver);
            if json_mode {
                write_json_pretty(out, &list)?;
            } else {
                writeln!(
                    out,
                    "configured_persons={} active_links={}",
                    list["configured_persons"].as_array().map_or(0, Vec::len),
                    list["active_links"].as_array().map_or(0, Vec::len)
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
            let payload = link_initiate_json(
                &resolver,
                (&primary_body, &primary_account),
                (&secondary_body, &secondary_account),
                emit_nonces,
                now,
            )?;
            if json_mode {
                write_json_pretty(out, &payload)?;
            } else {
                writeln!(
                    out,
                    "challenge_id={} primary={} secondary={} expires_at_ms={}",
                    payload["challenge_id"].as_str().unwrap_or_default(),
                    payload["primary_actor"].as_str().unwrap_or_default(),
                    payload["secondary_actor"].as_str().unwrap_or_default(),
                    payload["expires_at_ms"].as_u64().unwrap_or_default()
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
            let payload =
                link_verify_json(&resolver, &challenge, &nonce1, &nonce2, owner_confirmed, now)?;
            if json_mode {
                write_json_pretty(out, &payload)?;
            } else {
                writeln!(
                    out,
                    "linked actor={}",
                    payload["actor_id"].as_str().unwrap_or_default()
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
        LinkSubcommand::Unlink {
            body,
            account,
            reason,
        } => {
            let payload = link_unlink_json(&resolver, &body, &account, &reason, now)?;
            if json_mode {
                write_json_pretty(out, &payload)?;
            } else {
                writeln!(out, "unlinked ({body}, {account})").map_err(|e| e.to_string())?;
            }
            Ok(0)
        }
    }
}

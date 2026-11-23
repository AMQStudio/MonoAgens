//! Slash commands sent from an adapter (`/help`, `/whoami`, `/link`): answered by the gateway
//! with zero model calls, filtered by the sender's permissions, for every admitted sender
//! including guests who may not wake the agent.

use std::collections::BTreeSet;

use monoagens_protocol::{ActorId, CapabilityId, Percept, PlaceId};
use monoagens_valve::{RbacConfig, agent_converse_permitted};
use serde_json::Value;

use crate::commands::{CommandOutcome, CommandRegistry};
use crate::config::GatewayConfig;

/// Facts the command handler needs, refreshed whenever `[[person]]`, RBAC or `[valve]` change.
#[derive(Debug, Clone, Default)]
pub struct CommandContext {
    pub owner: Option<ActorId>,
    pub rbac: Option<RbacConfig>,
    /// Actors of `[[person]]` entries (everyone else is a guest).
    pub registered: BTreeSet<ActorId>,
    pub guest_access: bool,
}

impl CommandContext {
    /// Builds the context from a parsed configuration.
    #[must_use]
    pub fn from_config(cfg: &GatewayConfig) -> Self {
        let owner = cfg
            .persons
            .iter()
            .find(|p| p.is_owner)
            .map_or_else(|| ActorId::new("per_owner"), |p| p.actor_id.clone());
        Self {
            owner: Some(owner),
            rbac: cfg.rbac.clone(),
            registered: cfg.persons.iter().map(|p| p.actor_id.clone()).collect(),
            guest_access: cfg.valve.guest_access,
        }
    }

    fn is_registered(&self, actor: &ActorId) -> bool {
        self.owner.as_ref() == Some(actor) || self.registered.contains(actor)
    }
}

/// The text of a percept payload: `{"text": ...}` JSON or plain UTF-8; `None` for media etc.
#[must_use]
pub fn percept_text(percept: &Percept) -> Option<String> {
    let bytes = percept.payload.as_bytes();
    match serde_json::from_slice::<Value>(bytes) {
        Ok(Value::Object(map)) => map.get("text").and_then(Value::as_str).map(str::to_owned),
        Ok(Value::String(s)) => Some(s),
        Ok(_) => None,
        Err(_) => String::from_utf8(bytes.to_vec()).ok(),
    }
}

fn command_name(text: &str) -> String {
    text.trim()
        .trim_start_matches('/')
        .split_whitespace()
        .next()
        .unwrap_or("")
        .split('@')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn whoami(ctx: &CommandContext, actor: &ActorId, place: &PlaceId) -> String {
    let is_owner = ctx.owner.as_ref() == Some(actor);
    let registered = ctx.is_registered(actor);
    let role = if is_owner {
        "owner".to_owned()
    } else {
        let roles: Vec<String> = ctx
            .rbac
            .as_ref()
            .map(|r| {
                r.effective_roles(actor)
                    .into_iter()
                    .map(|d| d.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        if roles.is_empty() {
            if registered { "member".to_owned() } else { "guest".to_owned() }
        } else {
            roles.join(", ")
        }
    };
    let may = agent_converse_permitted(
        ctx.rbac.as_ref(),
        ctx.owner.as_ref(),
        actor,
        Some(place),
        registered,
        ctx.guest_access,
    );
    format!(
        "account: {}\nrole: {role}\nregistered: {}\nuse the agent: {}",
        actor.as_str(),
        if registered { "yes" } else { "no (guest)" },
        if may { "yes" } else { "no (agent.converse not granted)" },
    )
}

fn link_text(ctx: &CommandContext, actor: &ActorId) -> String {
    if ctx.is_registered(actor) {
        "This account is already registered. The owner can manage accounts in /people.".to_owned()
    } else {
        format!(
            "To link this account to a person, send the owner this account id: {}\n\
             The owner adds it under /people (or runs `monoagens link`).",
            actor.as_str()
        )
    }
}

/// Handles `text` as a slash command from `percept.actor`. `Some(reply)` means the gateway
/// answers and the message never reaches the core; `None` leaves the message to the normal flow
/// (not a command, an unknown command or one a plugin owns).
#[must_use]
pub fn answer(
    registry: &CommandRegistry,
    ctx: &CommandContext,
    percept: &Percept,
    text: &str,
    capabilities: &[CapabilityId],
) -> Option<String> {
    if !text.trim_start().starts_with('/') {
        return None;
    }
    let owner = ctx.owner.clone().unwrap_or_else(|| ActorId::new("per_owner"));
    let outcome = registry.try_handle_slash_command(
        text,
        &percept.actor,
        &owner,
        Some(&percept.place),
        ctx.rbac.as_ref(),
        capabilities,
    )?;
    match outcome {
        CommandOutcome::Handled { reply_text } => Some(match command_name(text).as_str() {
            "whoami" => whoami(ctx, &percept.actor, &percept.place),
            "link" => link_text(ctx, &percept.actor),
            _ => reply_text,
        }),
        CommandOutcome::Denied { reason } => Some(reason),
        CommandOutcome::ForwardToPlugin { .. } => None,
    }
}

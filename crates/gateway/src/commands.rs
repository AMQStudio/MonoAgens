//! Gateway slash-command registry and RBAC-filtered `/help` execution (zero LLM calls).

use std::collections::BTreeMap;

use monoagens_protocol::{ActorId, BodyId, CapabilityId, PlaceId};
use monoagens_valve::{RbacConfig, RoleTier};
use serde::{Deserialize, Serialize};

/// Specification for a built-in or plugin-declared slash command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandSpec {
    pub name: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub usage: Option<String>,
    #[serde(default = "default_command_permission")]
    pub permission: String,
    #[serde(default)]
    pub base_tier: Option<RoleTier>,
    #[serde(default)]
    pub owner_plugin: Option<BodyId>,
}

fn default_command_permission() -> String {
    String::new()
}

/// Deterministic outcome of handling an inbound slash command in Gateway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandOutcome {
    Handled {
        reply_text: String,
    },
    Denied {
        reason: String,
    },
    ForwardToPlugin {
        body_id: BodyId,
        command: String,
        args: String,
    },
}

/// Gateway command registry indexing built-in commands and plugin commands from `discover._meta.commands`.
#[derive(Debug, Clone)]
pub struct CommandRegistry {
    commands: BTreeMap<String, CommandSpec>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRegistry {
    /// Creates a [`CommandRegistry`] pre-populated with built-in Gateway commands.
    #[must_use]
    pub fn new() -> Self {
        let mut reg = Self {
            commands: BTreeMap::new(),
        };
        for spec in [
            CommandSpec {
                name: "help".to_owned(),
                summary: "List available commands and capabilities".to_owned(),
                usage: Some("/help".to_owned()),
                permission: "command.help".to_owned(),
                base_tier: Some(RoleTier::Guest),
                owner_plugin: None,
            },
            CommandSpec {
                name: "whoami".to_owned(),
                summary: "Show resolved actor identity and RBAC tier".to_owned(),
                usage: Some("/whoami".to_owned()),
                permission: "identity.whoami".to_owned(),
                base_tier: Some(RoleTier::Guest),
                owner_plugin: None,
            },
            CommandSpec {
                name: "link".to_owned(),
                summary: "Show how to link this account to a person".to_owned(),
                usage: Some("/link".to_owned()),
                permission: "command.link".to_owned(),
                base_tier: Some(RoleTier::Guest),
                owner_plugin: None,
            },
            CommandSpec {
                name: "roles".to_owned(),
                summary: "Inspect configured RBAC roles and caller bindings".to_owned(),
                usage: Some("/roles".to_owned()),
                permission: "roles.inspect".to_owned(),
                base_tier: Some(RoleTier::Admin),
                owner_plugin: None,
            },
        ] {
            reg.commands.insert(spec.name.clone(), spec);
        }
        reg
    }

    /// Registers plugin-declared commands (from `body/discover` `_meta.commands`) owned by `body_id`.
    ///
    /// Built-in commands cannot be overwritten by plugins.
    pub fn register_plugin_commands(&mut self, body_id: &BodyId, commands: Vec<CommandSpec>) {
        for mut spec in commands {
            let normalized = normalize_command_name(&spec.name);
            if normalized.is_empty() {
                continue;
            }
            if let Some(existing) = self.commands.get(&normalized)
                && existing.owner_plugin.is_none()
            {
                // Never allow plugins to shadow built-in Gateway commands.
                continue;
            }
            spec.name = normalized.clone();
            if spec.permission.trim().is_empty() {
                spec.permission = format!("command.{normalized}");
            }
            spec.owner_plugin = Some(body_id.clone());
            self.commands.insert(normalized, spec);
        }
    }

    /// Unregisters all commands owned by `body_id` (e.g. on body disconnect).
    pub fn unregister_plugin_commands(&mut self, body_id: &BodyId) {
        self.commands
            .retain(|_, spec| spec.owner_plugin.as_ref() != Some(body_id));
    }

    /// Looks up a registered command by name (with or without leading `/`).
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&CommandSpec> {
        let normalized = normalize_command_name(name);
        self.commands.get(&normalized)
    }

    /// Returns all registered commands in deterministic alphabetical order.
    #[must_use]
    pub fn all_commands(&self) -> Vec<&CommandSpec> {
        self.commands.values().collect()
    }

    /// Evaluates whether `actor` is permitted to see and run `spec` at `place` under `rbac`.
    #[must_use]
    pub fn is_command_permitted(
        &self,
        spec: &CommandSpec,
        actor: &ActorId,
        owner: &ActorId,
        place: Option<&PlaceId>,
        rbac: Option<&RbacConfig>,
    ) -> bool {
        match rbac {
            None => actor == owner || spec.base_tier.is_some(),
            Some(cfg) => {
                if cfg.permission_bases.contains_key(&spec.permission) {
                    cfg.is_permitted(actor, owner, place, &spec.permission)
                } else {
                    let mut effective = cfg.clone();
                    effective
                        .permission_bases
                        .insert(spec.permission.clone(), spec.base_tier);
                    effective.is_permitted(actor, owner, place, &spec.permission)
                }
            }
        }
    }

    /// Renders RBAC-filtered `/help` output listing only permitted commands and capabilities.
    #[must_use]
    pub fn render_help(
        &self,
        actor: &ActorId,
        owner: &ActorId,
        place: Option<&PlaceId>,
        rbac: Option<&RbacConfig>,
        capabilities: &[CapabilityId],
    ) -> String {
        let permitted_cmds: Vec<&CommandSpec> = self
            .commands
            .values()
            .filter(|spec| self.is_command_permitted(spec, actor, owner, place, rbac))
            .collect();

        let permitted_caps: Vec<&CapabilityId> = capabilities
            .iter()
            .filter(|cap| match rbac {
                None => true,
                Some(cfg) => cfg.is_permitted(actor, owner, place, cap.as_str()),
            })
            .collect();

        let mut lines = Vec::new();
        lines.push("Available commands:".to_owned());
        for cmd in &permitted_cmds {
            let usage = cmd
                .usage
                .as_deref()
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map_or_else(|| format!("/{}", cmd.name), ToOwned::to_owned);
            if cmd.summary.trim().is_empty() {
                lines.push(format!("  {usage}"));
            } else {
                lines.push(format!("  {usage} — {}", cmd.summary));
            }
        }

        if !permitted_caps.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "Permitted capabilities ({}): {}",
                permitted_caps.len(),
                permitted_caps
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        lines.join("\n")
    }

    /// Attempts to handle a slash command directly in Gateway without invoking any LLM.
    ///
    /// Returns `None` when `text` is not a slash command or does not match a registered command.
    #[must_use]
    pub fn try_handle_slash_command(
        &self,
        text: &str,
        actor: &ActorId,
        owner: &ActorId,
        place: Option<&PlaceId>,
        rbac: Option<&RbacConfig>,
        capabilities: &[CapabilityId],
    ) -> Option<CommandOutcome> {
        let (cmd_name, args) = parse_slash_command(text)?;
        let spec = self.commands.get(&cmd_name)?;

        if !self.is_command_permitted(spec, actor, owner, place, rbac) {
            return Some(CommandOutcome::Denied {
                reason: format!("permission denied for '/{}'", spec.name),
            });
        }

        if let Some(body_id) = &spec.owner_plugin {
            return Some(CommandOutcome::ForwardToPlugin {
                body_id: body_id.clone(),
                command: spec.name.clone(),
                args,
            });
        }

        let reply_text = match spec.name.as_str() {
            "help" => self.render_help(actor, owner, place, rbac, capabilities),
            "whoami" => render_whoami(actor, owner, rbac),
            "roles" => render_roles(actor, owner, rbac),
            _ => format!("Executed /{}", spec.name),
        };

        Some(CommandOutcome::Handled { reply_text })
    }
}

fn normalize_command_name(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('/')
        .split('@')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

fn parse_slash_command(text: &str) -> Option<(String, String)> {
    let trimmed = text.trim();
    let rest = trimmed.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }
    let mut parts = rest.splitn(2, char::is_whitespace);
    let raw_cmd = parts.next()?.trim();
    if raw_cmd.is_empty() {
        return None;
    }
    let cmd_name = normalize_command_name(raw_cmd);
    if cmd_name.is_empty() {
        return None;
    }
    let args = parts.next().map(str::trim).unwrap_or("").to_owned();
    Some((cmd_name, args))
}

fn render_whoami(actor: &ActorId, owner: &ActorId, rbac: Option<&RbacConfig>) -> String {
    let is_owner = actor == owner;
    match rbac {
        None => format!(
            "actor: {}\nowner: {}",
            actor.as_str(),
            if is_owner { "yes" } else { "no" }
        ),
        Some(cfg) => {
            let tier = cfg.effective_tier(actor, owner);
            let roles: Vec<&str> = cfg
                .effective_roles(actor)
                .into_iter()
                .map(|r| r.name.as_str())
                .collect();
            let roles_str = if roles.is_empty() {
                "none".to_owned()
            } else {
                roles.join(", ")
            };
            format!(
                "actor: {}\nowner: {}\ntier: {}\nroles: {}",
                actor.as_str(),
                if is_owner { "yes" } else { "no" },
                tier.as_str(),
                roles_str
            )
        }
    }
}

fn render_roles(actor: &ActorId, owner: &ActorId, rbac: Option<&RbacConfig>) -> String {
    let Some(cfg) = rbac else {
        return "RBAC is not configured.".to_owned();
    };
    let mut lines = Vec::new();
    lines.push(format!(
        "Caller '{}' effective tier: {}",
        actor.as_str(),
        cfg.effective_tier(actor, owner).as_str()
    ));
    if let Some(fb) = &cfg.fallback_role {
        lines.push(format!("Fallback role: {fb}"));
    }
    if cfg.roles.is_empty() {
        lines.push("Configured roles: (none)".to_owned());
    } else {
        lines.push("Configured roles:".to_owned());
        for role in cfg.roles.values() {
            lines.push(format!("  - {} (base={})", role.name, role.base.as_str()));
        }
    }
    lines.join("\n")
}

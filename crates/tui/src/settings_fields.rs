//! Typed field editors of `/settings`: select options, number ranges, path rules, the
//! `config.apply` patches each edit produces, and the coverage tables (every configuration key
//! has a settings row, an editor, or a documented exclusion).

use std::path::Path;

use monoagens_gateway::{EmbedBackendKind, GatewayConfig};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::config::{
    IconMode, MessageLayout, MotionMode, ShimmerMode, SidebarMode, TuiPreferences,
};
use crate::list_editor::NumberSpec;
use crate::selector::SelectOption;
use crate::theme::ThemePreset;

/// Everything select options may depend on.
pub struct SelectCtx<'a> {
    pub prefs: &'a TuiPreferences,
    pub cfg: &'a GatewayConfig,
    /// `(id, label)` of the catalog providers.
    pub providers: &'a [(String, String)],
}

/// The result of one field edit.
#[derive(Debug, Clone, PartialEq)]
pub enum Edit {
    Toggle(bool),
    Select(String),
    Text(String),
    Number(String),
    Path(String),
    List(Vec<String>),
    /// `old` is needed to null out removed keys (a merge patch cannot replace a table).
    Map {
        old: Vec<(String, String)>,
        new: Vec<(String, String)>,
    },
}

/// One valve toggle: id suffix, label, what it switches.
pub const VALVE_TOGGLES: [(&str, &str); 8] = [
    (
        "cedar_abac",
        "Evaluates the Cedar policy rules (attribute-based access) on every action; off skips them.",
    ),
    (
        "line2_scope",
        "Keeps replies inside the audience of the message that caused them (Line 2 visibility boundary).",
    ),
    (
        "taint_blocking",
        "Quarantines content from untrusted sources at all three doors so it cannot trigger actions.",
    ),
    (
        "pending_approval",
        "Holds high-risk actions until the owner approves them instead of running them at once.",
    ),
    (
        "budget_guard",
        "Stops model calls once the daily token budget (Model tab) is used up.",
    ),
    (
        "chain_brake",
        "Stops a chain of self-triggered wakes after the limit set by max_chain_wakes.",
    ),
    (
        "quiet_hours",
        "Holds non-urgent outbound notifications during quiet hours (for example 23:00-07:00).",
    ),
    (
        "proactive_cap",
        "Caps unsolicited proactive notifications per day.",
    ),
];

/// Description of a valve toggle (C2).
#[must_use]
pub fn valve_toggle_description(key: &str) -> &'static str {
    VALVE_TOGGLES
        .iter()
        .find(|(k, _)| *k == key)
        .map_or("", |(_, d)| *d)
}

/// Human-friendly `Friendly Label (tech.id)` for a valve toggle key.
#[must_use]
pub fn valve_toggle_label(key: &str) -> crate::core::labels::FriendlyLabel {
    crate::core::labels::friendly_valve_toggle(key)
}

fn serde_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn cycle<T: Copy + PartialEq + Serialize>(
    start: T,
    next: impl Fn(T) -> T,
    label: impl Fn(T) -> &'static str,
) -> Vec<SelectOption> {
    let mut out = Vec::new();
    let mut cur = start;
    loop {
        out.push(SelectOption {
            value: serde_str(&cur),
            label: label(cur).to_owned(),
            description: String::new(),
        });
        cur = next(cur);
        if cur == start || out.len() > 16 {
            break;
        }
    }
    out
}

/// Options and the current value of an enum row, `None` when `id` is not an enum.
#[must_use]
pub fn select_spec(id: &str, ctx: &SelectCtx<'_>) -> Option<(Vec<SelectOption>, String)> {
    let p = ctx.prefs;
    Some(match id {
        "tui.theme" => (
            cycle(ThemePreset::Titanium, ThemePreset::next, ThemePreset::label),
            serde_str(&p.theme),
        ),
        "tui.message_layout" => (
            cycle(
                MessageLayout::Chat,
                MessageLayout::next,
                MessageLayout::label,
            ),
            serde_str(&p.message_layout),
        ),
        "tui.sidebar" => (
            cycle(SidebarMode::Auto, SidebarMode::next, SidebarMode::label),
            serde_str(&p.sidebar),
        ),
        "tui.motion" => (
            cycle(MotionMode::Full, MotionMode::next, MotionMode::label),
            serde_str(&p.motion),
        ),
        "tui.shimmer" => (
            cycle(ShimmerMode::Classic, ShimmerMode::next, ShimmerMode::label),
            serde_str(&p.shimmer),
        ),
        "tui.icons" => (
            cycle(IconMode::Symbols, IconMode::next, IconMode::label),
            serde_str(&p.icons),
        ),
        "tui.conv.notifications" => (
            cycle(
                crate::notify::NotifyMode::Off,
                crate::notify::NotifyMode::next,
                crate::notify::NotifyMode::label,
            ),
            serde_str(&p.conv.notifications),
        ),
        "embed.backend" => (
            [
                (EmbedBackendKind::None, "No pre-queue embedding"),
                (
                    EmbedBackendKind::Hash,
                    "Deterministic hash embedder (no model needed)",
                ),
                (
                    EmbedBackendKind::EmbeddingGemma2Http,
                    "EmbeddingGemma 2 sidecar over HTTP",
                ),
                (
                    EmbedBackendKind::EmbeddingGemma2Stdio,
                    "EmbeddingGemma 2 sidecar over stdio",
                ),
                (
                    EmbedBackendKind::EmbeddingGemma2Remote,
                    "Remote OpenAI-style embeddings of EmbeddingGemma 2",
                ),
                (
                    EmbedBackendKind::OpenAiCompatible,
                    "OpenAI-compatible embeddings endpoint",
                ),
            ]
            .iter()
            .map(|(k, d)| SelectOption::plain(k.as_str(), d))
            .collect(),
            ctx.cfg.embed.backend.as_str().to_owned(),
        ),
        "valve.mode" => (
            vec![
                SelectOption::plain(
                    "enforcing",
                    "Every rule applies; denied actions are blocked",
                ),
                SelectOption::plain("partial", "Rules apply but some holds are relaxed"),
                SelectOption::plain("off", "No enforcement (testing only)"),
            ],
            ctx.cfg.valve.mode.clone(),
        ),
        "model.output_limit_param" => (
            vec![
                SelectOption::plain("max_tokens", "Classic chat-completions parameter"),
                SelectOption::plain(
                    "max_completion_tokens",
                    "Newer parameter required by reasoning models",
                ),
            ],
            ctx.cfg.model.output_limit_param.clone(),
        ),
        _ => {
            if let Some(purpose) = id
                .strip_prefix("model.purposes.")
                .and_then(|r| r.strip_suffix(".provider"))
            {
                let mut opts = vec![SelectOption::plain("", "Use the main provider")];
                opts.extend(ctx.providers.iter().map(|(pid, label)| SelectOption {
                    value: pid.clone(),
                    label: label.clone(),
                    description: pid.clone(),
                }));
                let cur = ctx
                    .cfg
                    .model
                    .purposes
                    .get(purpose)
                    .and_then(|pm| pm.catalog_provider.clone())
                    .unwrap_or_default();
                return Some((opts, cur));
            }
            return None;
        }
    })
}

/// Allowed range of a number row, `None` when `id` is not a number.
#[must_use]
pub fn number_spec(id: &str) -> Option<NumberSpec> {
    Some(match id {
        "soul.seed" => NumberSpec::int(0, i64::MAX),
        "runtime.hook_timeout_ms" | "runtime.working_cue_ms" | "runtime.body_shutdown_grace_ms" => {
            NumberSpec::int(1, 60_000)
        }
        "runtime.action_timeout_ms" | "runtime.model_timeout_ms" | "runtime.progress_note_ms" => {
            NumberSpec::int(1, 600_000)
        }
        "model.daily_token_budget" => NumberSpec::int(1, i64::MAX),
        "model.max_output_tokens" => NumberSpec::int(1, 10_000_000),
        "model.temperature" => NumberSpec::float(0.0, 2.0),
        "model.output_limits.reason"
        | "model.output_limits.classify"
        | "model.output_limits.consolidate" => NumberSpec::int(1, 1_000_000),
        "valve.max_chain_wakes" => NumberSpec::int(1, 1_000),
        "embed.dim_d" => NumberSpec::int(1, 8_192),
        "embed.timeout_ms" => NumberSpec::int(1, 600_000),
        "identity.link_challenge_ttl_s" => NumberSpec::int(1, 86_400),
        "identity.link_max_attempts" => NumberSpec::int(1, 20),
        "tui.autocomplete_limit" => NumberSpec::int(1, 10),
        _ => return None,
    })
}

/// Path rows: `Some(true)` when the path must exist, `Some(false)` when only its parent must.
#[must_use]
pub fn path_rule(id: &str) -> Option<bool> {
    match id {
        "soul.core_config" | "soul.model_path" | "embed.model_path" => Some(true),
        "data_dir" => Some(false),
        _ => None,
    }
}

/// Optional text/path rows: an empty value removes the key.
#[must_use]
pub fn is_optional(id: &str) -> bool {
    matches!(
        id,
        "data_dir"
            | "soul.core_config"
            | "soul.model_path"
            | "soul.expected_blake3"
            | "embed.model_path"
            | "embed.endpoint_url"
            | "embed.program"
            | "embed.remote_model"
            | "model.provider_options.base_url"
            | "model.usage_paths.cost"
            | "model.max_output_tokens"
            | "model.temperature"
    ) || (id.starts_with("model.purposes.") && id.ends_with(".provider"))
}

/// Builds `{"a":{"b":value}}` from `["a","b"]`.
#[must_use]
pub fn nested(path: &[&str], value: Value) -> Value {
    path.iter().rev().fold(value, |acc, seg| {
        let mut m = Map::new();
        m.insert((*seg).to_owned(), acc);
        Value::Object(m)
    })
}

/// Checks a path against `must_exist`.
pub fn check_path(raw: &str, must_exist: bool) -> Result<(), String> {
    let p = Path::new(raw);
    if must_exist {
        if !p.exists() {
            return Err(format!("path does not exist: {raw}"));
        }
    } else if let Some(parent) = p.parent().filter(|d| !d.as_os_str().is_empty())
        && !parent.exists()
    {
        return Err(format!(
            "parent directory does not exist: {}",
            parent.display()
        ));
    }
    Ok(())
}

fn num_json(spec: NumberSpec, v: f64) -> Value {
    if spec.integer {
        json!(v as i64)
    } else {
        // Two decimals are plenty for sampling parameters and avoid f32 noise.
        json!((v * 100.0).round() / 100.0)
    }
}

/// The `config.apply` patch for editing config row `id` (a gateway key, not a `tui.*` key).
pub fn config_patch(id: &str, edit: &Edit) -> Result<Value, String> {
    let path: Vec<&str> = id.split('.').collect();
    let value = match edit {
        Edit::Toggle(b) => json!(b),
        Edit::Select(v) => {
            if v.is_empty() && is_optional(id) {
                Value::Null
            } else {
                json!(v)
            }
        }
        Edit::Text(t) | Edit::Path(t) => {
            let t = t.trim();
            if let Edit::Path(_) = edit
                && !t.is_empty()
                && let Some(must) = path_rule(id)
            {
                check_path(t, must)?;
            }
            if t.is_empty() {
                if is_optional(id) {
                    Value::Null
                } else {
                    return Err("value must not be empty".to_owned());
                }
            } else {
                json!(t)
            }
        }
        Edit::Number(raw) => {
            if raw.trim().is_empty() && is_optional(id) {
                Value::Null
            } else {
                let spec =
                    number_spec(id).ok_or_else(|| format!("'{id}' is not a number field"))?;
                num_json(spec, spec.validate(raw)?)
            }
        }
        Edit::List(items) => json!(items),
        Edit::Map { old, new } => return map_patch(id, old, new),
    };
    Ok(nested(&path, value))
}

fn map_patch(
    id: &str,
    old: &[(String, String)],
    new: &[(String, String)],
) -> Result<Value, String> {
    let mut table = Map::new();
    for (k, _) in old {
        if !new.iter().any(|(nk, _)| nk == k) {
            table.insert(k.clone(), Value::Null);
        }
    }
    for (k, v) in new {
        let val = match id {
            "model.prices" => {
                let (i, o) = v.split_once(',').ok_or_else(|| {
                    format!("price for '{k}' must be 'input,output' (USD per million tokens)")
                })?;
                let parse = |s: &str| {
                    s.trim()
                        .parse::<f64>()
                        .ok()
                        .filter(|n| n.is_finite() && *n >= 0.0)
                        .ok_or_else(|| {
                            format!("price '{}' for '{k}' must be a number >= 0", s.trim())
                        })
                };
                json!({ "input_per_mtok": parse(i)?, "output_per_mtok": parse(o)? })
            }
            _ => json!(v),
        };
        table.insert(k.clone(), val);
    }
    let path: Vec<&str> = id.split('.').collect();
    Ok(nested(&path, Value::Object(table)))
}

/// Applies a `tui.*` edit to a copy of `prefs` through its serde form (so the editor can never
/// write a value `tui.toml` would refuse to read).
pub fn apply_pref(prefs: &TuiPreferences, id: &str, edit: &Edit) -> Result<TuiPreferences, String> {
    let key = id
        .strip_prefix("tui.")
        .ok_or_else(|| format!("'{id}' is not a tui.toml key"))?;
    let mut table = toml::Table::try_from(prefs).map_err(|e| e.to_string())?;
    let new = match edit {
        Edit::Toggle(b) => toml::Value::Boolean(*b),
        Edit::Select(s) | Edit::Text(s) => toml::Value::String(s.clone()),
        Edit::Number(raw) => {
            let spec = number_spec(id).ok_or_else(|| format!("'{id}' is not a number field"))?;
            toml::Value::Integer(spec.validate(raw)? as i64)
        }
        _ => return Err(format!("'{id}' cannot take this kind of edit")),
    };
    // `tui.conv.<key>` lives in the `[conv]` table of tui.toml.
    match key.split_once('.') {
        Some((section, leaf)) => {
            let sub = table
                .entry(section.to_owned())
                .or_insert_with(|| toml::Value::Table(toml::Table::new()))
                .as_table_mut()
                .ok_or_else(|| format!("'{id}' is not a tui.toml key"))?;
            sub.insert(leaf.to_owned(), new);
        }
        None => {
            table.insert(key.to_owned(), new);
        }
    }
    toml::Value::Table(table)
        .try_into::<TuiPreferences>()
        .map_err(|e| format!("invalid value for {id}: {e}"))
}

/// Keys of `tui.toml`, from the serialised defaults (the `[conv]` table is flattened to
/// `tui.conv.<key>`).
#[must_use]
pub fn tui_keys() -> Vec<String> {
    let Ok(table) = toml::Table::try_from(TuiPreferences::default()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, v) in &table {
        match v.as_table() {
            Some(sub) => out.extend(sub.keys().map(|sk| format!("tui.{k}.{sk}"))),
            None => out.push(format!("tui.{k}")),
        }
    }
    out
}

/// Configuration keys that deliberately have no settings row, each with the reason.
pub const COVERAGE_EXCLUSIONS: &[(&str, &str)] = &[
    (
        "version",
        "schema version; changing it would make the file unreadable",
    ),
    (
        "bodies",
        "legacy [[bodies]] alias of [plugins.<id>]; plugins are edited on the Plugins tab",
    ),
    (
        "identity.heuristic_merge_enabled",
        "invariant-locked to false (identity merges are never heuristic)",
    ),
    (
        "identity.owner_escalation_guard",
        "invariant-locked to true (Absolute Line 1)",
    ),
    (
        "person.id",
        "alias of person.actor_id; the RBAC editor writes actor_id",
    ),
    (
        "person.is_owner",
        "alias of person.owner; the owner flag is not editable here (invariant: owner identity)",
    ),
    (
        "attachment_master_key",
        "generated key; rotating it orphans stored attachments (set MONOAGENS_ATTACHMENT_MASTER_KEY)",
    ),
    (
        "attestation_key",
        "generated key; rotating it invalidates pending approvals (set MONOAGENS_APPROVAL_ATTESTATION_KEY)",
    ),
];

/// Keys edited by a dedicated editor instead of a flat row: key pattern -> editor.
pub const EDITOR_COVERAGE: &[(&str, &str)] = &[
    ("plugins.<id>.enabled", "plugin detail"),
    ("plugins.<id>.kind", "plugin detail"),
    ("plugins.<id>.transport", "plugin detail"),
    ("plugins.<id>.command", "plugin detail"),
    ("plugins.<id>.args", "plugin detail"),
    ("plugins.<id>.socket_path", "plugin detail"),
    ("plugins.<id>.dependencies", "plugin detail"),
    ("plugins.<id>.optional_dependencies", "plugin detail"),
    ("person.owner", "people editor (/people)"),
    ("person.accounts", "people editor (/people)"),
    ("person.body_id", "people editor (/people)"),
    ("person.account_id", "people editor (/people)"),
    ("person.actor_id", "rbac actors"),
    ("person.roles", "rbac actors"),
    ("roles.fallback", "rbac fallback"),
    ("roles.<name>.base", "rbac role page"),
    ("roles.<name>.allow", "rbac role page"),
    ("roles.<name>.deny", "rbac role page"),
    ("roles.<name>.approval", "rbac role page"),
    ("roles.<name>.label", "rbac role page"),
    ("roles.<name>.context", "rbac role page"),
    ("roles.<name>.settings.<key>", "rbac role page"),
    ("places.<place>.allow", "rbac places"),
    ("places.<place>.deny", "rbac places"),
    ("permission_bases.<capability>", "rbac permission bases"),
    (
        "model.extra_headers.<name>",
        "model tab map row model.extra_headers",
    ),
    (
        "model.prices.<id>.input_per_mtok",
        "model tab map row model.prices",
    ),
    (
        "model.prices.<id>.output_per_mtok",
        "model tab map row model.prices",
    ),
    ("env.<NAME>", "providers tab [env] secret rows"),
    (
        "providers.enabled",
        "provider page: enable a local provider",
    ),
    (
        "providers.custom.<name>.base_url",
        "provider page: Add custom provider",
    ),
    (
        "providers.custom.<name>.dialect",
        "provider page: Add custom provider",
    ),
    (
        "providers.custom.<name>.keyless",
        "provider page: Add custom provider",
    ),
    (
        "providers.<id>.api_key",
        "providers tab provider page and /provider",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_patch_shapes() {
        assert_eq!(
            config_patch("valve.cedar_abac", &Edit::Toggle(false)).unwrap(),
            json!({"valve": {"cedar_abac": false}})
        );
        assert_eq!(
            config_patch(
                "model.output_limit_param",
                &Edit::Select("max_completion_tokens".into())
            )
            .unwrap(),
            json!({"model": {"output_limit_param": "max_completion_tokens"}})
        );
        assert_eq!(
            config_patch("model.temperature", &Edit::Number("0.755".into())).unwrap(),
            json!({"model": {"temperature": 0.76}})
        );
        assert!(
            config_patch("runtime.hook_timeout_ms", &Edit::Number("70000".into()))
                .unwrap_err()
                .contains("1..=60000")
        );
        assert_eq!(
            config_patch("embed.args", &Edit::List(vec!["--a".into(), "--b".into()])).unwrap(),
            json!({"embed": {"args": ["--a", "--b"]}})
        );
        assert_eq!(
            config_patch("embed.endpoint_url", &Edit::Text(String::new())).unwrap(),
            json!({"embed": {"endpoint_url": null}})
        );
        assert!(config_patch("soul.name", &Edit::Text(String::new())).is_err());
    }

    #[test]
    fn map_patch_nulls_removed_keys() {
        let old = vec![
            ("A".to_owned(), "1".to_owned()),
            ("B".to_owned(), "2".to_owned()),
        ];
        let new = vec![("B".to_owned(), "3".to_owned())];
        assert_eq!(
            config_patch("model.extra_headers", &Edit::Map { old, new }).unwrap(),
            json!({"model": {"extra_headers": {"A": null, "B": "3"}}})
        );
        let price = config_patch(
            "model.prices",
            &Edit::Map {
                old: vec![],
                new: vec![("groq/x".into(), "0.5, 1.5".into())],
            },
        )
        .unwrap();
        assert_eq!(
            price["model"]["prices"]["groq/x"]["output_per_mtok"],
            json!(1.5)
        );
        assert!(
            config_patch(
                "model.prices",
                &Edit::Map {
                    old: vec![],
                    new: vec![("k".into(), "1".into())]
                }
            )
            .is_err()
        );
    }

    #[test]
    fn path_rule_checks_existence() {
        assert!(
            config_patch(
                "soul.model_path",
                &Edit::Path("/definitely/not/here.safetensors".into())
            )
            .is_err()
        );
        assert!(config_patch("soul.model_path", &Edit::Path(String::new())).is_ok());
    }

    #[test]
    fn prefs_roundtrip_through_serde() {
        let p = TuiPreferences::default();
        let q = apply_pref(&p, "tui.theme", &Edit::Select("slate".into())).unwrap();
        assert_eq!(q.theme, ThemePreset::Slate);
        assert!(apply_pref(&p, "tui.theme", &Edit::Select("nope".into())).is_err());
        let q = apply_pref(&p, "tui.autocomplete_limit", &Edit::Number("4".into())).unwrap();
        assert_eq!(q.autocomplete_limit, 4);
        assert!(apply_pref(&p, "tui.autocomplete_limit", &Edit::Number("40".into())).is_err());
    }
}

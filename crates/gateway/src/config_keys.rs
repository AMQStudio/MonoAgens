//! Hand-kept inventory of every key `monoagens.toml` can parse, used by the TUI settings
//! coverage test (`crates/tui/tests/settings_complete_tests.rs`): each key needs a settings
//! row, an editor, or a documented exclusion there.
//!
//! The list is checked against the parser in `config.rs` by [`tests::parser_keys_are_listed`],
//! which fails when a key is read by the parser but missing here. A segment in angle brackets
//! (`<id>`) stands for a user-chosen name.

/// Every dotted key of `monoagens.toml` (`[[person]]` and `[[bodies]]` entries use `person.*`
/// and `bodies.*`).
pub const CONFIG_KEYS: &[&str] = &[
    "version",
    "data_dir",
    "soul.name",
    "soul.seed",
    "soul.core_config",
    "soul.model_path",
    "soul.expected_blake3",
    "soul.online_adaptation",
    "runtime.hook_timeout_ms",
    "runtime.action_timeout_ms",
    "runtime.model_timeout_ms",
    "runtime.working_cue_ms",
    "runtime.progress_note_ms",
    "runtime.body_shutdown_grace_ms",
    "model.provider",
    "providers.enabled",
    "providers.custom.<name>.base_url",
    "providers.custom.<name>.dialect",
    "providers.custom.<name>.keyless",
    "model.provider_options.base_url",
    "model.purposes.<purpose>.model",
    "model.purposes.<purpose>.provider",
    "model.prices.<id>.input_per_mtok",
    "model.prices.<id>.output_per_mtok",
    "model.model",
    "model.daily_token_budget",
    "model.max_output_tokens",
    "model.temperature",
    "model.output_limit_param",
    "model.output_limits.reason",
    "model.output_limits.classify",
    "model.output_limits.consolidate",
    "model.extra_headers.<name>",
    "model.usage_paths.input_tokens",
    "model.usage_paths.output_tokens",
    "model.usage_paths.cached_tokens",
    "model.usage_paths.reasoning_tokens",
    "model.usage_paths.total_tokens",
    "model.usage_paths.cost",
    "valve.mode",
    "valve.max_chain_wakes",
    "valve.guest_access",
    "embed.backend",
    "embed.model_ver",
    "embed.dim_d",
    "embed.timeout_ms",
    "embed.model_path",
    "embed.endpoint_url",
    "embed.program",
    "embed.args",
    "embed.remote_model",
    "identity.heuristic_merge_enabled",
    "identity.link_challenge_ttl_s",
    "identity.link_max_attempts",
    "identity.retroactive_merge",
    "identity.owner_escalation_guard",
    "plugins.<id>.enabled",
    "plugins.<id>.kind",
    "plugins.<id>.transport",
    "plugins.<id>.command",
    "plugins.<id>.args",
    "plugins.<id>.socket_path",
    "plugins.<id>.dependencies",
    "plugins.<id>.optional_dependencies",
    "bodies",
    "person.actor_id",
    "person.id",
    "person.owner",
    "person.is_owner",
    "person.body_id",
    "person.account_id",
    "person.accounts",
    "person.roles",
    "roles.fallback",
    "roles.<name>.base",
    "roles.<name>.allow",
    "roles.<name>.deny",
    "roles.<name>.approval",
    "roles.<name>.label",
    "roles.<name>.context",
    "roles.<name>.settings.<key>",
    "places.<place>.allow",
    "places.<place>.deny",
    "permission_bases.<capability>",
];

/// Keys of `secrets.toml` (never in `monoagens.toml`).
pub const SECRET_KEYS: &[&str] = &[
    "embed_api_key",
    "attachment_master_key",
    "attestation_key",
    "env.<NAME>",
    "providers.<id>.api_key",
];

#[cfg(test)]
mod tests {
    use super::CONFIG_KEYS;
    use std::collections::BTreeSet;

    /// Leaf names the parser asks for: `get_opt_*(tbl, "leaf", ..)`, `get_required_*`, and
    /// `root.get("section")`.
    fn parser_leaves() -> BTreeSet<String> {
        let src: String = include_str!("config.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let mut out = BTreeSet::new();
        for marker in ["get_opt_", "get_required_"] {
            for (at, _) in src.match_indices(marker) {
                let rest = &src[at + marker.len()..];
                let Some(open) = rest.find('(') else { continue };
                if rest[..open].chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_')) {
                    continue;
                }
                let args = &rest[open + 1..];
                let Some(comma) = args.find(',') else { continue };
                let after = &args[comma + 1..];
                if let Some(lit) = after.strip_prefix('"')
                    && let Some(end) = lit.find('"')
                {
                    out.insert(lit[..end].to_owned());
                }
            }
        }
        for (at, _) in src.match_indices("root.get(\"") {
            let lit = &src[at + "root.get(\"".len()..];
            if let Some(end) = lit.find('"') {
                out.insert(lit[..end].to_owned());
            }
        }
        out
    }

    fn listed_leaves() -> BTreeSet<String> {
        CONFIG_KEYS
            .iter()
            .flat_map(|k| k.split('.'))
            .filter(|seg| !seg.starts_with('<'))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn parser_keys_are_listed() {
        let listed = listed_leaves();
        let missing: Vec<String> = parser_leaves().difference(&listed).cloned().collect();
        assert!(
            missing.is_empty(),
            "keys read by config.rs but missing from CONFIG_KEYS (add them, then give the TUI a settings row): {missing:?}"
        );
    }

    #[test]
    fn scanner_finds_the_known_keys() {
        let leaves = parser_leaves();
        for k in ["seed", "hook_timeout_ms", "body_shutdown_grace_ms", "fallback", "permission_bases", "person"] {
            assert!(leaves.contains(k), "scanner missed {k}: {leaves:?}");
        }
    }

    #[test]
    fn permission_bases_parse_tiers_and_owner_only() {
        use crate::config::parse_config_str;
        use std::path::Path;
        let cfg = parse_config_str(
            "version = 1\n[permission_bases]\n\"tool.shell\" = \"owner-only\"\n\"chat.speak\" = \"Member\"\n",
            Path::new("m.toml"),
            Path::new("s.toml"),
        )
        .expect("parses");
        let bases = cfg.rbac.expect("rbac").permission_bases;
        assert_eq!(bases["tool.shell"], None);
        assert_eq!(bases["chat.speak"], Some(monoagens_valve::rbac::RoleTier::Member));
        let bad = parse_config_str(
            "version = 1\n[permission_bases]\n\"x\" = \"king\"\n",
            Path::new("m.toml"),
            Path::new("s.toml"),
        );
        assert!(bad.is_err());
    }
}

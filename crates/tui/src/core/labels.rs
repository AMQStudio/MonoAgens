//! Human-friendly label formatting for capabilities, plugins, Valve toggles, and settings.

use std::fmt;

/// Paired human-readable display label and canonical identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FriendlyLabel {
    pub label: String,
    pub id: String,
}

impl FriendlyLabel {
    /// Returns the short human-readable label.
    #[must_use]
    pub fn short_label(&self) -> &str {
        &self.label
    }

    /// Formats `"{label} ({id})"` for inline display or search.
    #[must_use]
    pub fn format_inline(&self) -> String {
        format!("{self}")
    }
}

impl fmt::Display for FriendlyLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.label, self.id)
    }
}

/// Resolves a capability identifier to a [`FriendlyLabel`] via `monoagens_gateway`.
#[must_use]
pub fn friendly_capability(id: &str) -> FriendlyLabel {
    FriendlyLabel {
        label: monoagens_gateway::friendly_capability_label(id),
        id: id.to_owned(),
    }
}

/// Resolves a plugin identifier to a [`FriendlyLabel`].
#[must_use]
pub fn friendly_plugin(id: &str) -> FriendlyLabel {
    let label = match id {
        "whatsapp" => "WhatsApp".to_owned(),
        "telegram" => "Telegram".to_owned(),
        "discord" => "Discord".to_owned(),
        "slack" => "Slack".to_owned(),
        "workspace" => "Workspace".to_owned(),
        "web-fetch" => "Web Fetch".to_owned(),
        "pdf-reader" => "PDF Reader".to_owned(),
        "soul-tuning" => "Soul Tuning".to_owned(),
        _ => humanize_words(id),
    };
    FriendlyLabel {
        label,
        id: id.to_owned(),
    }
}

/// Resolves a Valve toggle key to a curated [`FriendlyLabel`], falling back to Title Case.
#[must_use]
pub fn friendly_valve_toggle(key: &str) -> FriendlyLabel {
    let label = match key {
        "valve.guest_access" => "Allow Guests".to_owned(),
        "cedar_abac" => "Cedar Access Rules".to_owned(),
        "line2_scope" => "Privacy Boundary".to_owned(),
        "taint_blocking" => "Untrusted Input Shield".to_owned(),
        "pending_approval" => "Require Approval for Risky Actions".to_owned(),
        "budget_guard" => "Daily Token Budget Guard".to_owned(),
        "chain_brake" => "Auto-Wake Chain Brake".to_owned(),
        "quiet_hours" => "Quiet Hours".to_owned(),
        "proactive_cap" => "Daily Proactive Cap".to_owned(),
        _ => humanize_words(key),
    };
    FriendlyLabel {
        label,
        id: key.to_owned(),
    }
}

/// Resolves a setting key to a [`FriendlyLabel`], using `fallback` when non-empty or
/// Title-Case humanization of `key` otherwise.
#[must_use]
pub fn friendly_setting(key: &str, fallback: &str) -> FriendlyLabel {
    let trimmed = fallback.trim();
    let label = if trimmed.is_empty() {
        humanize_words(key)
    } else {
        trimmed.to_owned()
    };
    FriendlyLabel {
        label,
        id: key.to_owned(),
    }
}

/// Returns `true` when `query` matches `label`, `id`, or `"{label} ({id})"` case-insensitively.
#[must_use]
pub fn matches_friendly_query(query: &str, label: &str, id: &str) -> bool {
    let q = query.trim();
    if q.is_empty() {
        return true;
    }
    let q_lower = q.to_lowercase();
    let inline = format!("{label} ({id})");
    label.to_lowercase().contains(&q_lower)
        || id.to_lowercase().contains(&q_lower)
        || inline.to_lowercase().contains(&q_lower)
}

fn humanize_words(raw: &str) -> String {
    let words: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(title_case_word)
        .collect();
    if words.is_empty() {
        raw.trim().to_owned()
    } else {
        words.join(" ")
    }
}

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut out = String::with_capacity(word.len());
    out.extend(first.to_uppercase());
    out.extend(chars.flat_map(char::to_lowercase));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curated_valve_toggles_and_fallback() {
        let cases = [
            ("valve.guest_access", "Allow Guests"),
            ("cedar_abac", "Cedar Access Rules"),
            ("line2_scope", "Privacy Boundary"),
            ("taint_blocking", "Untrusted Input Shield"),
            ("pending_approval", "Require Approval for Risky Actions"),
            ("budget_guard", "Daily Token Budget Guard"),
            ("chain_brake", "Auto-Wake Chain Brake"),
            ("quiet_hours", "Quiet Hours"),
            ("proactive_cap", "Daily Proactive Cap"),
        ];
        for (key, expected) in cases {
            let fl = friendly_valve_toggle(key);
            assert_eq!(fl.short_label(), expected);
            assert_eq!(fl.id, key);
            assert_eq!(fl.format_inline(), format!("{expected} ({key})"));
        }

        let unknown = friendly_valve_toggle("custom_rate_limit");
        assert_eq!(unknown.short_label(), "Custom Rate Limit");
        assert_eq!(unknown.id, "custom_rate_limit");
    }

    #[test]
    fn capability_plugin_and_setting_labels() {
        let cap = friendly_capability("chat.speak");
        assert_eq!(cap.short_label(), "Send Messages");
        assert_eq!(cap.id, "chat.speak");

        let plugin = friendly_plugin("whatsapp");
        assert_eq!(plugin.short_label(), "WhatsApp");
        assert_eq!(
            friendly_plugin("custom-bridge").short_label(),
            "Custom Bridge"
        );

        let setting = friendly_setting("ui.theme_preset", "Theme Preset");
        assert_eq!(setting.short_label(), "Theme Preset");
        assert_eq!(
            friendly_setting("ui.custom_flag", "").short_label(),
            "Ui Custom Flag"
        );
    }

    #[test]
    fn query_matches_friendly_label_and_canonical_id() {
        assert!(matches_friendly_query(
            "",
            "Cedar Access Rules",
            "cedar_abac"
        ));
        assert!(matches_friendly_query(
            "cedar",
            "Cedar Access Rules",
            "cedar_abac"
        ));
        assert!(matches_friendly_query(
            "abac",
            "Cedar Access Rules",
            "cedar_abac"
        ));
        assert!(matches_friendly_query(
            "rules (cedar",
            "Cedar Access Rules",
            "cedar_abac"
        ));
        assert!(!matches_friendly_query(
            "whatsapp",
            "Cedar Access Rules",
            "cedar_abac"
        ));
    }
}

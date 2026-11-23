//! Self-managed plugin configuration and state persistence for `plugin-soul-tuning`.
//!
//! Reads `MONOAGENS_PLUGIN_CONFIG` (auto-generating a commented default `soul-tuning.toml`
//! on first run only when the env var is set and non-empty) and persists `active_mode.json`
//! atomically inside `MONOAGENS_PLUGIN_STATE_DIR` (falling back to `MONOAGENS_BODY_STATE_DIR`).

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use monoagens_body_kit::load_state_dir_from_env;
use serde::{Deserialize, Serialize};

/// Environment variable pointing to the plugin's self-managed TOML config file.
pub const ENV_PLUGIN_CONFIG: &str = "MONOAGENS_PLUGIN_CONFIG";

/// Environment variable pointing to the plugin's self-managed state directory.
pub const ENV_PLUGIN_STATE_DIR: &str = "MONOAGENS_PLUGIN_STATE_DIR";

/// Current schema version for `soul-tuning.toml`.
pub const SOUL_TUNING_CONFIG_VERSION: u32 = 1;

/// Default active speech mode name.
pub const DEFAULT_MODE_NAME: &str = "daily";

/// Minimum allowed `talkativeness_offset` (`[-0.2, 0.2]`).
pub const MIN_TALKATIVENESS_OFFSET: f32 = -0.2;

/// Maximum allowed `talkativeness_offset` (`[-0.2, 0.2]`).
pub const MAX_TALKATIVENESS_OFFSET: f32 = 0.2;

/// Maximum number of exchanges (2 turns per exchange = 10 turns).
pub const MAX_EXCHANGES: usize = 5;

/// Maximum number of example turns per mode (`MAX_EXCHANGES * 2`).
pub const MAX_EXAMPLE_TURNS: usize = MAX_EXCHANGES * 2;

/// Filename used to persist the active mode inside the plugin state directory.
pub const ACTIVE_MODE_FILENAME: &str = "active_mode.json";

/// Specification of a single speech mode (`rule`, clamped `talkativeness_offset`, bounded `examples`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoulModeSpec {
    pub rule: String,
    #[serde(default)]
    pub talkativeness_offset: f32,
    #[serde(default)]
    pub examples: Vec<[String; 2]>,
}

impl SoulModeSpec {
    /// Validates and normalizes this mode spec: ensures non-empty rule, finite and clamped
    /// `talkativeness_offset` in `[-0.2, 0.2]`, and at most `MAX_EXAMPLE_TURNS` (10) turns.
    pub fn validated(mut self, mode_name: &str) -> Result<Self, String> {
        if self.rule.trim().is_empty() {
            return Err(format!("soul-tuning mode '{mode_name}' rule must not be empty"));
        }
        if !self.talkativeness_offset.is_finite() {
            return Err(format!(
                "soul-tuning mode '{mode_name}' talkativeness_offset must be finite"
            ));
        }
        self.talkativeness_offset = clamp_talkativeness_offset(self.talkativeness_offset);
        if self.examples.len() > MAX_EXAMPLE_TURNS {
            return Err(format!(
                "soul-tuning mode '{mode_name}' has {} example turns, exceeding max {MAX_EXAMPLE_TURNS} ({MAX_EXCHANGES} exchanges)",
                self.examples.len()
            ));
        }
        for (idx, [speaker, _text]) in self.examples.iter().enumerate() {
            if speaker.trim().is_empty() {
                return Err(format!(
                    "soul-tuning mode '{mode_name}' example turn {idx} has empty speaker"
                ));
            }
        }
        Ok(self)
    }
}

/// Clamps `offset` strictly into `[-0.2, 0.2]`, returning `0.0` for non-finite inputs.
#[must_use]
pub fn clamp_talkativeness_offset(offset: f32) -> f32 {
    if !offset.is_finite() {
        return 0.0;
    }
    offset.clamp(MIN_TALKATIVENESS_OFFSET, MAX_TALKATIVENESS_OFFSET)
}

/// Self-managed configuration loaded from `soul-tuning.toml` (`MONOAGENS_PLUGIN_CONFIG`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoulTuningConfig {
    pub version: u32,
    pub default_mode: String,
    pub modes: BTreeMap<String, SoulModeSpec>,
}

impl Default for SoulTuningConfig {
    fn default() -> Self {
        Self {
            version: SOUL_TUNING_CONFIG_VERSION,
            default_mode: DEFAULT_MODE_NAME.to_owned(),
            modes: default_modes(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct RawSoulTuningConfig {
    version: Option<u32>,
    default_mode: Option<String>,
    modes: Option<BTreeMap<String, SoulModeSpec>>,
}

#[derive(Serialize)]
struct ModesTomlView<'a> {
    modes: &'a BTreeMap<String, SoulModeSpec>,
}

impl SoulTuningConfig {
    /// Loads configuration from `MONOAGENS_PLUGIN_CONFIG` if set and non-empty.
    ///
    /// - When `MONOAGENS_PLUGIN_CONFIG` is unset or empty, returns [`Self::default()`] without
    ///   creating any files on disk (safe for conformance runs).
    /// - When set to a non-existent path, creates parent directories, writes a commented
    ///   default `soul-tuning.toml`, and returns [`Self::default()`].
    /// - When the file exists, parses TOML, clamps offsets to `[-0.2, 0.2]`, and validates examples.
    pub fn load_or_init_from_env() -> Result<Self, String> {
        let Some(raw) = env::var_os(ENV_PLUGIN_CONFIG) else {
            return Ok(Self::default());
        };
        let path_str = raw.to_string_lossy();
        let trimmed = path_str.trim();
        if trimmed.is_empty() {
            return Ok(Self::default());
        }
        Self::load_or_init_from_path(Path::new(trimmed))
    }

    /// Loads configuration from `path`, creating a commented default file if it does not exist.
    pub fn load_or_init_from_path(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "failed to create plugin config parent directory '{}': {e}",
                        parent.display()
                    )
                })?;
            }
            let defaults = Self::default();
            fs::write(path, defaults.to_toml_string()).map_err(|e| {
                format!(
                    "failed to write default soul-tuning config '{}': {e}",
                    path.display()
                )
            })?;
            return Ok(defaults);
        }

        let content = fs::read_to_string(path).map_err(|e| {
            format!(
                "failed to read soul-tuning config '{}': {e}",
                path.display()
            )
        })?;
        Self::from_toml_str(&content).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Parses a `soul-tuning.toml` document, merging custom/overridden modes onto defaults,
    /// clamping `talkativeness_offset` strictly to `[-0.2, 0.2]`, and validating examples.
    pub fn from_toml_str(input: &str) -> Result<Self, String> {
        let raw: RawSoulTuningConfig =
            toml::from_str(input).map_err(|e| format!("invalid TOML: {e}"))?;

        let version = raw.version.unwrap_or(SOUL_TUNING_CONFIG_VERSION);
        if version == 0 || version > SOUL_TUNING_CONFIG_VERSION {
            return Err(format!(
                "unsupported config version {version} (expected {SOUL_TUNING_CONFIG_VERSION})"
            ));
        }

        let default_mode = raw
            .default_mode
            .unwrap_or_else(|| DEFAULT_MODE_NAME.to_owned());
        if default_mode.trim().is_empty() {
            return Err("default_mode must not be empty".to_owned());
        }

        let mut modes = default_modes();
        if let Some(configured_modes) = raw.modes {
            for (name, spec) in configured_modes {
                if name.trim().is_empty() {
                    return Err("mode name must not be empty".to_owned());
                }
                let validated = spec.validated(&name)?;
                modes.insert(name, validated);
            }
        }

        if !modes.contains_key(&default_mode) {
            return Err(format!(
                "default_mode '{default_mode}' is not defined in modes"
            ));
        }

        Ok(Self {
            version,
            default_mode,
            modes,
        })
    }

    /// Renders a clean, well-commented `soul-tuning.toml` document.
    #[must_use]
    pub fn to_toml_string(&self) -> String {
        let header = format!(
            "# MonoAgens Soul-Tuning plugin configuration (self-managed)\n\
             # talkativeness_offset is clamped strictly to [{MIN_TALKATIVENESS_OFFSET}, {MAX_TALKATIVENESS_OFFSET}].\n\
             # examples supports up to {MAX_EXCHANGES} exchanges ({MAX_EXAMPLE_TURNS} [speaker, text] turns).\n\n\
             version = {}\n\
             default_mode = {}\n\n\
             # You may add custom modes under [modes.<name>], e.g.:\n\
             # [modes.concise]\n\
             # rule = \"Answer in at most two short sentences.\"\n\
             # talkativeness_offset = -0.15\n\
             # examples = [[\"person\", \"status?\"], [\"you\", \"All checks pass.\"]]\n\n",
            self.version,
            toml::Value::String(self.default_mode.clone()),
        );
        let modes_body =
            toml::to_string_pretty(&ModesTomlView { modes: &self.modes }).unwrap_or_default();
        format!("{header}{modes_body}")
    }
}

/// Seeds the canonical `"daily"` and `"professional"` modes from `soul-tuning`.
#[must_use]
pub fn default_modes() -> BTreeMap<String, SoulModeSpec> {
    let mut modes = BTreeMap::new();
    modes.insert(
        "daily".to_owned(),
        SoulModeSpec {
            rule: "Speak casually, like a friend in the same chat; short lines.".to_owned(),
            talkativeness_offset: 0.05,
            examples: vec![
                ["person".to_owned(), "morning".to_owned()],
                ["you".to_owned(), "morning! how can I help?".to_owned()],
                ["person".to_owned(), "busy?".to_owned()],
                ["you".to_owned(), "not really, how about you?".to_owned()],
                ["person".to_owned(), "still on tonight?".to_owned()],
                [
                    "you".to_owned(),
                    "yes. I'll tell you if anything changes.".to_owned(),
                ],
            ],
        },
    );
    modes.insert(
        "professional".to_owned(),
        SoulModeSpec {
            rule: "Speak plainly and professionally; no slang, no filler, complete sentences."
                .to_owned(),
            talkativeness_offset: -0.05,
            examples: vec![
                ["person".to_owned(), "good morning".to_owned()],
                [
                    "you".to_owned(),
                    "Good morning. How can I help today?".to_owned(),
                ],
                [
                    "person".to_owned(),
                    "please summarise yesterday's report".to_owned(),
                ],
                [
                    "you".to_owned(),
                    "Certainly. I will prepare and send the summary today.".to_owned(),
                ],
                ["person".to_owned(), "thank you".to_owned()],
                [
                    "you".to_owned(),
                    "You are welcome. Please let me know if you need anything.".to_owned(),
                ],
            ],
        },
    );
    modes
}

/// Resolves the state directory from `MONOAGENS_PLUGIN_STATE_DIR` first,
/// falling back to `MONOAGENS_BODY_STATE_DIR`.
#[must_use]
pub fn resolve_state_dir_from_env() -> Option<PathBuf> {
    env::var_os(ENV_PLUGIN_STATE_DIR)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(load_state_dir_from_env)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedActiveMode {
    active_mode: String,
}

/// Loads the persisted active mode from `<state_dir>/active_mode.json` if present and valid
/// in `config.modes`; otherwise returns `config.default_mode`.
#[must_use]
pub fn load_active_mode(state_dir: Option<&Path>, config: &SoulTuningConfig) -> String {
    let Some(dir) = state_dir else {
        return config.default_mode.clone();
    };
    let path = dir.join(ACTIVE_MODE_FILENAME);
    if !path.exists() {
        return config.default_mode.clone();
    }
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(err) => {
            eprintln!(
                "plugin-soul-tuning: failed to read '{}': {err}; using default_mode '{}'",
                path.display(),
                config.default_mode
            );
            return config.default_mode.clone();
        }
    };
    match serde_json::from_str::<PersistedActiveMode>(&content) {
        Ok(persisted) if config.modes.contains_key(&persisted.active_mode) => persisted.active_mode,
        Ok(persisted) => {
            eprintln!(
                "plugin-soul-tuning: persisted active_mode '{}' in '{}' is not in configured modes; using default_mode '{}'",
                persisted.active_mode,
                path.display(),
                config.default_mode
            );
            config.default_mode.clone()
        }
        Err(err) => {
            eprintln!(
                "plugin-soul-tuning: failed to parse '{}': {err}; using default_mode '{}'",
                path.display(),
                config.default_mode
            );
            config.default_mode.clone()
        }
    }
}

/// Atomically persists `active_mode` to `<state_dir>/active_mode.json` when `state_dir` is `Some`.
pub fn save_active_mode(state_dir: Option<&Path>, active_mode: &str) -> Result<(), String> {
    let Some(dir) = state_dir else {
        return Ok(());
    };
    fs::create_dir_all(dir).map_err(|e| {
        format!(
            "failed to create state directory '{}': {e}",
            dir.display()
        )
    })?;

    let target = dir.join(ACTIVE_MODE_FILENAME);
    let tmp = dir.join(format!(".{ACTIVE_MODE_FILENAME}.tmp.{}", std::process::id()));
    let encoded = serde_json::to_vec_pretty(&PersistedActiveMode {
        active_mode: active_mode.to_owned(),
    })
    .map_err(|e| format!("failed to serialize active_mode: {e}"))?;

    {
        let mut file = fs::File::create(&tmp)
            .map_err(|e| format!("failed to create temp file '{}': {e}", tmp.display()))?;
        file.write_all(&encoded)
            .map_err(|e| format!("failed to write temp file '{}': {e}", tmp.display()))?;
        file.sync_all()
            .map_err(|e| format!("failed to sync temp file '{}': {e}", tmp.display()))?;
    }

    fs::rename(&tmp, &target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!(
            "failed to rename '{}' to '{}': {e}",
            tmp.display(),
            target.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_seeds_daily_and_professional() {
        let cfg = SoulTuningConfig::default();
        assert_eq!(cfg.version, 1);
        assert_eq!(cfg.default_mode, "daily");
        assert!(cfg.modes.contains_key("daily"));
        assert!(cfg.modes.contains_key("professional"));
        assert!((cfg.modes["daily"].talkativeness_offset - 0.05).abs() < 1e-6);
        assert!((cfg.modes["professional"].talkativeness_offset - (-0.05)).abs() < 1e-6);
        assert_eq!(cfg.modes["daily"].examples.len(), 6);
        assert_eq!(cfg.modes["professional"].examples.len(), 6);
    }

    #[test]
    fn toml_roundtrip_preserves_defaults_and_custom_modes() {
        let mut cfg = SoulTuningConfig::default();
        cfg.modes.insert(
            "concise".to_owned(),
            SoulModeSpec {
                rule: "Keep replies to one sentence.".to_owned(),
                talkativeness_offset: -0.15,
                examples: vec![
                    ["person".to_owned(), "status?".to_owned()],
                    ["you".to_owned(), "Green across the board.".to_owned()],
                ],
            },
        );
        let rendered = cfg.to_toml_string();
        let parsed = SoulTuningConfig::from_toml_str(&rendered).unwrap();
        assert_eq!(parsed, cfg);
    }

    #[test]
    fn toml_clamps_out_of_range_talkativeness_offsets() {
        let raw = r#"
version = 1
default_mode = "daily"

[modes.hyper]
rule = "Talk enthusiastically."
talkativeness_offset = 0.85

[modes.silent]
rule = "Say very little."
talkativeness_offset = -0.95
"#;
        let cfg = SoulTuningConfig::from_toml_str(raw).unwrap();
        assert!((cfg.modes["hyper"].talkativeness_offset - 0.2).abs() < 1e-6);
        assert!((cfg.modes["silent"].talkativeness_offset - (-0.2)).abs() < 1e-6);
        assert!(cfg.modes.contains_key("daily"));
        assert!(cfg.modes.contains_key("professional"));
    }

    #[test]
    fn toml_rejects_excessive_examples_and_invalid_versions() {
        let too_many = (0..11)
            .map(|i| format!("[\"person\", \"turn {i}\"]"))
            .collect::<Vec<_>>()
            .join(", ");
        let raw = format!(
            "version = 1\n[modes.daily]\nrule = \"ok\"\ntalkativeness_offset = 0.0\nexamples = [{too_many}]\n"
        );
        let err = SoulTuningConfig::from_toml_str(&raw).unwrap_err();
        assert!(err.contains("exceeding max 10"));

        let ver_err = SoulTuningConfig::from_toml_str("version = 99\n").unwrap_err();
        assert!(ver_err.contains("unsupported config version 99"));
    }
}

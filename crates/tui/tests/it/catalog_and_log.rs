//! Contract tests for Catalog (`/model`, `/provider`, `/usage`), `/log` overlay filters,
//! config-error banner + `/retry`, and `/doctor` diagnostics.

use std::os::unix::fs::PermissionsExt;

use crossterm::event::{KeyCode, KeyModifiers};
use monoagens_gateway::{
    GatewayConfig, LogEntry, PluginConfig, PluginKind, ResolvedPaths, RunLogLevel,
};
use monoagens_protocol::ActorId;
use monoagens_tui::catalog_ui::{CatalogAction, CatalogEvent, Screen};
use monoagens_tui::doctor::{DoctorInput, DoctorStatus, format_report, run_doctor};
use monoagens_tui::{
    AppAction, HitRegions, LocalChatStore, SecretsUiSummary, SettingsTab, TuiAppState, TuiEffects,
    TuiPreferences, render_to_buffer,
};
use serde_json::json;
use tempfile::TempDir;

use crate::common::{TuiTestHarness, key_press};

fn make_state(dir: &TempDir) -> TuiAppState {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    let paths = ResolvedPaths {
        config_path: cfg_dir.join("monoagens.toml"),
        secrets_path: cfg_dir.join("secrets.toml"),
        data_dir,
    };
    TuiAppState::new(
        paths,
        GatewayConfig::default(),
        TuiPreferences::default(),
        LocalChatStore::default(),
        SecretsUiSummary::default(),
        ActorId::new("per_owner"),
    )
}

fn type_str(s: &mut TuiAppState, text: &str) {
    for c in text.chars() {
        s.handle_key_event(key_press(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn render_text(s: &TuiAppState, w: u16, h: u16) -> String {
    let mut harness = TuiTestHarness::new(w, h);
    let mut fx = TuiEffects::new();
    let mut hit = HitRegions::default();
    harness.draw_with(|area, buf| render_to_buffer(area, buf, s, &mut fx, &mut hit, 16));
    harness.buffer_text()
}

fn sample_providers() -> serde_json::Value {
    json!({
        "providers": [
            { "id": "openai", "label": "OpenAI", "key_set": true, "logged_in": true, "discovery": true },
            { "id": "groq", "label": "Groq", "key_set": false, "logged_in": false, "discovery": true }
        ]
    })
}

fn sample_models(ids: &[&str]) -> serde_json::Value {
    json!({
        "models": ids.iter().map(|id| json!({
            "id": id,
            "display_name": id,
            "context_length": 128000,
            "price_in_per_mtok": 1.0,
            "price_out_per_mtok": 2.0
        })).collect::<Vec<_>>()
    })
}

#[test]
fn model_picker_search_select_custom_and_unconfigured_provider_key_prompt() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = make_state(&dir);

    let open_acts = s.execute_slash_command("/model");
    assert!(!open_acts.is_empty());
    s.catalog.apply(CatalogEvent::Providers(Ok(sample_providers())));
    s.catalog.apply(CatalogEvent::Models {
        provider: "openai".into(),
        refresh: false,
        result: Ok(sample_models(&["gpt-4o", "gpt-4o-mini"])),
    });

    let txt = render_text(&s, 100, 30);
    assert!(txt.contains("OpenAI"), "missing configured provider:\n{txt}");
    assert!(txt.contains("gpt-4o-mini"), "missing model:\n{txt}");

    // Unconfigured provider selection prompts for API key first before switching.
    s.handle_key_event(key_press(KeyCode::Left, KeyModifiers::NONE));
    s.handle_key_event(key_press(KeyCode::Down, KeyModifiers::NONE)); // groq (key_set: false)
    s.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE));
    s.handle_key_event(key_press(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(s.catalog.screen, Screen::Picker);

    // Search filters to gpt-4o-mini and Enter emits SelectModel.
    s.handle_key_event(key_press(KeyCode::Up, KeyModifiers::NONE)); // back to openai
    s.handle_key_event(key_press(KeyCode::Tab, KeyModifiers::NONE));
    type_str(&mut s, "mini");
    let picked = s.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(picked.iter().any(|a| matches!(a, AppAction::Catalog(CatalogAction::SelectModel(_)))));

    // Unlisted typed ID allows using custom model directly under a configured provider.
    s.gateway_cfg.model.catalog_provider = Some("openai".into());
    s.execute_slash_command("/model");
    s.catalog.apply(CatalogEvent::Models {
        provider: "openai".into(),
        refresh: false,
        result: Ok(sample_models(&["gpt-4o"])),
    });
    type_str(&mut s, "my-custom-ft-model");
    let custom = s.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE));
    assert!(custom.iter().any(|a| matches!(a, AppAction::Catalog(CatalogAction::SelectModel(_)))));
}

#[test]
fn provider_login_usage_and_status_bar_cost_rendering() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = make_state(&dir);

    s.execute_slash_command("/provider");
    s.catalog.apply(CatalogEvent::Providers(Ok(sample_providers())));
    let txt = render_text(&s, 100, 28);
    assert!(txt.contains("OpenAI") && txt.contains("Groq"), "{txt}");

    s.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!s.catalog.is_open());

    s.execute_slash_command("/usage");
    s.catalog.apply(CatalogEvent::Usage(Ok(json!({
        "daily_used": 1200,
        "daily_budget": 10000,
        "estimated_cost_usd": 0.042,
        "purposes": [
            { "purpose": "reason", "tokens": 900, "calls": 3, "cost_usd": 0.032 },
            { "purpose": "classify", "tokens": 300, "calls": 2, "cost_usd": 0.010 }
        ]
    }))));
    let u_txt = render_text(&s, 100, 28);
    assert!(u_txt.contains("reason") && u_txt.contains("classify"), "{u_txt}");
    s.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!s.catalog.is_open());

    // Cost display appears when known and hides when None.
    s.estimated_cost_usd = None;
    let bar_no_cost = render_text(&s, 100, 24);
    assert!(!bar_no_cost.contains("$0."));
    s.estimated_cost_usd = Some(0.15);
    let bar_cost = render_text(&s, 100, 24);
    assert!(bar_cost.contains("$0.15"));
}

#[test]
fn log_overlay_filters_sources_follow_toggle_and_doctor_config_error() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = make_state(&dir);
    s.log.push(LogEntry { ts_ms: 1, level: RunLogLevel::Debug, source: "supervisor".into(), message: "dbg".into() });
    s.log.push(LogEntry { ts_ms: 2, level: RunLogLevel::Warn, source: "whatsapp".into(), message: "reconnect".into() });
    s.log.push(LogEntry { ts_ms: 3, level: RunLogLevel::Error, source: "provider".into(), message: "rate limited".into() });

    s.execute_slash_command("/log");
    assert!(s.log.open);
    s.handle_key_event(key_press(KeyCode::Char('4'), KeyModifiers::NONE)); // Error only
    assert_eq!(s.log.filtered().len(), 1);
    s.handle_key_event(key_press(KeyCode::Char('1'), KeyModifiers::NONE)); // All levels
    s.handle_key_event(key_press(KeyCode::Char('s'), KeyModifiers::NONE)); // Cycle source
    assert!(s.log.source.is_some());
    s.handle_key_event(key_press(KeyCode::Char('f'), KeyModifiers::NONE)); // Toggle follow
    assert!(!s.log.follow);
    s.handle_key_event(key_press(KeyCode::Esc, KeyModifiers::NONE));
    assert!(!s.log.open);

    // /doctor command emits RunDoctor + missing binary check works.
    assert_eq!(s.execute_slash_command("/doctor"), vec![AppAction::RunDoctor]);

    let missing = dir.path().join("no-such-bin");
    std::fs::write(
        &s.paths.config_path,
        format!(
            "version = 1\n[model]\nprovider = \"scripted\"\nmodel = \"m\"\n\n[plugins.ghost]\nkind = \"adapter\"\nenabled = true\ncommand = \"{}\"\n",
            missing.display()
        ),
    )
    .unwrap();
    std::fs::write(&s.paths.secrets_path, "").unwrap();
    std::fs::set_permissions(&s.paths.secrets_path, std::fs::Permissions::from_mode(0o644)).unwrap();

    let checks = run_doctor(&DoctorInput {
        paths: s.paths.clone(),
        link: "hosting".into(),
    });
    let report = format_report(&checks);
    assert!(report.contains("[FAIL] plugin:ghost"), "{report}");
    assert_eq!(
        checks.iter().find(|c| c.name == "secrets").unwrap().status,
        DoctorStatus::Fail
    );

    // Config error banner disables chat and /settings opens on the offending tab.
    s.set_config_error("bad provider".into(), Some("model.provider".into()));
    let err_screen = render_text(&s, 100, 24);
    assert!(err_screen.contains("Configuration error") && err_screen.contains("/retry"));
    type_str(&mut s, "blocked chat");
    assert!(s.handle_key_event(key_press(KeyCode::Enter, KeyModifiers::NONE)).is_empty());
    s.execute_slash_command("/settings");
    assert_eq!(s.settings.tab, SettingsTab::Model);

    let _ = PluginConfig {
        id: String::new(),
        enabled: false,
        kind: PluginKind::Adapter,
        transport: monoagens_gateway::BodyTransportConfig::Stdio { command: String::new(), args: vec![] },
        dependencies: vec![],
        optional_dependencies: vec![],
    };
}

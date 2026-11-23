//! `/model` picker, `/login`, key detour, `/usage`, cost display and slash descriptions
//! (change `model-provider-catalog`, groups 4 and 5). Gateway answers are synthetic
//! [`CatalogEvent`]s; the real round trips are covered in `catalog_ui::tests`.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use monoagens_gateway::{GatewayConfig, ResolvedPaths};
use monoagens_protocol::ActorId;
use monoagens_tui::catalog_ui::Screen;
use monoagens_tui::{
    AppAction, CatalogAction, CatalogEvent, HitRegions, LocalChatStore, SecretsUiSummary, TuiAppState,
    TuiEffects, TuiPreferences, render_to_buffer,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use serde_json::json;
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

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
    let mut cfg = GatewayConfig::default();
    cfg.model.model = "deepseek-v4.1-flash".to_owned();
    cfg.model.catalog_provider = Some("alpha".to_owned());
    TuiAppState::new(
        paths,
        cfg,
        TuiPreferences::default(),
        LocalChatStore::default(),
        SecretsUiSummary::default(),
        ActorId::new("per_owner"),
    )
}

fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
}

fn type_text(s: &mut TuiAppState, t: &str) {
    for ch in t.chars() {
        s.handle_key_event(key(KeyCode::Char(ch)));
    }
}

fn providers_json() -> serde_json::Value {
    json!({
        "current": { "provider": "alpha", "model": "deepseek-v4.1-flash" },
        "providers": [
            { "id": "alpha", "label": "Alpha Cloud", "keyless": false, "key_set": true, "key_source": "secrets", "discovery": true },
            { "id": "local", "label": "Local Box", "keyless": true, "key_set": false, "key_source": null, "discovery": true },
            { "id": "beta", "label": "Beta AI", "keyless": false, "key_set": false, "key_source": null, "discovery": true },
        ]
    })
}

fn models_json(ids: &[&str]) -> serde_json::Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    json!({
        "provider": "alpha", "fetched_at_ms": now, "stale": false, "cached": true, "error": null,
        "models": ids.iter().map(|i| json!({ "id": i, "context_length": 128_000, "price_in_per_mtok": 0.5, "price_out_per_mtok": 1.5 })).collect::<Vec<_>>(),
    })
}

/// Opens `/model` and feeds the provider list plus three models of `alpha`.
fn open_picker(dir: &TempDir) -> TuiAppState {
    let mut s = make_state(dir);
    let actions = s.execute_slash_command("/model");
    assert_eq!(actions, vec![AppAction::Catalog(CatalogAction::LoadProviders)]);
    assert_eq!(s.catalog.screen, Screen::Picker);
    s.catalog.apply(CatalogEvent::Providers(Ok(providers_json())));
    s.catalog.apply(CatalogEvent::Models {
        provider: "alpha".into(),
        refresh: false,
        result: Ok(models_json(&["deepseek-v4.1-flash", "qwen3-coder", "qwen3-mini", "llama-big"])),
    });
    s
}

fn render(w: u16, h: u16, state: &mut TuiAppState) -> Buffer {
    state.catalog.view_rows = (h as usize).saturating_sub(5).max(1);
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render_to_buffer(f.area(), f.buffer_mut(), state, &mut effects, &mut hit, 16))
        .unwrap();
    term.backend().buffer().clone()
}

fn text_of(buf: &Buffer) -> String {
    let mut out = String::new();
    for y in 0..buf.area.height {
        let mut line = String::new();
        let mut skip = 0usize;
        for x in 0..buf.area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf.cell((x, y)).unwrap().symbol();
            skip = sym.width().saturating_sub(1);
            line.push_str(sym);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[test]
fn picker_lists_configured_first_then_separator_and_unconfigured() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = open_picker(&dir);
    let text = text_of(&render(100, 30, &mut s));
    let all = text.find("All models").unwrap();
    let alpha = text.find("● Alpha Cloud (4)").expect("configured with count");
    let local = text.find("● Local Box").expect("keyless counts as configured");
    let beta = text.find("○ Beta AI").expect("unconfigured");
    assert!(all < alpha && alpha < local && local < beta, "{text}");
    assert!(text.contains("───"), "separator: {text}");
    assert!(text.contains("qwen3-coder") && text.contains("ctx 128k"), "{text}");
    assert!(text.contains("now: Alpha Cloud / deepseek-v4.1-flash"), "{text}");
}

#[test]
fn search_narrows_the_list_and_enter_selects_through_config_apply() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = open_picker(&dir);
    type_text(&mut s, "qwen");
    let rows = s.catalog.right_rows();
    assert_eq!(rows.len(), 3, "two qwen models plus the custom-id row");
    let text = text_of(&render(100, 30, &mut s));
    assert!(text.contains("qwen3-coder") && text.contains("qwen3-mini"), "{text}");
    assert!(!text.contains("llama-big"), "{text}");

    let actions = s.handle_key_event(key(KeyCode::Enter));
    match actions.as_slice() {
        [AppAction::Catalog(CatalogAction::SelectModel(c))] => {
            assert_eq!(c.provider, "alpha");
            assert_eq!(c.model, "qwen3-coder");
            assert!(!c.provider_changed);
        }
        other => panic!("{other:?}"),
    }
    assert!(!s.catalog.is_open());
    // The gateway confirms: model name and slash description follow.
    let choice = match &actions[0] {
        AppAction::Catalog(CatalogAction::SelectModel(c)) => c.clone(),
        _ => unreachable!(),
    };
    let sel = s.catalog.apply(CatalogEvent::ModelSelected { choice, result: Ok(()) });
    assert_eq!(sel.unwrap().model, "qwen3-coder");
    assert!(s.catalog.take_toast().unwrap().0.contains("qwen3-coder"));
}

#[test]
fn typed_unlisted_id_can_be_used_as_custom_model() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = open_picker(&dir);
    type_text(&mut s, "my-private-model");
    let actions = s.handle_key_event(key(KeyCode::Enter));
    assert!(matches!(
        actions.as_slice(),
        [AppAction::Catalog(CatalogAction::SelectModel(c))] if c.model == "my-private-model" && c.provider == "alpha"
    ));
}

#[test]
fn choosing_a_model_of_an_unconfigured_provider_opens_the_key_page_first() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = open_picker(&dir);
    // Provider pane: move to Beta AI (All, Alpha, Local, separator skipped, Beta).
    s.handle_key_event(key(KeyCode::Left));
    for _ in 0..3 {
        s.handle_key_event(key(KeyCode::Down));
    }
    assert_eq!(s.catalog.picker_provider().as_deref(), Some("beta"));
    s.handle_key_event(key(KeyCode::Right));
    type_text(&mut s, "beta-model-1");
    let actions = s.handle_key_event(key(KeyCode::Enter));
    assert!(actions.is_empty());
    assert_eq!(s.catalog.screen, Screen::KeyEntry);
    let text = text_of(&render(100, 30, &mut s));
    assert!(text.contains("API key") && text.contains("Beta AI") && text.contains("beta-model-1"), "{text}");

    // The key is masked on screen and in the action's Debug output.
    type_text(&mut s, "sk-secret-123");
    let text = text_of(&render(100, 30, &mut s));
    assert!(!text.contains("sk-secret"), "{text}");
    assert!(text.contains("•••••••••••••"), "{text}");
    let actions = s.handle_key_event(key(KeyCode::Enter));
    let [AppAction::Catalog(CatalogAction::TestKey { provider, then, .. })] = actions.as_slice() else {
        panic!("{actions:?}")
    };
    assert_eq!(provider, "beta");
    assert_eq!(then.as_ref().unwrap().model, "beta-model-1");
    assert!(!format!("{actions:?}").contains("sk-secret"));

    // Test and save succeed; the page reports model count and latency; the selection follows.
    s.catalog.apply(CatalogEvent::KeyTested {
        provider: "beta".into(),
        result: Ok(json!({ "ok": true, "model_count": 30, "latency_ms": 187 })),
    });
    let text = text_of(&render(100, 30, &mut s));
    assert!(text.contains("30 models") && text.contains("187 ms"), "{text}");
    s.catalog.apply(CatalogEvent::KeySaved { provider: "beta".into(), result: Ok(()) });
    assert!(s.catalog.provider("beta").unwrap().key_set);
}

#[test]
fn login_list_shows_key_state_searches_and_esc_returns() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = make_state(&dir);
    let actions = s.execute_slash_command("/login");
    assert_eq!(actions, vec![AppAction::Catalog(CatalogAction::LoadProviders)]);
    s.catalog.apply(CatalogEvent::Providers(Ok(providers_json())));
    let text = text_of(&render(100, 30, &mut s));
    assert!(text.contains("● Alpha Cloud"), "{text}");
    assert!(text.contains("key set"), "{text}");
    assert!(text.contains("○ Beta AI") && text.contains("not set"), "{text}");

    type_text(&mut s, "beta");
    assert_eq!(s.catalog.login_rows().len(), 1);
    s.handle_key_event(key(KeyCode::Enter));
    assert_eq!(s.catalog.screen, Screen::KeyEntry);
    s.handle_paste("sk-pasted\n");
    let actions = s.handle_key_event(key(KeyCode::Enter));
    assert!(matches!(actions.as_slice(), [AppAction::Catalog(CatalogAction::TestKey { then: None, .. })]));
    // Esc goes back to the list, a second Esc (after clearing the search) to the chat.
    s.handle_key_event(key(KeyCode::Esc));
    assert_eq!(s.catalog.screen, Screen::LoginList);
    s.handle_key_event(key(KeyCode::Esc));
    s.handle_key_event(key(KeyCode::Esc));
    assert!(!s.catalog.is_open());
}

#[test]
fn failed_test_is_shown_and_not_saved() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = make_state(&dir);
    s.execute_slash_command("/login");
    s.catalog.apply(CatalogEvent::Providers(Ok(providers_json())));
    type_text(&mut s, "beta");
    s.handle_key_event(key(KeyCode::Enter));
    type_text(&mut s, "bad");
    s.handle_key_event(key(KeyCode::Enter));
    s.catalog.apply(CatalogEvent::KeyTested {
        provider: "beta".into(),
        result: Ok(json!({ "ok": false, "error": "HTTP 401: nope", "latency_ms": 5, "model_count": 0 })),
    });
    let text = text_of(&render(100, 30, &mut s));
    assert!(text.contains("HTTP 401: nope") && text.contains("Ctrl+S"), "{text}");
    assert!(!s.catalog.provider("beta").unwrap().key_set);
}

#[test]
fn slash_menu_descriptions_show_live_state() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let s = make_state(&dir);
    let cmds = s.available_slash_commands();
    let model = cmds.iter().find(|c| c.name == "model").unwrap();
    assert!(model.description.contains("deepseek-v4.1-flash"), "{model:?}");
    assert!(cmds.iter().any(|c| c.name == "login"));
    assert!(cmds.iter().any(|c| c.name == "usage"));
}

#[test]
fn usage_view_lists_both_purposes_and_budget_left() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = make_state(&dir);
    let actions = s.execute_slash_command("/usage");
    assert_eq!(actions, vec![AppAction::Catalog(CatalogAction::LoadUsage)]);
    s.catalog.apply(CatalogEvent::Usage(Ok(json!({
        "day_utc": 1, "cost_usd": null,
        "budget": { "used": 450, "limit": 1000, "left": 550 },
        "purposes": [
            { "purpose": "classify", "provider": "alpha", "model": "small", "calls": 1, "input_tokens": 100, "output_tokens": 20, "cost_usd": null },
            { "purpose": "reason", "provider": "alpha", "model": "big", "calls": 1, "input_tokens": 250, "output_tokens": 80, "cost_usd": null },
        ]
    }))));
    let text = text_of(&render(120, 30, &mut s));
    assert!(text.contains("classify") && text.contains("reason"), "{text}");
    assert!(text.contains("550 left"), "{text}");
    assert!(text.contains("250") && text.contains("80"), "{text}");
    assert!(text.contains("unknown"), "no price -> cost unknown: {text}");
}

#[test]
fn status_bar_hides_cost_when_unknown_and_shows_it_when_known() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let mut s = make_state(&dir);
    s.prefs.show_cost_estimate = true;
    s.estimated_cost_usd = None;
    let text = text_of(&render(140, 30, &mut s));
    assert!(!text.contains("$0.00") && !text.contains(" · $"), "{text}");
    s.estimated_cost_usd = Some(0.0004);
    let text = text_of(&render(140, 30, &mut s));
    assert!(text.contains("$0.0004"), "{text}");
    s.estimated_cost_usd = Some(1.5);
    assert!(text_of(&render(140, 30, &mut s)).contains("$1.50"));
}

#[test]
fn provider_error_notice_matches_the_recorded_failure() {
    let n = monoagens_tui::catalog_ui::provider_failure_notice(
        "model",
        "model call failed (rate_limit, model 'm', latency 3ms): HTTP 429: {\"error\":{\"message\":\"too many\"}}",
    )
    .unwrap();
    assert!(n.contains("too many") && n.contains("/log") && n.contains("429"), "{n}");
}

#[test]
fn snapshot_picker_at_two_sizes() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    for (w, h) in [(80u16, 24u16), (120, 40)] {
        let mut s = open_picker(&dir);
        insta::assert_snapshot!(format!("picker_{w}x{h}"), text_of(&render(w, h, &mut s)));
    }
}

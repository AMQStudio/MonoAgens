//! Contract tests for Wave 2 RBAC Simple ON/OFF Toggle Matrix (`views/rbac/`):
//! auto-detected groups, Simple ON/OFF + `[🔒]` owner-only + `[–]` partial master toggles,
//! wildcard-to-explicit materialization on uncheck with zero hidden `deny` entries, live
//! `RbacConfig::evaluate` Inspector accuracy, `Friendly Label (tech.id)` dual search,
//! `Custom patterns` inline `TextInput` paste isolation, and attached-mode config sync.

use crossterm::event::{KeyCode, KeyModifiers};
use monoagens_gateway::{ResolvedPaths, load_config_file, parse_config_str};
use monoagens_protocol::ActorId;
use monoagens_tui::core::hitmap::{HitMap, HitTarget};
use monoagens_tui::core::overlay::WidgetOutcome;
use monoagens_tui::core::theme::{ThemePalette, ThemePreset};
use monoagens_tui::settings_runtime::SettingsEvent;
use monoagens_tui::views::rbac::catalog::{CapabilityCatalog, CapabilityEntry, CapabilityFlagsInfo};
use monoagens_tui::views::rbac::matrix::{
    CUSTOM_DOMAIN_ID, apply_rbac_patch_to_toml, capability_state, evaluate_inspector, group_state,
    toggle_capability_patch_with_catalog, toggle_group_patch_with_catalog,
};
use monoagens_tui::views::rbac::{RbacAction, RbacPaneFocus, RbacViewState, ToggleState};
use monoagens_tui::{
    LocalChatStore, PluginUiEntry, PluginUiState, SecretsUiSummary, SettingsTab, TuiAppState,
    TuiPreferences,
};
use monoagens_valve::rbac::{RbacConfig, RoleDefinition};
use serde_json::{Value, json};
use tempfile::TempDir;

use crate::common::{TuiTestHarness, key_press, mouse_click};

fn sample_caps_json() -> Value {
    json!({
        "groups": [
            {
                "id": "core",
                "label": "Core Agent",
                "kind": "core",
                "running": true,
                "body_id": Value::Null,
                "capabilities": [
                    {
                        "id": "agent.converse",
                        "display_name": "Allow Conversation",
                        "description": "Allow this role to chat with and wake the agent",
                        "flags": { "communicative": true, "identity_changing": false, "irreversible": false },
                        "risk_tier": "R0",
                        "owner_only": false
                    }
                ]
            },
            {
                "id": "whatsapp",
                "label": "WhatsApp",
                "kind": "adapter",
                "running": true,
                "body_id": "whatsapp",
                "capabilities": [
                    {
                        "id": "chat.speak",
                        "display_name": "Send Messages",
                        "description": "Send outbound messages in chat",
                        "flags": { "communicative": true, "identity_changing": false, "irreversible": false },
                        "risk_tier": "R1",
                        "owner_only": false
                    },
                    {
                        "id": "chat.react",
                        "display_name": "Send Reactions",
                        "description": "React to chat messages",
                        "flags": { "communicative": true, "identity_changing": false, "irreversible": false },
                        "risk_tier": "R0",
                        "owner_only": false
                    },
                    {
                        "id": "chat.delete",
                        "display_name": "Delete Messages",
                        "description": "Delete chat messages",
                        "flags": { "communicative": false, "identity_changing": false, "irreversible": true },
                        "risk_tier": "R2",
                        "owner_only": false
                    }
                ]
            },
            {
                "id": "soul-tuning",
                "label": "Soul Tuning",
                "kind": "tool",
                "running": false,
                "body_id": "soul-tuning",
                "capabilities": [
                    {
                        "id": "soul_tuning.get_mode",
                        "display_name": "View Active Mode",
                        "description": "Read active soul mode",
                        "flags": { "communicative": false, "identity_changing": false, "irreversible": false },
                        "risk_tier": "R0",
                        "owner_only": false
                    },
                    {
                        "id": "soul_tuning.set_mode",
                        "display_name": "Switch Soul Mode",
                        "description": "Change identity mode",
                        "flags": { "communicative": false, "identity_changing": true, "irreversible": false },
                        "risk_tier": "R3",
                        "owner_only": true
                    }
                ]
            }
        ],
        "capabilities": []
    })
}

fn sample_rbac_toml() -> &'static str {
    r#"version = 1

[roles]
fallback = "guest"

[roles.guest]
base = "guest"
approval = "default"
allow = ["agent.converse", "chat.*"]
deny = []

[roles.member]
base = "member"
approval = "default"
allow = ["agent.converse", "chat.speak"]
deny = []
"#
}

fn parse_rbac_from_toml(toml_src: &str) -> RbacConfig {
    let dir = TempDir::new().expect("tempdir");
    let cfg = parse_config_str(
        toml_src,
        &dir.path().join("monoagens.toml"),
        &dir.path().join("secrets.toml"),
    )
    .expect("valid monoagens.toml");
    cfg.rbac.expect("parsed rbac section")
}

#[test]
fn rbac_matrix_renders_detected_groups_status_dots_friendly_labels_and_locked_rows() {
    let rbac = parse_rbac_from_toml(sample_rbac_toml());
    let caps = sample_caps_json();
    let plugins: Vec<PluginUiEntry> = Vec::new();
    let mut view = RbacViewState::default();
    view.sync_from_config(&rbac, Some(&caps), &plugins, &[]);
    view.selected_domain_id = "whatsapp".to_owned();
    view.matrix_idx = 1; // Select `chat.speak` so Inspector evaluates it live.

    for (w, h) in [(80_u16, 24_u16), (40_u16, 12_u16)] {
        let mut harness = TuiTestHarness::new(w, h);
        let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);
        let mut hitmap = HitMap::default();
        harness.draw_with(|area, buf| {
            hitmap.clear();
            view.render(area, buf, &rbac, &palette, &mut hitmap);
        });
        let full = harness.buffer_text();
        assert!(full.contains("Send Messages (chat.speak)"), "missing friendly cap at {w}x{h}:\n{full}");
        assert!(full.contains("ALLOWED"), "missing Inspector verdict at {w}x{h}:\n{full}");
        if w >= 56 {
            assert!(full.contains("● Core Agent"), "missing running dot at {w}x{h}:\n{full}");
            assert!(full.contains("● WhatsApp [3/3]"), "missing whatsapp badge at {w}x{h}:\n{full}");
            assert!(full.contains("○ Soul Tuning [0/2]"), "missing stopped dot at {w}x{h}:\n{full}");
            assert!(full.contains("Custom patterns"), "missing custom patterns at {w}x{h}:\n{full}");
        }
    }

    // Switch to Soul Tuning group and verify owner-only capability renders locked & cannot be toggled.
    view.selected_domain_id = "soul-tuning".to_owned();
    view.focus = RbacPaneFocus::Matrix;
    let mut harness = TuiTestHarness::new(100, 24);
    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);
    let mut hitmap = HitMap::default();
    harness.draw_with(|area, buf| {
        hitmap.clear();
        view.render(area, buf, &rbac, &palette, &mut hitmap);
    });
    let full = harness.buffer_text();
    assert!(full.contains("[🔒] Switch Soul Mode (soul_tuning.set_mode)"), "expected lock badge:\n{full}");

    let locked_entry = CapabilityEntry {
        id: "soul_tuning.set_mode".to_owned(),
        display_name: "Switch Soul Mode".to_owned(),
        description: String::new(),
        risk_tier: "R3".to_owned(),
        owner_only: true,
        flags: CapabilityFlagsInfo {
            communicative: false,
            identity_changing: true,
            irreversible: false,
        },
    };
    let role = rbac.roles.get("guest").expect("guest role");
    assert_eq!(capability_state(role, &locked_entry), ToggleState::LockedOwner);

    // Move cursor to soul_tuning.set_mode row and press Space -> refusal toast, no ApplyPatch.
    view.matrix_idx = 2; // 0 = All in Soul Tuning, 1 = get_mode, 2 = set_mode
    let outcome = view.handle_key(key_press(KeyCode::Char(' '), KeyModifiers::NONE), &rbac);
    match outcome {
        WidgetOutcome::Action(acts) => {
            assert!(acts.iter().all(|a| !matches!(a, RbacAction::ApplyPatch { .. })), "owner-only toggled unexpectedly: {acts:?}");
            assert!(acts.iter().any(|a| matches!(a, RbacAction::Toast { error: true, .. })));
        }
        other => panic!("expected error toast Action for owner-only toggle, got {other:?}"),
    }
}

#[test]
fn unchecking_one_capability_from_wildcard_materializes_explicit_allow_without_hidden_deny_and_roundtrips() {
    let rbac = parse_rbac_from_toml(sample_rbac_toml());
    let catalog = CapabilityCatalog::from_json_and_plugins(Some(&sample_caps_json()), &[]);
    let wa_group = catalog.group("whatsapp").expect("whatsapp group");
    let guest_role: &RoleDefinition = rbac.roles.get("guest").expect("guest role");

    // Before toggle: guest has "chat.*", so group master state is On.
    assert_eq!(group_state(guest_role, wa_group), ToggleState::On);

    // Uncheck `chat.delete` while `chat.*` is present.
    let delete_cap = wa_group
        .capabilities
        .iter()
        .find(|c| c.id == "chat.delete")
        .expect("chat.delete");
    let (new_allow, new_deny, patch) =
        toggle_capability_patch_with_catalog("guest", guest_role, delete_cap, wa_group, &catalog);

    assert!(new_deny.is_empty(), "must never add hidden deny entry, got {new_deny:?}");
    assert!(!new_allow.iter().any(|p| p == "chat.*"), "wildcard should expand, got {new_allow:?}");
    assert!(new_allow.contains(&"agent.converse".to_owned()));
    assert!(new_allow.contains(&"chat.speak".to_owned()));
    assert!(new_allow.contains(&"chat.react".to_owned()));
    assert!(!new_allow.contains(&"chat.delete".to_owned()));

    // Apply patch via toml_edit and reload RbacConfig.
    let updated_toml = apply_rbac_patch_to_toml(sample_rbac_toml(), &patch).expect("apply patch");
    let updated_rbac = parse_rbac_from_toml(&updated_toml);
    let updated_guest = updated_rbac.roles.get("guest").expect("updated guest");

    assert_eq!(group_state(updated_guest, wa_group), ToggleState::Partial);
    assert_eq!(
        evaluate_inspector(&updated_rbac, "guest", Some(delete_cap)).status_label,
        "DENIED"
    );
    let speak_cap = wa_group
        .capabilities
        .iter()
        .find(|c| c.id == "chat.speak")
        .expect("chat.speak");
    assert_eq!(
        evaluate_inspector(&updated_rbac, "guest", Some(speak_cap)).status_label,
        "ALLOWED"
    );

    // Toggling group master from Partial enables all -> compacts into wildcard "chat.*";
    // toggling again when On clears all whatsapp caps without touching agent.converse.
    let (all_on_allow, _, on_patch) =
        toggle_group_patch_with_catalog("guest", updated_guest, wa_group, &catalog);
    assert_eq!(all_on_allow, vec!["agent.converse".to_owned(), "chat.*".to_owned()]);
    let on_toml = apply_rbac_patch_to_toml(&updated_toml, &on_patch).expect("apply group on");
    let on_rbac = parse_rbac_from_toml(&on_toml);
    let on_guest = on_rbac.roles.get("guest").unwrap();
    assert_eq!(group_state(on_guest, wa_group), ToggleState::On);

    let (off_allow, _, off_patch) =
        toggle_group_patch_with_catalog("guest", on_guest, wa_group, &catalog);
    assert_eq!(off_allow, vec!["agent.converse".to_owned()]);
    let off_toml = apply_rbac_patch_to_toml(&on_toml, &off_patch).expect("apply group off");
    let off_rbac = parse_rbac_from_toml(&off_toml);
    assert_eq!(group_state(off_rbac.roles.get("guest").unwrap(), wa_group), ToggleState::Off);
}

#[test]
fn rbac_filter_matches_friendly_label_and_id_and_custom_patterns_accepts_paste_without_search_leak() {
    let rbac = parse_rbac_from_toml(sample_rbac_toml());
    let caps = sample_caps_json();
    let mut view = RbacViewState::default();
    view.sync_from_config(&rbac, Some(&caps), &[], &[]);
    view.selected_domain_id = "whatsapp".to_owned();
    view.focus = RbacPaneFocus::Matrix;

    // Live `/` filter matching friendly label "Send Reactions".
    view.handle_key(key_press(KeyCode::Char('/'), KeyModifiers::NONE), &rbac);
    assert!(view.has_active_input());
    assert!(view.handle_paste("Reactions\n"));

    let mut harness = TuiTestHarness::new(80, 24);
    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);
    let mut hitmap = HitMap::default();
    harness.draw_with(|area, buf| {
        hitmap.clear();
        view.render(area, buf, &rbac, &palette, &mut hitmap);
    });
    let text = harness.buffer_text();
    assert!(text.contains("Send Reactions (chat.react)"), "filter missed friendly match:\n{text}");
    assert!(!text.contains("Delete Messages (chat.delete)"), "filter failed to hide non-match:\n{text}");

    // Escape closes filter bar cleanly and preserves matrix navigation.
    view.handle_key(key_press(KeyCode::Esc, KeyModifiers::NONE), &rbac);
    view.handle_key(key_press(KeyCode::Esc, KeyModifiers::NONE), &rbac);

    // Mouse click on `Custom patterns` rail item switches domain, then `a` opens inline prompt for paste.
    let mut click_target = None;
    harness.draw_with(|area, buf| {
        hitmap.clear();
        view.render(area, buf, &rbac, &palette, &mut hitmap);
    });
    for y in 0..24_u16 {
        let row = harness.row_text(y);
        if row.contains("Custom patterns")
            && (0..32_u16).any(|x| matches!(hitmap.hit_test(x, y), Some(HitTarget::RailRow(_))))
        {
            click_target = (0..32_u16).find(|&x| matches!(hitmap.hit_test(x, y), Some(HitTarget::RailRow(_)))).map(|x| (x, y));
            break;
        }
    }
    let (cx, cy) = click_target.expect("Custom patterns rail hit rect");
    view.handle_mouse(mouse_click(cx, cy), &hitmap, &rbac);
    assert_eq!(view.selected_domain_id, CUSTOM_DOMAIN_ID);

    view.handle_key(key_press(KeyCode::Char('a'), KeyModifiers::NONE), &rbac);
    assert!(view.has_active_input());
    assert!(view.handle_paste("pdf.read*\r\n"));
    let outcome = view.handle_key(key_press(KeyCode::Enter, KeyModifiers::NONE), &rbac);
    match outcome {
        WidgetOutcome::Action(acts) => {
            assert!(acts.iter().any(|a| matches!(a, RbacAction::ApplyPatch { patch, .. } if patch.to_string().contains("pdf.read*"))));
        }
        other => panic!("expected ApplyPatch from custom pattern submit, got {other:?}"),
    }
}

#[test]
fn attached_mode_settings_syncs_rbac_from_config_get_and_capabilities_list() {
    let dir = TempDir::new().expect("temp dir");
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    let paths = ResolvedPaths {
        config_path: cfg_dir.join("monoagens.toml"),
        secrets_path: cfg_dir.join("secrets.toml"),
        data_dir,
    };
    std::fs::write(&paths.config_path, sample_rbac_toml()).unwrap();
    let cfg = load_config_file(&paths.config_path, &paths.secrets_path).unwrap();

    let mut state = TuiAppState::new(
        paths.clone(),
        cfg,
        TuiPreferences::default(),
        LocalChatStore::default(),
        SecretsUiSummary::default(),
        ActorId::new("per_owner"),
    );
    state.plugins = vec![PluginUiEntry {
        id: "whatsapp".into(),
        kind: monoagens_gateway::PluginKind::Adapter,
        enabled: true,
        installed: true,
        state: PluginUiState::Running,
        command: None,
        socket_path: None,
        config_path: paths.config_path.clone(),
        state_dir: paths.data_dir.clone(),
        dependencies: Vec::new(),
        optional_dependencies: Vec::new(),
    }];
    state.open_settings_at(SettingsTab::Rbac);

    // Simulate attached ConfigLoaded changing guest allow to include chat.react explicitly,
    // plus enriched capabilities.list from daemon.
    let updated_toml = r#"version = 1

[roles]
fallback = "guest"

[roles.guest]
base = "guest"
approval = "default"
allow = ["agent.converse", "chat.speak", "chat.react"]
deny = []
"#;
    state.on_settings_event(SettingsEvent::ConfigLoaded(json!({
        "path": paths.config_path.display().to_string(),
        "toml": updated_toml,
    })));
    state.on_settings_event(SettingsEvent::RbacHints {
        capabilities: vec!["chat.speak".into(), "chat.react".into(), "chat.delete".into()],
        capabilities_json: Some(sample_caps_json()),
        actors: vec!["wa:user:6285".into()],
    });

    // Render full Settings -> RBAC overlay via TestBackend and verify synced allow counts + zero search leak on paste.
    let mut harness = TuiTestHarness::new(90, 28);
    let mut hitmap = HitMap::default();
    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, true);
    harness.draw_with(|r, buf| {
        hitmap.clear();
        let rbac_cfg = state.gateway_cfg.rbac.clone().unwrap_or_default();
        state.settings.rbac.render(r, buf, &rbac_cfg, &palette, &mut hitmap);
    });
    let screen = harness.buffer_text();
    assert!(screen.contains("WhatsApp [2/3]"), "expected live synced count [2/3]:\n{screen}");

    // Typing `g` inside RBAC tab when `/` filter is NOT active does NOT leak into Settings search_query.
    state.settings.search_query.clear();
    state.settings.search_input.clear();
    let _ = state.handle_settings_paste("leak_check");
    assert!(state.settings.search_query.is_empty(), "RBAC tab paste leaked to Settings search");
}

#[test]
fn role_delete_refuses_referenced_roles_and_full_rbac_toml_roundtrips() {
    use monoagens_tui::views::rbac::matrix::role_reference_reason;
    use monoagens_valve::rbac::{ApprovalMode, RoleTier};

    let dir = TempDir::new().expect("temp dir");
    let full = monoagens_gateway::parse_config_str(
        r#"version = 1
[roles]
fallback = "guest-plus"
[roles.guest-plus]
base = "guest"
[roles.family]
base = "trusted"
allow = ["chat.speak"]
approval = "strict"
[places."tg:1"]
allow = ["chat.*"]
deny = []
[permission_bases]
"tool.shell" = "owner-only"
"chat.speak" = "member"
[[person]]
actor_id = "per_a"
roles = ["family"]
"#,
        &dir.path().join("monoagens.toml"),
        &dir.path().join("secrets.toml"),
    )
    .expect("parse full rbac");
    let rbac = full.rbac.expect("rbac parsed");
    assert_eq!(
        role_reference_reason(&rbac, "guest-plus").as_deref(),
        Some("'guest-plus' is the fallback role")
    );
    assert!(
        role_reference_reason(&rbac, "family")
            .as_deref()
            .is_some_and(|s| s.contains("per_a"))
    );
    assert_eq!(rbac.fallback_role.as_deref(), Some("guest-plus"));
    assert_eq!(rbac.roles["family"].base, RoleTier::Trusted);
    assert_eq!(rbac.roles["family"].approval, Some(ApprovalMode::Strict));
    assert_eq!(rbac.permission_bases["tool.shell"], None);
    assert_eq!(rbac.permission_bases["chat.speak"], Some(RoleTier::Member));
    assert_eq!(rbac.places.len(), 1);
}

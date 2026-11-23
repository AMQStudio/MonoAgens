use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use monoagens_gateway::{GatewayConfig, PluginConfig, PluginKind, ResolvedPaths, BodyTransportConfig};
use monoagens_protocol::{ActionId, ActorId, HAct, ModalitySlot, Time};
use monoagens_reason::{ModelProvider, ProviderFailureClass, ScriptedModelProvider, ScriptedReply};
use monoagens_tui::{
    AppAction, ChatTimelineItem, DynamicTomlDoc, DynamicTomlValue, HitRegions, LocalChatEntry,
    LocalChatStore, MAX_TUI_PERCEPT_INLINE_BYTES, MessageLayout, MotionMode, PendingApprovalCard,
    PluginUiState, SecretsUiSummary, SettingsTab, SetupWizardState, ShimmerMode, SidebarMode,
    ThemePalette, ThemePreset, TitleTrigger, TuiAppState, TuiEffects, TuiPreferences, WizardStep,
    build_percept_payload_and_attachments, encode_base64, format_byte_size, generate_chat_title,
    osc52_sequence_with_tmux, parse_image_dimensions, render_to_buffer, should_request_title,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tempfile::TempDir;

fn make_test_state(dir: &TempDir) -> TuiAppState {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).expect("cfg dir");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let paths = ResolvedPaths {
        config_path: cfg_dir.join("monoagens.toml"),
        secrets_path: cfg_dir.join("secrets.toml"),
        data_dir,
    };
    let mut cfg = GatewayConfig::default();
    cfg.plugins.push(PluginConfig {
        id: "whatsapp".to_owned(),
        enabled: true,
        kind: PluginKind::Adapter,
        transport: BodyTransportConfig::Stdio {
            command: "body-whatsapp".to_owned(),
            args: vec![],
        },
        dependencies: vec![],
        optional_dependencies: vec![],
    });
    cfg.plugins.push(PluginConfig {
        id: "web-search".to_owned(),
        enabled: true,
        kind: PluginKind::Capability,
        transport: BodyTransportConfig::Stdio {
            command: "plugin-web-search".to_owned(),
            args: vec![],
        },
        dependencies: vec![],
        optional_dependencies: vec![],
    });
    TuiAppState::new(
        paths,
        cfg,
        TuiPreferences::default(),
        LocalChatStore::default(),
        SecretsUiSummary::default(),
        ActorId::new("per_owner"),
    )
}

fn buffer_text(buf: &ratatui::buffer::Buffer) -> String {
    use unicode_width::UnicodeWidthStr;
    let mut out = String::new();
    let area = buf.area;
    for y in area.y..area.y + area.height {
        let mut line = String::new();
        let mut skip = 0usize;
        for x in area.x..area.x + area.width {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf.cell((x, y)).expect("cell").symbol();
            let w = sym.width();
            if w > 1 {
                skip = w - 1;
            }
            line.push_str(sym);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn render_headless(
    width: u16,
    height: u16,
    state: &TuiAppState,
    effects: &mut TuiEffects,
    hit: &mut HitRegions,
) -> Terminal<TestBackend> {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).expect("test terminal");
    terminal
        .draw(|frame| {
            render_to_buffer(frame.area(), frame.buffer_mut(), state, effects, hit, 16);
        })
        .expect("draw");
    terminal
}

#[test]
fn local_chat_renders_right_aligned_bubble_no_labels_and_ctrl_b_toggles_sidebar() {
    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);
    state.push_timeline_item(
        "direct",
        ChatTimelineItem::UserMessage {
            sender: "per_owner".to_owned(),
            text: "Hello Mono".to_owned(),
            timestamp_ms: 1000,
            is_group: false,
        },
    );
    state.push_timeline_item(
        "direct",
        ChatTimelineItem::AssistantMessage {
            text: "Hi there, ready to help.".to_owned(),
            duration_ms: Some(420),
            timestamp_ms: 1420,
        },
    );

    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let term = render_headless(130, 32, &state, &mut effects, &mut hit);
    let buf = term.backend().buffer();
    let text = buffer_text(buf);

    assert!(text.contains("CHATS"), "wide viewport shows CHATS sidebar");
    assert!(text.contains("PLUGINS"), "wide viewport shows PLUGINS sidebar");
    assert!(text.contains("Hello Mono"));
    assert!(text.contains("Hi there, ready to help."));
    assert!(!text.contains("You:"), "no You label in 1:1 local chat");
    assert!(!text.contains("Mono:"), "no Mono label in 1:1 local chat");
    assert!(!text.contains("Coming Soon"), "no placeholder header tabs");
    assert!(!text.contains("SPEAK AS"), "SPEAK AS hidden outside /lab");

    // Verify right-aligned user bubble has bubble_bg on right side of chat pane
    let palette = ThemePalette::for_preset_with_no_color(ThemePreset::Titanium, false);
    let mut found_bubble_row = None;
    for y in 0..buf.area.height {
        let row_str: String = (0..buf.area.width)
            .map(|x| buf.cell((x, y)).expect("cell").symbol())
            .collect();
        if row_str.contains("Hello Mono") {
            let pos = row_str.find("Hello Mono").expect("pos") as u16;
            found_bubble_row = Some((y, pos));
            break;
        }
    }
    let (row_y, text_x) = found_bubble_row.expect("bubble row present");
    assert!(text_x > 80, "user message must be right-aligned (x={text_x})");
    assert_eq!(
        buf.cell((text_x, row_y)).expect("cell").bg,
        palette.bubble_bg
    );

    // Ctrl+B hides sidebar, Ctrl+B again restores it
    state.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    assert!(!state.sidebar_visible(130));
    let term_hidden = render_headless(130, 32, &state, &mut effects, &mut hit);
    let text_hidden = buffer_text(term_hidden.backend().buffer());
    assert!(!text_hidden.contains("CHATS"));

    state.handle_key_event(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    assert!(state.sidebar_visible(130));
}

#[test]
fn sidebar_plugins_render_kind_badges_and_three_color_status_dots() {
    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);

    let cases = [
        (PluginUiState::Running, Color::Green, "●"),
        (PluginUiState::Starting, Color::Yellow, "◐"),
        (PluginUiState::Stopping, Color::Yellow, "◐"),
        (PluginUiState::Stopped, Color::Red, "●"),
        (PluginUiState::Failed("boom".to_owned()), Color::Red, "●"),
    ];

    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();

    for (pstate, expected_color, expected_glyph) in cases {
        state.set_plugin_state("whatsapp", pstate.clone());
        state.set_plugin_state("web-search", pstate);
        let term = render_headless(130, 30, &state, &mut effects, &mut hit);
        let buf = term.backend().buffer();
        let text = buffer_text(buf);

        assert!(text.contains("(Adapter)"), "missing (Adapter) badge");
        assert!(text.contains("(Capabilities)"), "missing (Capabilities) badge");

        let mut dot_color_found = false;
        for y in 0..buf.area.height {
            let row_str: String = (0..28)
                .map(|x| buf.cell((x, y)).expect("cell").symbol())
                .collect();
            if row_str.contains("whatsapp") {
                for x in 0..5 {
                    let cell = buf.cell((x, y)).expect("cell");
                    if cell.symbol() == expected_glyph && cell.fg == expected_color {
                        dot_color_found = true;
                    }
                }
            }
        }
        assert!(
            dot_color_found,
            "expected {expected_glyph} with {expected_color:?} for whatsapp"
        );
    }
}

#[test]
fn lab_mode_is_command_gated_and_exit_is_context_aware() {
    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);
    assert!(!state.lab_mode);
    assert!(!state.should_quit);

    // /as before /lab does not switch actor
    state.execute_slash_command("/as alice");
    assert_eq!(state.lab_session.current_actor.as_str(), "per_owner");
    assert!(
        state
            .available_slash_commands()
            .iter()
            .all(|c| !c.lab_only),
        "lab commands hidden in normal chat"
    );

    // Enter /lab
    state.execute_slash_command("/lab");
    assert!(state.lab_mode);
    assert!(
        state
            .available_slash_commands()
            .iter()
            .any(|c| c.name == "as"),
        "/as unlocked in Lab mode"
    );
    state.execute_slash_command("/as alice");
    assert_eq!(state.lab_session.current_actor.as_str(), "alice");

    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let term_lab = render_headless(130, 30, &state, &mut effects, &mut hit);
    let lab_text = buffer_text(term_lab.backend().buffer());
    assert!(lab_text.contains("SPEAK AS"));
    assert!(lab_text.contains("lab"));

    // First /exit leaves Lab mode WITHOUT quitting harness
    state.execute_slash_command("/exit");
    assert!(!state.lab_mode);
    assert!(!state.should_quit);

    // Second /exit quits cleanly from normal chat mode
    state.execute_slash_command("/exit");
    assert!(state.should_quit);
}

#[test]
fn mouse_clicks_and_scroll_drive_chats_plugins_approvals_and_settings() {
    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);
    let second_id = state.chats.create_chat(Some("Rust Notes")).id.clone();
    state.chats.active_chat_id = "direct".to_owned();
    for i in 0..40 {
        state.push_timeline_item(
            "direct",
            ChatTimelineItem::AssistantMessage {
                text: format!("Line {i}"),
                duration_ms: None,
                timestamp_ms: i as u64,
            },
        );
    }
    state.pending_approval = Some(PendingApprovalCard {
        action_id: ActionId::new("act-42"),
        h_act: HAct::new("hash-42"),
        state_epoch: 3,
        capability_id: "shell.exec".to_owned(),
        summary: "Run cargo check".to_owned(),
        selected_approve: true,
    });

    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let viewport = Rect::new(0, 0, 130, 36);
    let _ = render_headless(viewport.width, viewport.height, &state, &mut effects, &mut hit);

    // 1. Mouse wheel scroll up in chat viewport disables auto_scroll and moves offset
    let chat_cx = hit.chat_area.x + hit.chat_area.width / 2;
    let chat_cy = hit.chat_area.y + hit.chat_area.height / 2;
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: chat_cx,
            row: chat_cy,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert!(!state.auto_scroll);

    // 2. Left-click [ Approve ] resolves pending approval
    let app_rect = hit.approval_approve.expect("approve rect");
    let actions = state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: app_rect.x + 1,
            row: app_rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert!(matches!(
        actions.as_slice(),
        [AppAction::ResolveApproval { approved: true, .. }]
    ));
    assert!(state.pending_approval.is_none());

    // 3. Left-click second chat in CHATS switches active chat
    let second_chat_rect = hit
        .sidebar_chats
        .iter()
        .find(|(_, id)| id == &second_id)
        .expect("second chat rect")
        .0;
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: second_chat_rect.x + 1,
            row: second_chat_rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert_eq!(state.chats.active_chat_id, second_id);

    // 4. Left-click a plugin in PLUGINS opens /settings -> Plugins › <id>
    let plugin_rect = hit
        .sidebar_plugins
        .iter()
        .find(|(_, id)| id == "whatsapp")
        .expect("whatsapp rect")
        .0;
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: plugin_rect.x + 1,
            row: plugin_rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert!(state.settings.open);
    assert_eq!(state.settings.tab, SettingsTab::Plugins);
    assert_eq!(
        state
            .settings
            .plugin_detail
            .as_ref()
            .map(|d| d.plugin_id.as_str()),
        Some("whatsapp")
    );

    // 5. Render settings and click Valve tab
    let _ = render_headless(viewport.width, viewport.height, &state, &mut effects, &mut hit);
    let valve_tab_rect = hit
        .settings_tabs
        .iter()
        .find(|(_, t)| *t == SettingsTab::Valve)
        .expect("valve tab rect")
        .0;
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: valve_tab_rect.x + 1,
            row: valve_tab_rect.y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert_eq!(state.settings.tab, SettingsTab::Valve);
    assert!(state.settings.plugin_detail.is_none());

    // 6. Scroll wheel inside settings updates selection index
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: hit.settings_area.x + 30,
            row: hit.settings_area.y + 6,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert_eq!(state.settings.selected_index, 1);
}

#[test]
fn llm_auto_titler_titles_initial_topic_shifts_respects_lock_and_degrades_offline() {
    let mut chat = LocalChatEntry::direct_default();
    assert_eq!(
        should_request_title(&chat, "Help me debug Rust tokio async mutex", true),
        TitleTrigger::Initial
    );

    let scripted: Arc<dyn ModelProvider> = Arc::new(ScriptedModelProvider::from_replies([
        ScriptedReply::text(r#"{"label": "\"Rust Async Mutex\"", "score": 0.95}"#),
        ScriptedReply::text(r#"{"label": "KEEP", "score": 0.90}"#),
        ScriptedReply::text(r#"{"label": "Tokyo Flight Itinerary", "score": 0.92}"#),
        ScriptedReply::failure(ProviderFailureClass::Timeout, "offline"),
    ]));

    let initial = generate_chat_title(
        &scripted,
        "gpt-4o-mini",
        None,
        &["Help me debug Rust tokio async mutex".to_owned()],
    );
    assert_eq!(initial.as_deref(), Some("Rust Async Mutex"));

    // Simulate 2 turns on Rust async topic (Unicode 3-grams)
    chat.title = initial.expect("initial title");
    chat.turns_since_retitle = 2;
    chat.topic_ngrams = monoagens_tui::extract_topic_ngrams(
        "Help me debug Rust tokio async mutex deadlock",
    );

    // Same topic -> None
    assert_eq!(
        should_request_title(&chat, "tokio async mutex deadlock debug", true),
        TitleTrigger::None
    );

    // Model returning KEEP preserves existing title cleanly
    let kept = generate_chat_title(
        &scripted,
        "gpt-4o-mini",
        Some(&chat.title),
        &["tokio async mutex".to_owned()],
    );
    assert_eq!(kept, None);

    // Distinctly different topic -> TopicShift (and test non-Latin multilingual 3-grams too!)
    assert_eq!(
        should_request_title(&chat, "東京のラーメンと新幹線予約", true),
        TitleTrigger::TopicShift
    );
    assert!(!monoagens_tui::extract_topic_ngrams("東京のラーメンと新幹線予約").is_empty());

    // Semantic embedding cosine similarity check
    assert_eq!(
        monoagens_tui::should_request_title_with_embeddings(
            &chat,
            "same topic",
            Some(&[1.0, 0.0, 0.0]),
            Some(&[0.98, 0.1, 0.0]),
            true
        ),
        TitleTrigger::None
    );
    assert_eq!(
        monoagens_tui::should_request_title_with_embeddings(
            &chat,
            "shifted topic",
            Some(&[0.0, 1.0, 0.0]),
            Some(&[1.0, 0.0, 0.0]),
            true
        ),
        TitleTrigger::TopicShift
    );

    let shifted = generate_chat_title(
        &scripted,
        "gpt-4o-mini",
        Some(&chat.title),
        &["Book Tokyo flights and Shibuya ramen".to_owned()],
    );
    assert_eq!(shifted.as_deref(), Some("Tokyo Flight Itinerary"));

    // Manual /chat rename locks title against further auto-retitling
    chat.title_locked = true;
    assert_eq!(
        should_request_title(&chat, "Completely unrelated gardening topic", true),
        TitleTrigger::None
    );

    // Offline/timeout error degrades silently to None
    let failed = generate_chat_title(&scripted, "gpt-4o-mini", None, &["anything".to_owned()]);
    assert_eq!(failed, None);
}

#[test]
fn settings_search_toml_backup_masked_secrets_dynamic_plugin_toml_and_wizard() {
    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);

    // Write initial monoagens.toml with a comment we expect to preserve
    let initial_toml = "# user comment\nversion = 1\n\n[valve]\nmode = \"enforcing\"\nmax_chain_wakes = 8\n";
    std::fs::write(&state.paths.config_path, initial_toml).expect("write monoagens.toml");

    // Open /settings -> Valve, search for "taint", toggle it
    state.settings.open_tab(SettingsTab::Valve);
    state.settings.search_query = "taint".to_owned();
    state.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!state.settings.valve_toggles.taint_blocking);

    // Check .bak was created and comment preserved
    let bak_path = std::path::PathBuf::from(format!("{}.bak", state.paths.config_path.display()));
    assert!(bak_path.exists(), "monoagens.toml.bak must exist");
    let updated_toml = std::fs::read_to_string(&state.paths.config_path).expect("read");
    assert!(updated_toml.contains("# user comment"));
    assert!(updated_toml.contains("taint_blocking = false"));

    // TuiPreferences tui.toml round-trip
    state.prefs.theme = ThemePreset::Terracotta;
    state.prefs.message_layout = MessageLayout::Stream;
    state.prefs.motion = MotionMode::Reduced;
    state.prefs.shimmer = ShimmerMode::Subtle;
    state.prefs.sidebar = SidebarMode::Open;
    state.persist_prefs();
    let prefs_loaded =
        TuiPreferences::load_from_path(&TuiPreferences::path_for_config(&state.paths.config_path))
            .expect("load tui.toml");
    assert_eq!(prefs_loaded, state.prefs);

    // Dynamic plugins/<id>.toml round-trip
    let plugin_toml = dir.path().join("config/plugins/whatsapp.toml");
    std::fs::create_dir_all(plugin_toml.parent().expect("parent")).expect("mkdir");
    std::fs::write(
        &plugin_toml,
        "# WhatsApp config\nauto_reconnect = true\nmax_media_mb = 16\n",
    )
    .expect("write plugin toml");
    let mut doc = DynamicTomlDoc::load_or_init(&plugin_toml).expect("load");
    assert_eq!(doc.entries().len(), 2);
    doc.set_entry("auto_reconnect", DynamicTomlValue::Bool(false))
        .expect("set bool");
    doc.set_entry(
        "allowed_locales",
        DynamicTomlValue::StrArray(vec!["id".to_owned(), "en".to_owned()]),
    )
    .expect("set array");
    doc.save().expect("save");
    let reloaded = DynamicTomlDoc::load_or_init(&plugin_toml).expect("reload");
    assert!(std::fs::read_to_string(&plugin_toml).expect("raw").contains("# WhatsApp config"));
    assert!(reloaded.entries().iter().any(|e| {
        e.key_path == "allowed_locales"
            && e.value == DynamicTomlValue::StrArray(vec!["id".to_owned(), "en".to_owned()])
    }));

    // First-Run Setup Wizard saves masked secrets.toml (0600) + enables selected plugin
    let mut wizard = SetupWizardState::new(
        &state.gateway_cfg,
        None,
        vec!["whatsapp".to_owned(), "telegram".to_owned()],
    );
    wizard.api_key = "sk-super-secret-value-12345".to_owned();
    wizard.step = WizardStep::Step2Plugins;
    wizard.plugin_enabled[0] = true;
    state.wizard = Some(wizard);
    state.settings.close();

    // Render wizard first to ensure API key is masked in output
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let term = render_headless(120, 30, &state, &mut effects, &mut hit);
    let wiz_text = buffer_text(term.backend().buffer());
    assert!(!wiz_text.contains("sk-super-secret-value-12345"));

    // Press Enter on Step 2 to finish wizard
    let actions = state.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(actions.contains(&AppAction::CompleteSetupWizard));
    assert!(state.wizard.is_none());
    assert!(state.secrets_summary.model_api_key_set);

    let secrets_raw = std::fs::read_to_string(&state.paths.secrets_path).expect("secrets.toml");
    assert!(secrets_raw.contains("model_api_key"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&state.paths.secrets_path)
            .expect("meta")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn live_spinner_phase_and_elapsed_timer_render_while_busy_and_hide_when_idle() {
    assert_eq!(TuiAppState::format_elapsed(Duration::from_secs(0)), "0s");
    assert_eq!(TuiAppState::format_elapsed(Duration::from_secs(3)), "3s");
    assert_eq!(TuiAppState::format_elapsed(Duration::from_secs(45)), "45s");
    assert_eq!(TuiAppState::format_elapsed(Duration::from_secs(65)), "1m 05s");
    assert_eq!(TuiAppState::format_elapsed(Duration::from_secs(132)), "2m 12s");

    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);
    state.thinking = true;
    state.thinking_phase = "Reasoning…".to_owned();
    state.thinking_elapsed_ms = 65_000;

    let mut effects = TuiEffects::new();
    effects.advance_tick();
    let mut hit = HitRegions::default();
    let term_busy = render_headless(120, 24, &state, &mut effects, &mut hit);
    let busy_text = buffer_text(term_busy.backend().buffer());

    assert!(busy_text.contains("Reasoning…"), "busy phase label rendered");
    assert!(busy_text.contains("1m 05s ›"), "live elapsed timer rendered beside spinner");

    // Idle hides phase line and elapsed timer
    state.thinking = false;
    let term_idle = render_headless(120, 24, &state, &mut effects, &mut hit);
    let idle_text = buffer_text(term_idle.backend().buffer());
    assert!(!idle_text.contains("Reasoning…"));
    assert!(!idle_text.contains("1m 05s ›"));
}

fn make_png_header(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(b"IHDR");
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&[8, 6, 0, 0, 0]);
    out
}

#[test]
fn image_dimension_parser_and_rounded_attachment_card_rendering() {
    assert_eq!(format_byte_size(512), "512 B");
    assert_eq!(format_byte_size(42 * 1024), "42 KB");
    assert_eq!(format_byte_size(20 * 1024 * 1024), "20.0 MB");

    // PNG 640x640
    let png = make_png_header(640, 640);
    assert_eq!(parse_image_dimensions(&png), Some((640, 640)));

    // GIF 320x240
    let mut gif = Vec::from(&b"GIF89a"[..]);
    gif.extend_from_slice(&320u16.to_le_bytes());
    gif.extend_from_slice(&240u16.to_le_bytes());
    assert_eq!(parse_image_dimensions(&gif), Some((320, 240)));

    // JPEG SOF0 800x600
    let jpeg = [
        0xFF, 0xD8, // SOI
        0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, // APP0
        0xFF, 0xC0, 0x00, 0x0B, 0x08, // SOF0
        0x02, 0x58, // height = 600
        0x03, 0x20, // width = 800
        0x03, 0x01, 0x11, 0x00,
    ];
    assert_eq!(parse_image_dimensions(&jpeg), Some((800, 600)));

    // WebP VP8 lossy 1024x768
    let mut vp8 = Vec::from(&b"RIFF\x00\x00\x00\x00WEBPVP8 "[..]);
    vp8.extend_from_slice(&20u32.to_le_bytes()); // chunk size
    vp8.extend_from_slice(&[0x00, 0x00, 0x00]); // frame tag
    vp8.extend_from_slice(&[0x9D, 0x01, 0x2A]); // start code
    vp8.extend_from_slice(&1024u16.to_le_bytes());
    vp8.extend_from_slice(&768u16.to_le_bytes());
    assert_eq!(parse_image_dimensions(&vp8), Some((1024, 768)));

    // WebP VP8L lossless 512x256
    let mut vp8l = Vec::from(&b"RIFF\x00\x00\x00\x00WEBPVP8L"[..]);
    vp8l.extend_from_slice(&12u32.to_le_bytes());
    vp8l.push(0x2F);
    let bits: u32 = (512 - 1) | ((256 - 1) << 14);
    vp8l.extend_from_slice(&bits.to_le_bytes());
    assert_eq!(parse_image_dimensions(&vp8l), Some((512, 256)));

    // WebP VP8X extended 1920x1080
    let mut vp8x = Vec::from(&b"RIFF\x00\x00\x00\x00WEBPVP8X"[..]);
    vp8x.extend_from_slice(&10u32.to_le_bytes());
    vp8x.extend_from_slice(&[0u8; 4]); // flags + reserved
    let w_m1 = (1920u32 - 1).to_le_bytes();
    let h_m1 = (1080u32 - 1).to_le_bytes();
    vp8x.extend_from_slice(&w_m1[..3]);
    vp8x.extend_from_slice(&h_m1[..3]);
    assert_eq!(parse_image_dimensions(&vp8x), Some((1920, 1080)));

    // Images ALWAYS attach even when workspace/pdf-reader plugins are Stopped
    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);
    let img_path = dir.path().join("shot.png");
    std::fs::write(&img_path, &png).expect("write png");

    state.handle_paste(&format!("'{}'", img_path.display()));
    assert_eq!(state.composer.pending_attachments.len(), 1);
    assert!(
        state.composer.pending_attachments[0].bytes.is_none(),
        "disk-backed image must not load full file into RAM"
    );
    assert!(state.composer.text.contains("📷 #1"));

    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let term = render_headless(120, 30, &state, &mut effects, &mut hit);
    let rendered = buffer_text(term.backend().buffer());
    assert!(rendered.contains("╭─ 📷 #1"), "rendered:\n{rendered}");
    assert!(rendered.contains("╰─ 640x640"), "rendered:\n{rendered}");
    assert_eq!(hit.attachment_cards.len(), 1);
}

#[test]
fn pdf_requires_pdf_reader_or_workspace_and_streams_on_submit() {
    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);
    // Add pdf-reader and workspace to plugin list (Stopped by default)
    state.gateway_cfg.plugins.push(PluginConfig {
        id: "pdf-reader".to_owned(),
        enabled: true,
        kind: PluginKind::Capability,
        transport: BodyTransportConfig::Stdio {
            command: "plugin-pdf-reader".to_owned(),
            args: vec![],
        },
        dependencies: vec![],
        optional_dependencies: vec![],
    });
    state.gateway_cfg.plugins.push(PluginConfig {
        id: "workspace".to_owned(),
        enabled: true,
        kind: PluginKind::Capability,
        transport: BodyTransportConfig::Stdio {
            command: "plugin-workspace".to_owned(),
            args: vec![],
        },
        dependencies: vec![],
        optional_dependencies: vec![],
    });
    state.refresh_plugins_from_config();

    let pdf_path = dir.path().join("annual_report.pdf");
    let pdf_bytes = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog >>\nendobj\n%%EOF\n";
    std::fs::write(&pdf_path, pdf_bytes).expect("write pdf");

    // 1. Both pdf-reader and workspace Stopped -> refused cleanly with toast
    state.execute_slash_command(&format!("/attach {}", pdf_path.display()));
    assert!(state.composer.pending_attachments.is_empty());
    let toast = state.toast.as_ref().expect("toast");
    assert!(toast.is_error);
    assert!(toast.text.contains("pdf-reader"), "toast: {}", toast.text);

    // 2. Start pdf-reader -> attaches as 📄 #1 card without loading PDF bytes into RAM
    state.set_plugin_state("pdf-reader", PluginUiState::Running);
    state.execute_slash_command(&format!("/attach {}", pdf_path.display()));
    assert_eq!(state.composer.pending_attachments.len(), 1);
    assert!(state.composer.pending_attachments[0].bytes.is_none());
    assert!(state.composer.text.contains("📄 #1"));

    // Verify Percept staging copies via streaming fs::copy without dumping PDF bytes or raw host path
    let (payload, handles) = build_percept_payload_and_attachments(
        "Summarize 📄 #1",
        &state.composer.pending_attachments,
        &state.paths.config_path,
        &state.paths.data_dir,
        "per_owner",
        false,
        Time::from_millis(12345),
    );
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].modality_slot, ModalitySlot::SensorBlob);
    assert_eq!(handles[0].byte_len, pdf_bytes.len() as u64);
    assert!(!payload.contains("%PDF-1.7"), "must not inline PDF bytes into prompt");
    assert!(
        !payload.contains(&dir.path().display().to_string()),
        "must not leak raw host path into percept: {payload}"
    );
    assert!(payload.contains("[Attachment #1: PDF \"annual_report.pdf\""));
    let staged_disk = state
        .paths
        .data_dir
        .join("tui_attachments")
        .join("12345_1_annual_report.pdf");
    assert_eq!(std::fs::read(&staged_disk).expect("staged pdf"), pdf_bytes);

    // 3. Render card and click-to-detach
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let viewport = Rect::new(0, 0, 120, 30);
    let term = render_headless(viewport.width, viewport.height, &state, &mut effects, &mut hit);
    let rendered = buffer_text(term.backend().buffer());
    assert!(rendered.contains("╭─ 📄 #1"), "rendered:\n{rendered}");
    assert!(rendered.contains("annual_report.pdf"), "rendered:\n{rendered}");
    let card_rect = hit.attachment_cards[0].0;
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: card_rect.x + 1,
            row: card_rect.y + 1,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert!(state.composer.pending_attachments.is_empty());
    assert!(!state.composer.text.contains("📄 #1"));
}

#[test]
fn workspace_gates_non_media_host_files_and_rewrites_to_inbox_path() {
    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);
    state.gateway_cfg.plugins.push(PluginConfig {
        id: "workspace".to_owned(),
        enabled: true,
        kind: PluginKind::Capability,
        transport: BodyTransportConfig::Stdio {
            command: "plugin-workspace".to_owned(),
            args: vec![],
        },
        dependencies: vec![],
        optional_dependencies: vec![],
    });
    state.refresh_plugins_from_config();

    let host_dir = dir.path().join("host_downloads");
    std::fs::create_dir_all(&host_dir).expect("mkdir");
    let jar_path = host_dir.join("ViaVersion-5.12.2.jar");
    let txt_path = host_dir.join("notes.txt");
    std::fs::write(&jar_path, b"PK\x03\x04fake-jar-bytes").expect("write jar");
    std::fs::write(&txt_path, b"hello workspace").expect("write txt");

    // 1. Workspace plugin Stopped: host path NOT inserted, no card created, clear toast
    state.handle_paste(&jar_path.display().to_string());
    assert!(state.composer.text.is_empty(), "must not insert raw host path");
    assert!(state.composer.pending_attachments.is_empty());
    let toast = state.toast.as_ref().expect("toast");
    assert!(toast.text.contains("workspace"), "toast: {}", toast.text);

    state.handle_paste(&txt_path.display().to_string());
    assert!(state.composer.text.is_empty(), "must not insert raw .txt host path");
    assert!(state.composer.pending_attachments.is_empty());

    // 2. Workspace plugin Running: copies into <data_dir>/plugins/workspace/workspaces/<actor>/inbox/
    //    and rewrites composer text to `inbox/ViaVersion-5.12.2.jar`
    state.set_plugin_state("workspace", PluginUiState::Running);
    state.handle_paste(&jar_path.display().to_string());
    assert!(state.composer.pending_attachments.is_empty());
    assert_eq!(state.composer.text, "inbox/ViaVersion-5.12.2.jar");
    assert!(
        !state.composer.text.contains("host_downloads"),
        "raw host path must never enter composer"
    );
    let copied = state
        .paths
        .data_dir
        .join("plugins/workspace/workspaces/per_owner/inbox/ViaVersion-5.12.2.jar");
    assert_eq!(std::fs::read(&copied).expect("copied jar"), b"PK\x03\x04fake-jar-bytes");
    assert!(
        state
            .toast
            .as_ref()
            .is_some_and(|t| t.text.contains("inbox/ViaVersion-5.12.2.jar") && !t.is_error)
    );

    // Deduplication when a DIFFERENT file has the same filename
    let jar_v2_dir = dir.path().join("other");
    std::fs::create_dir_all(&jar_v2_dir).expect("mkdir");
    let jar_v2 = jar_v2_dir.join("ViaVersion-5.12.2.jar");
    std::fs::write(&jar_v2, b"PK\x03\x04different-jar-bytes").expect("write v2");
    state.composer.text.clear();
    state.composer.cursor = 0;
    state.handle_paste(&jar_v2.display().to_string());
    assert_eq!(state.composer.text, "inbox/ViaVersion-5.12.2-2.jar");
}

#[test]
fn long_pasted_text_collapses_into_card_short_text_pastes_inline_and_240kib_cap_holds() {
    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);

    // Short text pastes directly inline
    state.handle_paste("hello world");
    assert!(state.composer.pending_attachments.is_empty());
    assert_eq!(state.composer.text, "hello world");

    // Long text (>= 8 lines) collapses into 📝 #1 card
    state.composer.text.clear();
    state.composer.cursor = 0;
    let long_lines = (1..=12)
        .map(|i| format!("line {i}: rust tokio diagnostics"))
        .collect::<Vec<_>>()
        .join("\n");
    state.handle_paste(&long_lines);
    assert_eq!(state.composer.pending_attachments.len(), 1);
    assert!(state.composer.text.contains("📝 #1"));

    // Submitting keeps timeline bubble compact (NOT dumping 12 lines) while percept expands cleanly
    let actions = state.submit_composer();
    assert_eq!(actions.len(), 1);
    let sent_attachments = match &actions[0] {
        AppAction::SendUserMessage { text, attachments, .. } => {
            assert!(text.contains("📝 #1"));
            assert!(!text.contains("line 12"));
            attachments.clone()
        }
        other => panic!("expected SendUserMessage, got {other:?}"),
    };
    let (percept_payload, _) = build_percept_payload_and_attachments(
        "📝 #1",
        &sent_attachments,
        &state.paths.config_path,
        &state.paths.data_dir,
        "per_owner",
        false,
        Time::from_millis(100),
    );
    assert!(percept_payload.contains("<attached_text id=\"#1\" lines=\"12\">"));
    assert!(percept_payload.contains("line 12: rust tokio diagnostics"));

    // Oversized > 260 KiB long text is safely bounded below 240 KiB
    let huge = "x".repeat(270 * 1024);
    state.handle_paste(&huge);
    assert_eq!(state.composer.pending_attachments.len(), 1);
    let (bounded_payload, _) = build_percept_payload_and_attachments(
        "📝 #1",
        &state.composer.pending_attachments,
        &state.paths.config_path,
        &state.paths.data_dir,
        "per_owner",
        false,
        Time::from_millis(200),
    );
    assert!(
        bounded_payload.len() <= MAX_TUI_PERCEPT_INLINE_BYTES,
        "len {} exceeded 240 KiB cap",
        bounded_payload.len()
    );
    assert!(bounded_payload.contains("truncated to fit 240 KiB cap"));
}

#[test]
fn mouse_drag_select_copies_viewport_text_respects_toggle_and_copy_command_and_osc52() {
    assert_eq!(encode_base64(b"hello"), "aGVsbG8=");
    assert_eq!(
        osc52_sequence_with_tmux("hello", false),
        "\x1b]52;c;aGVsbG8=\x07"
    );
    assert_eq!(
        osc52_sequence_with_tmux("hello", true),
        "\x1bPtmux;\x1b\x1b]52;c;aGVsbG8=\x07\x1b\\"
    );

    let dir = TempDir::new().expect("tempdir");
    let mut state = make_test_state(&dir);
    state.push_timeline_item(
        "direct",
        ChatTimelineItem::AssistantMessage {
            text: "Alpha Beta Gamma\nSecond line here".to_owned(),
            duration_ms: Some(120),
            timestamp_ms: 1000,
        },
    );

    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let viewport = Rect::new(0, 0, 120, 24);
    let _ = render_headless(viewport.width, viewport.height, &state, &mut effects, &mut hit);

    let row_idx = hit
        .chat_viewport_lines
        .iter()
        .position(|l| l.contains("Alpha Beta Gamma"))
        .expect("assistant line in viewport");
    let row_y = hit.chat_area.y + row_idx as u16;
    let col_start = hit.chat_area.x
        + hit.chat_viewport_lines[row_idx]
            .find("Beta")
            .expect("Beta pos") as u16;
    let col_end = col_start + "Beta Gamma".len() as u16 - 1;

    // Single click without drag does NOT copy
    let click_down = state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col_start,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert!(click_down.is_empty());
    let click_up = state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: col_start,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert!(click_up.is_empty());

    // Drag Down -> Drag -> Up emits CopyToClipboard with exact substring
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col_start,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: col_end,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    // Render while dragging to exercise highlight overlay path
    let _ = render_headless(viewport.width, viewport.height, &state, &mut effects, &mut hit);
    let drag_up = state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: col_end,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert_eq!(
        drag_up,
        vec![AppAction::CopyToClipboard("Beta Gamma".to_owned())]
    );

    // Disable copy_on_select -> drag release does NOT emit CopyToClipboard
    state.prefs.copy_on_select = false;
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col_start,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: col_end,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    let disabled_up = state.handle_mouse_event(
        MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: col_end,
            row: row_y,
            modifiers: KeyModifiers::NONE,
        },
        &hit,
        viewport,
    );
    assert!(disabled_up.is_empty());

    // /copy slash command copies latest assistant reply
    let copy_actions = state.execute_slash_command("/copy");
    assert_eq!(
        copy_actions,
        vec![AppAction::CopyToClipboard(
            "Alpha Beta Gamma\nSecond line here".to_owned()
        )]
    );
}

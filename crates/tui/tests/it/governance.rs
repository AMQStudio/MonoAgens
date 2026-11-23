//! Contract tests for governance panels (`/approvals`, `/people`, `/policy`, `/activity`,
//! `/events`, `/status`): keyboard & mouse row selection, wheel + scrollbar drag, backdrop
//! clearing without sidebar/chat bleed-through, and prompt paste isolation.

use crossterm::event::KeyCode;
use monoagens_gateway::{GatewayConfig, ResolvedPaths};
use monoagens_protocol::ActorId;
use monoagens_tui::core::hitmap::{HitTarget, ScrollId};
use monoagens_tui::core::overlay::OverlayId;
use monoagens_tui::views::{GovResponse, GovTag, Panel};
use monoagens_tui::{
    AppAction, HitRegions, LocalChatStore, SecretsUiSummary, TuiAppState, TuiEffects,
    TuiPreferences, render_to_buffer,
};
use ratatui::layout::Rect;
use serde_json::json;
use tempfile::TempDir;

use crate::common::{
    TuiTestHarness, key_press, mouse_click, mouse_drag, mouse_scroll_down, mouse_scroll_up,
    paste_event,
};

fn make_app_state(dir: &TempDir) -> TuiAppState {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).expect("cfg dir");
    std::fs::create_dir_all(&data_dir).expect("data dir");
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

fn find_scrollbar(hit: &HitRegions, target_id: ScrollId, area: Rect) -> Rect {
    for row in area.y..area.bottom() {
        for col in area.x..area.right() {
            if let Some((id, track, _, _)) = hit.hitmap.scrollbar_at(col, row)
                && id == target_id
            {
                return track;
            }
        }
    }
    panic!("expected scrollbar {target_id:?} in {area:?}");
}

fn find_first_target(hit: &HitRegions, target: HitTarget, area: Rect) -> (u16, u16) {
    for row in area.y..area.bottom() {
        for col in area.x..area.right() {
            if hit.hitmap.hit_test(col, row) == Some(&target) {
                return (col, row);
            }
        }
    }
    panic!("expected {target:?} in {area:?}");
}

#[test]
fn governance_activity_clears_backdrop_supports_mouse_click_wheel_scrollbar_and_prompt_paste() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    state.chats.chats[0].title = "SidebarLeakSentinel".to_owned();
    state.chat_scroll = 12;
    state.auto_scroll = false;

    let _ = state.open_gov(Panel::Activity);
    assert!(state.has_open_overlay());

    let rows: Vec<_> = (0..40)
        .map(|i| {
            json!({
                "trace": 100 + i,
                "timestamp_ms": 1_700_000_000_000u64 + i * 1000,
                "plugin": "whatsapp",
                "place": "whatsapp:room",
                "actor": format!("guest_{i:02}"),
                "summary": format!("msg {i:02}"),
                "outcome": "replied",
            })
        })
        .collect();
    let _ = state.gov.apply(GovResponse {
        tag: GovTag::Activity,
        result: Ok(json!({ "rows": rows })),
    });

    let viewport = Rect::new(0, 0, 120, 20);
    let mut harness = TuiTestHarness::new(120, 20);
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    harness.draw_with(|area, buf| {
        render_to_buffer(area, buf, &state, &mut effects, &mut hit, 16);
    });

    // Governance overlay must completely clear underlying sidebar/chat text.
    harness.assert_text_absent_in_rect(viewport, "SidebarLeakSentinel");
    harness.assert_text_in_rect(viewport, "Adapter activity (read-only)");
    harness.assert_text_in_rect(viewport, "guest_00");
    assert_eq!(
        hit.hitmap.hit_test(0, 0),
        Some(&HitTarget::OverlayBody(OverlayId::Governance))
    );

    // Clicking rendered row 3 selects guest_03.
    let (rx, ry) = find_first_target(&hit, HitTarget::ListRow(3), viewport);
    state.handle_mouse_event(mouse_click(rx, ry), &hit, viewport);
    assert_eq!(state.gov.activity.nav.selected, 3);

    // Wheel scrolling stays inside governance and never leaks to chat_scroll.
    state.handle_mouse_event(mouse_scroll_down(60, 10), &hit, viewport);
    assert_eq!(state.chat_scroll, 12);
    assert!(state.gov.activity.nav.selected > 3);
    state.handle_mouse_event(mouse_scroll_up(60, 10), &hit, viewport);

    // Scrollbar drag reaches the bottom of the activity feed.
    let sb = find_scrollbar(&hit, ScrollId::GovernanceList, viewport);
    state.handle_mouse_event(mouse_click(sb.x, sb.y), &hit, viewport);
    state.handle_mouse_event(mouse_drag(sb.x, sb.bottom() - 1), &hit, viewport);
    assert!(state.gov.activity.nav.offset > 10);

    // `/` opens search prompt and Event::Paste updates activity search without leaking to composer.
    state.handle_key_event(key_press(KeyCode::Char('/'), crossterm::event::KeyModifiers::NONE));
    assert!(state.gov.prompt.is_some());
    state.handle_terminal_event(paste_event("guest_39\r\n"), &hit, viewport);
    assert_eq!(state.gov.activity.search, "guest_39");
    assert!(state.composer.text.is_empty());

    // Esc cancels prompt, second Esc closes governance panel.
    state.handle_key_event(key_press(KeyCode::Esc, crossterm::event::KeyModifiers::NONE));
    assert!(state.gov.prompt.is_none());
    state.handle_key_event(key_press(KeyCode::Esc, crossterm::event::KeyModifiers::NONE));
    assert!(!state.gov.is_open());
    assert!(!state.has_open_overlay());
}

#[test]
fn governance_approvals_policy_people_events_and_status_render_and_navigate() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);
    let mut harness = TuiTestHarness::new(100, 20);
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();

    for panel in [
        Panel::Approvals,
        Panel::People,
        Panel::Policy,
        Panel::Events,
        Panel::Status,
    ] {
        let _ = state.open_gov(panel);
        match panel {
            Panel::Approvals => {
                let _ = state.gov.apply(GovResponse {
                    tag: GovTag::Approvals,
                    result: Ok(json!({
                        "approvals": [
                            {
                                "action_id": "act_1",
                                "capability_id": "chat.speak",
                                "places": ["cli:terminal"],
                                "requester": "guest_a",
                                "origin": "whatsapp",
                                "timestamp_ms": 1_000,
                                "expires_at_ms": 50_000,
                                "taint": "clean",
                                "h_act": "h1",
                                "state_epoch": 1
                            },
                            {
                                "action_id": "act_2",
                                "capability_id": "web.search",
                                "places": ["cli:terminal"],
                                "requester": "guest_b",
                                "origin": "telegram",
                                "timestamp_ms": 2_000,
                                "expires_at_ms": 50_000,
                                "taint": "clean",
                                "h_act": "h2",
                                "state_epoch": 1
                            }
                        ]
                    })),
                });
            }
            Panel::People => {
                let _ = state.gov.apply(GovResponse {
                    tag: GovTag::People,
                    result: Ok(json!({
                        "configured_persons": [
                            {"actor_id": "alice", "is_owner": false, "roles": ["member"], "accounts": []}
                        ],
                        "active_links": [],
                        "fallback_role": "guest"
                    })),
                });
            }
            Panel::Policy => {
                let _ = state.gov.apply(GovResponse {
                    tag: GovTag::Policy,
                    result: Ok(json!({
                        "valve_mode": "enforcing",
                        "symcc_proof": {"status": "verified", "solver": "z3", "counterexamples": 0},
                        "absolute_lines": ["No secrets leave"],
                        "rules": [{"id": "permit_chat", "kind": "permit", "source": "permit(principal, action, resource);"}],
                        "rbac": {"fallback_role": "guest", "place_rules": 0, "roles": ["guest", "member"], "actor_roles": []}
                    })),
                });
            }
            Panel::Events => {
                let _ = state.gov.apply(GovResponse {
                    tag: GovTag::Events,
                    result: Ok(json!({
                        "events": [
                            {"id": 1, "seq": 1, "kind": "percept", "timestamp_ms": 1000, "place": "cli:terminal", "actor": "owner", "summary": "hello"}
                        ],
                        "matching": 1
                    })),
                });
            }
            Panel::Status => {
                let _ = state.gov.apply(GovResponse {
                    tag: GovTag::Status,
                    result: Ok(json!({
                        "owner": {"pid": 1234, "mode": "hosting"},
                        "data_dir": "/tmp/mono",
                        "lock_path": "/tmp/mono/lock",
                        "provider": "openai",
                        "model": "gpt-4o-mini",
                        "embedder": "none",
                        "queue_depth": 0,
                        "budget": {"used": 10, "limit": 1000},
                        "plugins": []
                    })),
                });
            }
            Panel::Activity => unreachable!(),
        }

        harness.draw_with(|area, buf| {
            render_to_buffer(area, buf, &state, &mut effects, &mut hit, 16);
        });
        harness.assert_text_in_rect(harness.area(), panel.title());
        assert_eq!(
            hit.hitmap.hit_test(0, 1),
            Some(&HitTarget::ListRow(0))
        );

        state.handle_key_event(key_press(KeyCode::Esc, crossterm::event::KeyModifiers::NONE));
        assert!(!state.gov.is_open());
    }
}

#[test]
fn people_panel_adds_owner_account_and_promotes_recent_guest_via_config_apply() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);

    state.execute_slash_command("/people");
    let _ = state.gov.apply(GovResponse {
        tag: GovTag::People,
        result: Ok(json!({
            "configured_persons": [],
            "active_links": [],
            "fallback_role": "guest"
        })),
    });
    let _ = state.gov.apply(GovResponse {
        tag: GovTag::PeopleRecent,
        result: Ok(json!({
            "recent": [{
                "body_id": "body-whatsapp",
                "account_id": "wa:user:6285@s.whatsapp.net",
                "place": "wa:dm:x",
                "count": 2,
                "last_ms": 1
            }]
        })),
    });

    // `a` pre-fills owner account link and Enter emits config.apply.
    state.handle_key_event(key_press(KeyCode::Char('a'), crossterm::event::KeyModifiers::NONE));
    let acts = state.handle_key_event(key_press(KeyCode::Enter, crossterm::event::KeyModifiers::NONE));
    assert!(acts.iter().any(|a| matches!(
        a,
        AppAction::Gov(r) if r.tag == GovTag::PeopleApply && r.method == "config.apply"
    )));
}

#[test]
fn approvals_grant_confirm_and_lab_gated_replay_export_work() {
    let dir = TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("tempdir");
    let mut state = make_app_state(&dir);

    // Open /approvals with one pending hold item.
    state.execute_slash_command("/approvals");
    let _ = state.gov.apply(GovResponse {
        tag: GovTag::Approvals,
        result: Ok(json!({
            "approvals": [{
                "action_id": "act_1",
                "h_act": "h1",
                "state_epoch": 1,
                "capability": "workspace.run",
                "place_id": "tui:local",
                "reason": "shell command",
                "summary": "run cargo test"
            }]
        })),
    });
    // `g` arms confirmation; second `y` emits ResolveApproval { approved: true }.
    state.handle_key_event(key_press(KeyCode::Char('g'), crossterm::event::KeyModifiers::NONE));
    let acts = state.handle_key_event(key_press(KeyCode::Char('y'), crossterm::event::KeyModifiers::NONE));
    assert!(acts.iter().any(|a| matches!(a, AppAction::ResolveApproval { approved: true, .. })));

    // /events replay (`R`) only triggers in lab_mode.
    state.handle_key_event(key_press(KeyCode::Esc, crossterm::event::KeyModifiers::NONE));
    state.execute_slash_command("/events");
    let _ = state.gov.apply(GovResponse {
        tag: GovTag::Events,
        result: Ok(json!({
            "events": [{ "seq": 1, "kind": "turn.start", "actor": "per_owner", "detail": "ok" }],
            "next": null
        })),
    });
    let non_lab = state.handle_key_event(key_press(KeyCode::Char('R'), crossterm::event::KeyModifiers::SHIFT));
    assert!(non_lab.is_empty());
    state.lab_mode = true;
    state.gov.lab_mode = true;
    let lab_acts = state.handle_key_event(key_press(KeyCode::Char('R'), crossterm::event::KeyModifiers::SHIFT));
    assert!(!lab_acts.is_empty());
}

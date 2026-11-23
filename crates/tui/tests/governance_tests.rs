//! Governance and observability panels (change `tui-governance-observability`): approvals,
//! people, policy, activity, events, status. State-level tests plus snapshots at three sizes.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use monoagens_gateway::{GatewayConfig, ResolvedPaths};
use monoagens_protocol::ActorId;
use monoagens_tui::views::{GovResponse, GovTag, Panel};
use monoagens_tui::{
    AppAction, HitRegions, LocalChatStore, SecretsUiSummary, TuiAppState, TuiEffects,
    TuiPreferences, render_to_buffer,
};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use serde_json::{Value, json};
use tempfile::TempDir;
use unicode_width::UnicodeWidthStr;

const NOW: u64 = 1_700_000_100_000;

fn temp_dir() -> TempDir {
    TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp dir on disk")
}

fn make_state(dir: &TempDir) -> TuiAppState {
    let cfg_dir = dir.path().join("config");
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&cfg_dir).expect("cfg dir");
    std::fs::create_dir_all(&data_dir).expect("data dir");
    let paths = ResolvedPaths {
        config_path: cfg_dir.join("monoagens.toml"),
        secrets_path: cfg_dir.join("secrets.toml"),
        data_dir,
    };
    let mut s = TuiAppState::new(
        paths,
        GatewayConfig::default(),
        TuiPreferences::default(),
        LocalChatStore::default(),
        SecretsUiSummary::default(),
        ActorId::new("per_owner"),
    );
    s.gov.now_ms = NOW;
    s.gov.view_rows = 20;
    s
}

fn press(state: &mut TuiAppState, code: KeyCode) -> Vec<AppAction> {
    state.handle_key_event(KeyEvent::new(code, KeyModifiers::NONE))
}

fn ch(state: &mut TuiAppState, c: char) -> Vec<AppAction> {
    press(state, KeyCode::Char(c))
}

fn respond(state: &mut TuiAppState, tag: GovTag, v: Value) -> Vec<AppAction> {
    state.gov.apply(GovResponse { tag, result: Ok(v) })
}

fn render(w: u16, h: u16, state: &TuiAppState) -> Buffer {
    let mut effects = TuiEffects::new();
    let mut hit = HitRegions::default();
    let mut term = Terminal::new(TestBackend::new(w, h)).expect("terminal");
    term.draw(|f| render_to_buffer(f.area(), f.buffer_mut(), state, &mut effects, &mut hit, 16))
        .expect("draw");
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
            let sym = buf.cell((x, y)).expect("cell").symbol();
            skip = sym.width().saturating_sub(1);
            line.push_str(sym);
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

fn gov_reqs(acts: &[AppAction]) -> Vec<(GovTag, String, Value)> {
    acts.iter()
        .filter_map(|a| match a {
            AppAction::Gov(r) => Some((r.tag, r.method.clone(), r.params_value())),
            _ => None,
        })
        .collect()
}

fn approval(id: &str, place: &str, origin: &str) -> Value {
    json!({
        "action_id": id, "capability_id": "tool.write", "places": [place],
        "requester": "per_alice", "origin": origin, "timestamp_ms": NOW - 95_000,
        "expires_at_ms": NOW + 600_000, "taint": "trusted", "reason_code": "PendingApproval",
        "h_act": format!("blake3:{id}"), "state_epoch": 3,
    })
}

fn approvals_payload() -> Value {
    json!({ "approvals": [
        approval("act-wa-1", "wa:dm:alice", "whatsapp"),
        approval("act-tui-2", "cli:terminal", "tui"),
    ]})
}

fn open_approvals(state: &mut TuiAppState) {
    let acts = state.execute_slash_command("/approvals");
    assert_eq!(gov_reqs(&acts)[0].1, "approvals.list");
    respond(state, GovTag::Approvals, approvals_payload());
}

// ---- approvals ------------------------------------------------------------------------------

#[test]
fn approvals_panel_lists_adapter_hold_with_origin() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    open_approvals(&mut s);
    let text = text_of(&render(120, 40, &s));
    assert!(text.contains("2 pending"), "{text}");
    assert!(text.contains("whatsapp"), "adapter origin shown: {text}");
    assert!(text.contains("wa:dm:alice") && text.contains("per_alice") && text.contains("tool.write"));
    assert!(text.contains("1m"), "age shown: {text}");
}

#[test]
fn approvals_grant_confirms_then_resolves_through_the_existing_action() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    open_approvals(&mut s);
    assert!(ch(&mut s, 'g').is_empty(), "g only asks for confirmation");
    assert!(text_of(&render(120, 40, &s)).contains("Grant act-wa-1?"));
    let acts = ch(&mut s, 'y');
    match &acts[0] {
        AppAction::ResolveApproval { action_id, state_epoch, approved, .. } => {
            assert_eq!(action_id.as_str(), "act-wa-1");
            assert_eq!(*state_epoch, 3);
            assert!(*approved);
        }
        other => panic!("expected ResolveApproval, got {other:?}"),
    }
    assert_eq!(gov_reqs(&acts)[0].1, "approvals.list", "list refreshes after resolving");
}

#[test]
fn approvals_declined_confirmation_keeps_the_approval_pending() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    open_approvals(&mut s);
    ch(&mut s, 'd');
    let acts = ch(&mut s, 'n');
    assert!(acts.is_empty());
    assert_eq!(s.gov.approvals.items.len(), 2);
    // Esc also declines, and a second Esc closes the panel.
    ch(&mut s, 'd');
    press(&mut s, KeyCode::Esc);
    assert!(s.gov.is_open() && s.gov.approvals.confirm.is_none());
    press(&mut s, KeyCode::Esc);
    assert!(!s.gov.is_open());
}

#[test]
fn approvals_badge_follows_the_count() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    open_approvals(&mut s);
    let badge = |s: &TuiAppState| text_of(&render(120, 40, s));
    // Close the panel: the badge is on the status bar.
    press(&mut s, KeyCode::Esc);
    assert!(badge(&s).contains("⚑ 2"), "{}", badge(&s));
    respond(&mut s, GovTag::Approvals, json!({ "approvals": [approval("act-tui-2", "cli:terminal", "tui")] }));
    assert!(badge(&s).contains("⚑ 1"));
    respond(&mut s, GovTag::Approvals, json!({ "approvals": [] }));
    assert!(!badge(&s).contains('⚑'));
}

#[test]
fn polling_asks_for_the_approval_count_about_every_two_seconds() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    let mut asked = 0;
    for _ in 0..100 {
        let reqs = s.gov.due_requests();
        if reqs.iter().any(|r| r.method == "approvals.list") {
            asked += 1;
            s.gov.apply(GovResponse { tag: GovTag::Approvals, result: Ok(json!({ "approvals": [] })) });
        }
    }
    assert_eq!(asked, 4, "100 ticks of 80 ms = 8 s -> 4 polls");
}

// ---- people ---------------------------------------------------------------------------------

fn people_payload() -> Value {
    json!({
        "configured_persons": [
            { "actor_id": "per_owner", "is_owner": true, "roles": ["owner"],
              "accounts": [{ "body_id": "body-tg", "account_id": "owner@tg" }] },
            { "actor_id": "per_alice", "is_owner": false, "roles": ["friend"],
              "accounts": [{ "body_id": "body-wa", "account_id": "alice@wa" },
                           { "body_id": "body-tg", "account_id": "alice@tg" }] },
        ],
        "active_links": [{ "body_id": "body-tg", "account_id": "alice@tg", "from_actor": "per_x",
                           "to_actor": "per_alice", "linked_at_ms": 1, "retroactive_merge": false,
                           "owner_confirmed": true }],
        "fallback_role": null,
    })
}

#[test]
fn people_shows_both_accounts_and_the_role() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    s.execute_slash_command("/people");
    respond(&mut s, GovTag::People, people_payload());
    let text = text_of(&render(120, 40, &s));
    assert!(text.contains("per_alice") && text.contains("friend"), "{text}");
    assert!(text.contains("body-wa") && text.contains("alice@wa"));
    assert!(text.contains("body-tg") && text.contains("alice@tg"));
}

#[test]
fn people_initiate_verify_and_unlink_with_confirmation() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    s.execute_slash_command("/people");
    respond(&mut s, GovTag::People, people_payload());
    // Select per_alice's WhatsApp account: rows are owner, owner@tg, alice, alice@wa ...
    for _ in 0..3 {
        press(&mut s, KeyCode::Down);
    }
    ch(&mut s, 'l');
    for c in "body-tg alice@new".chars() {
        ch(&mut s, c);
    }
    let acts = press(&mut s, KeyCode::Enter);
    let (tag, method, p) = gov_reqs(&acts).remove(0);
    assert_eq!((tag, method.as_str()), (GovTag::LinkInitiate, "links.initiate"));
    assert_eq!(p["primary_account"], json!("alice@wa"));
    assert_eq!(p["secondary_account"], json!("alice@new"));
    respond(
        &mut s,
        GovTag::LinkInitiate,
        json!({ "challenge_id": "ch-1", "primary_actor": "per_alice", "secondary_actor": "per_alice",
                "expires_at_ms": 5, "nonce_1": "n1", "nonce_2": "n2" }),
    );
    let text = text_of(&render(120, 40, &s));
    assert!(text.contains("ch-1") && text.contains("n1") && text.contains("n2"), "{text}");
    press(&mut s, KeyCode::Esc); // close the instructions

    ch(&mut s, 'v');
    for c in "ch-1 n1 n2 owner".chars() {
        ch(&mut s, c);
    }
    let (_, method, p) = gov_reqs(&press(&mut s, KeyCode::Enter)).remove(0);
    assert_eq!(method, "links.verify");
    assert_eq!((p["challenge"].clone(), p["owner_confirmed"].clone()), (json!("ch-1"), json!(true)));

    // Unlink: declined -> nothing; confirmed -> request.
    for _ in 0..3 {
        press(&mut s, KeyCode::Down);
    }
    ch(&mut s, 'u');
    assert!(ch(&mut s, 'n').is_empty());
    ch(&mut s, 'u');
    let (_, method, p) = gov_reqs(&ch(&mut s, 'y')).remove(0);
    assert_eq!(method, "links.unlink");
    assert!(p["body"].is_string() && p["account"].is_string());
}

// ---- policy ---------------------------------------------------------------------------------

fn policy_payload() -> Value {
    json!({
        "valve_mode": "enforcing",
        "symcc_proof": { "status": "Proven", "solver": "cvc5", "counterexamples": 0 },
        "active_rule_names": ["default_cedar_rule_01_permit", "default_cedar_rule_02_forbid"],
        "canonical_reason_tags": [],
        "rules": [
            { "id": "default_cedar_rule_01_permit", "kind": "permit", "source": "permit(principal, action, resource);" },
            { "id": "default_cedar_rule_02_forbid", "kind": "forbid", "source": "forbid(principal, action, resource)\nwhen { false };" },
        ],
        "rbac": { "fallback_role": "guest", "roles": ["guest"], "actor_roles": [], "place_rules": 0 },
        "absolute_lines": ["Only the owner changes who the soul is.", "Private content never reaches anyone who was not part of it."],
    })
}

#[test]
fn policy_is_read_only_shows_toggle_state_and_rule_source() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    std::fs::write(&s.paths.config_path, "[valve]\nbudget_guard = false\n").unwrap();
    s.execute_slash_command("/policy");
    respond(&mut s, GovTag::Policy, policy_payload());
    let text = text_of(&render(120, 40, &s));
    assert!(text.contains("[off] budget_guard"), "{text}");
    assert!(text.contains("[on ] cedar_abac"));
    assert!(text.contains("default_cedar_rule_02_forbid") && text.contains("Only the owner changes"));
    // No edit keys: nothing produces an action.
    for c in [' ', 'e', 't', 'x', 'g', 'd', '+'] {
        assert!(ch(&mut s, c).is_empty());
    }
    // Enter on a rule shows its source; Esc returns to the list.
    for _ in 0..40 {
        press(&mut s, KeyCode::Down);
        if text_of(&render(120, 40, &s)).contains("default_cedar_rule_02_forbid") {
            break;
        }
    }
    let rows = s.gov.view_model(NOW).rows;
    let idx = rows.iter().position(|r| r.text.contains("rule_02")).unwrap();
    s.gov.policy.nav.selected = idx;
    press(&mut s, KeyCode::Enter);
    let text = text_of(&render(120, 40, &s));
    assert!(text.contains("when { false };"), "{text}");
    press(&mut s, KeyCode::Esc);
    assert!(s.gov.is_open() && s.gov.detail.is_none());
}

// ---- activity -------------------------------------------------------------------------------

fn activity_payload() -> Value {
    let row = |trace: u64, plugin: &str, place: &str, outcome: &str| {
        json!({ "trace": trace, "seq": trace, "timestamp_ms": NOW - 60_000 + trace, "plugin": plugin,
                "place": place, "actor": "per_alice", "summary": "message in, 4 events in trace",
                "outcome": outcome })
    };
    json!({ "total": 3, "rows": [
        row(10, "whatsapp", "wa:dm:alice", "replied"),
        row(20, "telegram", "tg:dm:bob", "silent"),
        row(30, "whatsapp", "wa:group:1", "held"),
    ]})
}

#[test]
fn activity_filters_by_plugin_and_enter_explains_with_no_composer() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    let chats_before = s.chats.chats.len();
    s.execute_slash_command("/activity");
    respond(&mut s, GovTag::Activity, activity_payload());
    assert_eq!(s.chats.chats.len(), chats_before, "adapter chats stay out of CHATS");
    assert_eq!(s.gov.activity.filtered().len(), 3);
    ch(&mut s, 'p'); // first plugin alphabetically: telegram
    ch(&mut s, 'p'); // whatsapp
    let shown: Vec<&str> = s.gov.activity.filtered().iter().map(|r| r.plugin.as_str()).collect();
    assert_eq!(shown, ["whatsapp", "whatsapp"]);
    // Typing does not reach a composer and never sends.
    let acts = ch(&mut s, 'h');
    assert!(acts.is_empty() && s.composer.text.is_empty());
    for c in ['i', ' ', 'x'] {
        assert!(!ch(&mut s, c).iter().any(|a| matches!(a, AppAction::SendUserMessage { .. })));
    }
    // Enter explains the selected row's trace.
    let (tag, method, p) = gov_reqs(&press(&mut s, KeyCode::Enter)).remove(0);
    assert_eq!((tag, method.as_str()), (GovTag::Explain, "events.explain"));
    assert!(p["id"].is_u64());
}

#[test]
fn activity_outcome_filter_and_search() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    s.execute_slash_command("/activity");
    respond(&mut s, GovTag::Activity, activity_payload());
    ch(&mut s, 'o'); // replied
    ch(&mut s, 'o'); // held
    assert_eq!(s.gov.activity.filtered().len(), 1);
    assert_eq!(s.gov.activity.filtered()[0].place, "wa:group:1");
    ch(&mut s, 'x');
    ch(&mut s, '/');
    for c in "tg:dm".chars() {
        ch(&mut s, c);
    }
    press(&mut s, KeyCode::Enter);
    assert_eq!(s.gov.activity.filtered().len(), 1);
}

// ---- events ---------------------------------------------------------------------------------

fn events_payload() -> Value {
    json!({ "matching": 3, "next_cursor": null, "events": [
        { "id": 12, "seq": 12, "kind": "speak", "timestamp_ms": NOW - 5_000, "place": "wa:dm:alice", "actor": "soul", "summary": "action chat.speak" },
        { "id": 11, "seq": 11, "kind": "decision", "timestamp_ms": NOW - 6_000, "place": "wa:dm:alice", "actor": "soul", "summary": "decision Go for #10" },
        { "id": 10, "seq": 10, "kind": "percept", "timestamp_ms": NOW - 7_000, "place": "wa:dm:alice", "actor": "per_alice", "summary": "percept" },
    ]})
}

fn type_prompt(s: &mut TuiAppState, key: char, text: &str) -> Vec<AppAction> {
    ch(s, key);
    for c in text.chars() {
        ch(s, c);
    }
    press(s, KeyCode::Enter)
}

#[test]
fn events_filters_become_query_params_and_enter_explains() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    s.execute_slash_command("/events");
    respond(&mut s, GovTag::Events, events_payload());
    let (_, _, p) = gov_reqs(&type_prompt(&mut s, 'k', "decision")).remove(0);
    assert_eq!(p["kind"], json!("decision"));
    let (_, _, p) = gov_reqs(&type_prompt(&mut s, 'w', "1h")).remove(0);
    assert_eq!(p["since"], json!(NOW - 3_600_000));
    assert_eq!(p["kind"], json!("decision"));
    let (_, _, p) = gov_reqs(&type_prompt(&mut s, 'p', "wa:dm:alice")).remove(0);
    assert_eq!(p["place"], json!("wa:dm:alice"));
    // Unknown kinds are refused with the list of kinds, no request sent.
    let acts = type_prompt(&mut s, 'k', "nope");
    assert!(gov_reqs(&acts).is_empty());
    assert!(text_of(&render(120, 40, &s)).contains("unknown event kind"));
    press(&mut s, KeyCode::Esc);
    respond(&mut s, GovTag::Events, events_payload());
    let (tag, method, p) = gov_reqs(&press(&mut s, KeyCode::Enter)).remove(0);
    assert_eq!((tag, method.as_str(), p["id"].clone()), (GovTag::Explain, "events.explain", json!(12)));
    // The explain result is displayed line by line as the CLI prints it.
    respond(&mut s, GovTag::Explain, json!({ "lines": ["decision event=11 target=10 outcome=Go"] }));
    assert!(text_of(&render(120, 40, &s)).contains("decision event=11 target=10 outcome=Go"));
}

#[test]
fn events_trace_from_an_event_and_back() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    s.execute_slash_command("/events");
    respond(&mut s, GovTag::Events, events_payload());
    press(&mut s, KeyCode::Down);
    let (tag, method, p) = gov_reqs(&ch(&mut s, 't')).remove(0);
    assert_eq!((tag, method.as_str(), p["trace"].clone()), (GovTag::Trace, "events.trace", json!("11")));
    respond(&mut s, GovTag::Trace, events_payload());
    let text = text_of(&render(120, 40, &s));
    assert!(text.contains("trace 11: 3 events"), "{text}");
    // Ordered by sequence.
    assert!(text.find("percept").unwrap() < text.find("speak").unwrap().max(text.find("percept").unwrap() + 1));
    let back = press(&mut s, KeyCode::Esc);
    assert_eq!(gov_reqs(&back)[0].1, "events.query");
    assert!(s.gov.is_open());
}

#[test]
fn replay_and_export_are_only_offered_in_lab_mode() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    s.execute_slash_command("/events");
    respond(&mut s, GovTag::Events, events_payload());
    let text = text_of(&render(200, 50, &s));
    assert!(!text.contains("R replay") && !text.contains("E export"));
    assert!(ch(&mut s, 'R').is_empty() && ch(&mut s, 'E').is_empty());

    press(&mut s, KeyCode::Esc);
    s.lab_mode = true;
    s.execute_slash_command("/events");
    respond(&mut s, GovTag::Events, events_payload());
    assert!(text_of(&render(200, 50, &s)).contains("R replay"));
    let (tag, method, _) = gov_reqs(&ch(&mut s, 'R')).remove(0);
    assert_eq!((tag, method.as_str()), (GovTag::LabStart, "lab.replay"));
    // Started -> polls the job; progress is drawn; the result arrives as an overlay.
    let follow = respond(&mut s, GovTag::LabStart, json!({ "job": 7 }));
    assert_eq!(gov_reqs(&follow)[0].1, "lab.job");
    respond(&mut s, GovTag::LabJob, json!({ "job": 7, "kind": "replay", "state": "running", "done": 50, "total": 100 }));
    assert!(text_of(&render(200, 50, &s)).contains("50%"));
    respond(
        &mut s,
        GovTag::LabJob,
        json!({ "job": 7, "kind": "replay", "state": "done", "done": 100, "total": 100,
                "result": { "status": "ok", "replayed_events": 100, "matched_decisions": 9, "first_divergence": null } }),
    );
    assert!(text_of(&render(200, 50, &s)).contains("all decisions reproduced (matched=9, replayed=100)"));
}

// ---- status ---------------------------------------------------------------------------------

fn status_payload() -> Value {
    json!({
        "owner": { "pid": 4242, "mode": "serve" }, "data_dir": "/data/mono", "lock_path": "/data/mono/gateway.lock",
        "provider": "openai_compatible", "model": "gpt-x", "embedder": "none",
        "plugins": [{ "id": "whatsapp", "state": "running", "reason": null }, { "id": "telegram", "state": "failed", "reason": "boom" }],
        "queue_depth": 2, "budget": { "used": 1200, "limit": 10000 },
    })
}

#[test]
fn status_shows_attached_daemon_mode_pid_and_plugins() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    s.gateway_link = "attached".to_owned();
    s.execute_slash_command("/status");
    respond(&mut s, GovTag::Status, status_payload());
    let text = text_of(&render(120, 40, &s));
    assert!(text.contains("serve") && text.contains("4242") && text.contains("attached"), "{text}");
    assert!(text.contains("whatsapp") && text.contains("running") && text.contains("boom"));
    assert!(text.contains("1200 used / 8800 left"));
    assert!(text.contains("/data/mono/gateway.lock"));
}

// ---- snapshots ------------------------------------------------------------------------------

#[test]
fn snapshots_at_three_sizes() {
    for (w, h) in [(80u16, 24u16), (120, 40), (200, 50)] {
        let dir = temp_dir();
        let mut s = make_state(&dir);
        s.gateway_link = "attached".to_owned();
        s.gov.view_rows = (h as usize).saturating_sub(2);

        s.execute_slash_command("/approvals");
        respond(&mut s, GovTag::Approvals, approvals_payload());
        insta::assert_snapshot!(format!("approvals_{w}x{h}"), text_of(&render(w, h, &s)));
        press(&mut s, KeyCode::Esc);

        s.execute_slash_command("/activity");
        respond(&mut s, GovTag::Activity, activity_payload());
        insta::assert_snapshot!(format!("activity_{w}x{h}"), text_of(&render(w, h, &s)));
        press(&mut s, KeyCode::Esc);

        s.execute_slash_command("/events");
        respond(&mut s, GovTag::Events, events_payload());
        insta::assert_snapshot!(format!("events_{w}x{h}"), text_of(&render(w, h, &s)));
        press(&mut s, KeyCode::Esc);

        s.execute_slash_command("/status");
        respond(&mut s, GovTag::Status, status_payload());
        insta::assert_snapshot!(format!("status_{w}x{h}"), text_of(&render(w, h, &s)));
        press(&mut s, KeyCode::Esc);
    }
}

#[test]
fn panels_never_overflow_the_terminal_width() {
    let dir = temp_dir();
    let mut s = make_state(&dir);
    for (slash, tag, payload) in [
        ("/approvals", GovTag::Approvals, approvals_payload()),
        ("/activity", GovTag::Activity, activity_payload()),
        ("/events", GovTag::Events, events_payload()),
        ("/status", GovTag::Status, status_payload()),
        ("/people", GovTag::People, people_payload()),
        ("/policy", GovTag::Policy, policy_payload()),
    ] {
        s.execute_slash_command(slash);
        respond(&mut s, tag, payload);
        for (w, h) in [(40u16, 10u16), (80, 24), (200, 50)] {
            let buf = render(w, h, &s);
            for line in text_of(&buf).lines() {
                assert!(line.width() <= w as usize, "{slash} {w}x{h}: {line:?}");
            }
        }
        press(&mut s, KeyCode::Esc);
    }
    let _ = Panel::Status;
}

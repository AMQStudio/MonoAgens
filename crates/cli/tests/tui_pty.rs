//! PTY-driven tests of the real `monoagens` TUI binary (scripted provider, no network).

mod support;

use std::time::Duration;

use support::fake_provider::{FakeProvider, http_get, http_request};
use support::pty::{PtySession, write_scripted_config};

fn temp_home() -> tempfile::TempDir {
    tempfile::TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp home on disk")
}

#[test]
fn fake_provider_serves_models_and_chat() {
    let fake = FakeProvider::start(&["alpha-1", "beta-2"], "hello from fake");
    let (status, body) = http_get(&fake.base_url(), "/models");
    assert!(status.contains("200"), "{status}");
    let v: serde_json::Value = serde_json::from_str(&body).expect("json");
    let ids: Vec<&str> = v["data"]
        .as_array()
        .expect("data")
        .iter()
        .map(|m| m["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["alpha-1", "beta-2"]);

    let (status, body) = http_request("POST", &fake.base_url(), "/chat/completions", "{}");
    assert!(status.contains("200"), "{status}");
    assert!(body.contains("hello from fake"));
    let (status, _) = http_get(&fake.base_url(), "/nope");
    assert!(status.contains("404"));
}

#[test]
fn tui_boots_with_scripted_provider_and_shows_composer() {
    let home = temp_home();
    let cfg = write_scripted_config(home.path());
    let home_s = home.path().to_str().expect("utf8");
    let mut s = PtySession::spawn(
        env!("CARGO_BIN_EXE_monoagens"),
        &["--config", cfg.to_str().expect("utf8")],
        &[("HOME", home_s), ("XDG_CONFIG_HOME", home_s)],
        24,
        100,
    );
    s.wait_for_text("scripted-test", Duration::from_secs(15));
    s.wait_for_text("enter send", Duration::from_secs(5));
    let screen = s.screen_text();
    assert!(screen.contains("scripted-test"), "status bar:\n{screen}");
    s.send_keys("/quit\r");
    assert_eq!(s.wait_exit(Duration::from_secs(10)), Some(0), "{}", s.screen_text());
}

fn spawn_tui(home: &tempfile::TempDir, rows: u16, cols: u16) -> PtySession {
    let cfg = write_scripted_config(home.path());
    let home_s = home.path().to_str().expect("utf8").to_owned();
    PtySession::spawn(
        env!("CARGO_BIN_EXE_monoagens"),
        &["--config", cfg.to_str().expect("utf8")],
        &[("HOME", &home_s), ("XDG_CONFIG_HOME", &home_s)],
        rows,
        cols,
    )
}

#[test]
fn tui_redraws_after_resize_and_alt_l_and_collapses_small_layouts() {
    let home = temp_home();
    let s = spawn_tui(&home, 24, 100);
    s.wait_for_text("enter send", Duration::from_secs(15));

    s.resize(15, 70);
    s.wait_for_absent("enter send", Duration::from_secs(5)); // hint hidden below 20 rows
    s.wait_for_text("Message Mono", Duration::from_secs(5));

    s.resize(8, 30);
    s.wait_for_text("terminal too small", Duration::from_secs(5));
    assert!(!s.screen_text().contains("Message Mono"));

    s.resize(24, 100);
    s.wait_for_text("enter send", Duration::from_secs(5));
    let mut s = s;
    s.send_keys("\x1bl"); // Alt+L
    s.wait_for_text("enter send", Duration::from_secs(5));
    s.send_keys("/quit\r");
    assert_eq!(s.wait_exit(Duration::from_secs(10)), Some(0), "{}", s.screen_text());
}

#[test]
fn tui_failed_turn_ends_busy_and_reports_the_error() {
    let home = temp_home();
    let mut s = spawn_tui(&home, 24, 100);
    s.wait_for_text("enter send", Duration::from_secs(15));
    s.send_keys("hello there\r");
    s.wait_for_text("hello there", Duration::from_secs(5));
    // The scripted provider has no reply queued, so the model call fails; busy must still end.
    s.wait_for_text("model call failed", Duration::from_secs(12));
    s.wait_for_text("esc to clear", Duration::from_secs(5)); // idle hint is back
    s.send_keys("/quit\r");
    assert_eq!(s.wait_exit(Duration::from_secs(10)), Some(0));
}

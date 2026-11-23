//! PTY tests of the conversation keys of the real `monoagens` TUI binary (scripted provider,
//! no network): the external editor and the composer history search.

mod support;

use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use support::pty::{PtySession, write_scripted_config};

fn temp_home() -> tempfile::TempDir {
    tempfile::TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp home on disk")
}

fn spawn_tui(home: &tempfile::TempDir, editor: Option<&str>) -> PtySession {
    let cfg = write_scripted_config(home.path());
    let home_s = home.path().to_str().expect("utf8").to_owned();
    let mut env = vec![("HOME", home_s.as_str()), ("XDG_CONFIG_HOME", home_s.as_str())];
    if let Some(e) = editor {
        env.push(("EDITOR", e));
        env.push(("VISUAL", ""));
    }
    PtySession::spawn(
        env!("CARGO_BIN_EXE_monoagens"),
        &["--config", cfg.to_str().expect("utf8")],
        &env,
        24,
        100,
    )
}

#[test]
fn ctrl_g_runs_the_editor_on_the_main_screen_and_returns_to_the_tui_with_the_edited_text() {
    let home = temp_home();
    let script = home.path().join("fake-editor.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\necho EDITOR-IS-RUNNING-NOW\nsleep 1\nprintf 'appended by editor\\n' >> \"$1\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut s = spawn_tui(&home, Some(script.to_str().unwrap()));
    s.wait_for_text("enter send", Duration::from_secs(15));
    s.send_keys("my draft");
    s.wait_for_text("my draft", Duration::from_secs(5));
    s.send_keys("\x07"); // Ctrl+G

    // While the editor runs the TUI has left the alternate screen.
    s.wait_for_text("EDITOR-IS-RUNNING-NOW", Duration::from_secs(10));
    assert!(!s.screen_text().contains("enter send"), "TUI hidden while editing:\n{}", s.screen_text());

    // Afterwards the alternate screen is back, with the edited text in the composer.
    s.wait_for_text("appended by editor", Duration::from_secs(10));
    s.wait_for_text("enter send", Duration::from_secs(5));
    let screen = s.screen_text();
    assert!(screen.contains("my draft"), "{screen}");
    assert!(!screen.contains("EDITOR-IS-RUNNING-NOW"), "main screen content is gone:\n{screen}");
    s.send_keys("\x1b"); // clears the composer
    std::thread::sleep(Duration::from_millis(300));
    s.send_keys("/quit\r");
    assert_eq!(s.wait_exit(Duration::from_secs(10)), Some(0), "{}", s.screen_text());
}

#[test]
fn ctrl_g_without_an_editor_shows_a_toast() {
    let home = temp_home();
    let cfg = write_scripted_config(home.path());
    let home_s = home.path().to_str().expect("utf8").to_owned();
    let mut s = PtySession::spawn(
        env!("CARGO_BIN_EXE_monoagens"),
        &["--config", cfg.to_str().expect("utf8")],
        &[
            ("HOME", home_s.as_str()),
            ("XDG_CONFIG_HOME", home_s.as_str()),
            ("EDITOR", ""),
            ("VISUAL", ""),
        ],
        24,
        100,
    );
    s.wait_for_text("enter send", Duration::from_secs(15));
    s.send_keys("\x07");
    s.wait_for_text("No editor", Duration::from_secs(5));
    s.send_keys("/quit\r");
    assert_eq!(s.wait_exit(Duration::from_secs(10)), Some(0), "{}", s.screen_text());
}

#[test]
fn ctrl_r_finds_a_sent_message_in_the_history() {
    let home = temp_home();
    let mut s = spawn_tui(&home, None);
    s.wait_for_text("enter send", Duration::from_secs(15));
    s.send_keys("alpha question\r");
    s.wait_for_text("Model call failed", Duration::from_secs(12));
    s.send_keys("\x12"); // Ctrl+R
    s.send_keys("alp");
    s.wait_for_text("(reverse-i-search) `alp`", Duration::from_secs(5));
    s.send_keys("\r");
    s.wait_for_absent("(reverse-i-search)", Duration::from_secs(5));
    assert!(s.screen_text().matches("alpha question").count() >= 2, "composer holds the match:\n{}", s.screen_text());
    s.send_keys("\x1b");
    std::thread::sleep(Duration::from_millis(300));
    s.send_keys("/quit\r");
    assert_eq!(s.wait_exit(Duration::from_secs(10)), Some(0), "{}", s.screen_text());
}

//! End-to-end tests of gateway ownership and startup robustness through the real `monoagens`
//! binary: attach/detach with `serve`, a hosting TUI that also serves the socket, bodies that are
//! offline, plugin autostart with the supervisor, and a TUI that opens on a bad config.
//! No paid model calls: the provider is a local fake HTTP server (or `scripted`).

mod support;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use support::fake_provider::FakeProvider;
use support::pty::{
    Dot, PtySession, silent_reply_json, speak_reply_json, write_fake_provider_config,
    write_scripted_config,
};

fn temp_home() -> tempfile::TempDir {
    tempfile::TempDir::new_in(env!("CARGO_TARGET_TMPDIR")).expect("temp home on disk")
}

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_monoagens")
}

fn fake_plugin_bin() -> PathBuf {
    let target = Path::new(bin()).parent().expect("target/debug");
    let p = target.join("examples").join("fake_plugin");
    assert!(
        p.exists(),
        "build the fake plugin first: cargo build --example fake_plugin -p monoagens-gateway ({} missing)",
        p.display()
    );
    p
}

fn spawn_tui(home: &Path, cfg: &Path, rows: u16, cols: u16) -> PtySession {
    let h = home.to_str().expect("utf8");
    PtySession::spawn(
        bin(),
        &["--config", cfg.to_str().expect("utf8")],
        &[("HOME", h), ("XDG_CONFIG_HOME", h)],
        rows,
        cols,
    )
}

struct Serve {
    child: Child,
    _stdout: BufReader<std::process::ChildStdout>,
}

impl Serve {
    fn start(home: &Path, cfg: &Path) -> Self {
        let h = home.to_str().expect("utf8");
        let mut child = Command::new(bin())
            .args(["--config", cfg.to_str().expect("utf8"), "serve", "--allow-empty"])
            .env("HOME", h)
            .env("XDG_CONFIG_HOME", h)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn serve");
        let mut out = BufReader::new(child.stdout.take().expect("stdout"));
        let mut line = String::new();
        out.read_line(&mut line).expect("serve banner");
        assert!(line.contains("monoagens serve running"), "banner: {line}");
        Self { child, _stdout: out }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Esc closes an overlay; it is sent alone and followed by a pause so the terminal parser does not
/// read it as the start of an Alt-key sequence together with the next keys.
fn close_overlay(s: &mut PtySession, title: &str) {
    s.send_keys("\x1b");
    s.wait_for_absent(title, Duration::from_secs(5));
}

fn quit(mut s: PtySession) {
    s.send_keys("/quit\r");
    assert_eq!(s.wait_exit(Duration::from_secs(15)), Some(0), "{}", s.screen_text());
}

// ---- 7.1: a bad config opens the TUI instead of exiting ---------------------------------------

#[test]
fn bad_config_opens_the_tui_with_a_banner_and_retry_boots_the_gateway() {
    let home = temp_home();
    let cfg = write_scripted_config(home.path());
    let good = std::fs::read_to_string(&cfg).expect("read");
    std::fs::write(&cfg, good.replace("provider = \"scripted\"", "provider = \"nonsense\"")).unwrap();

    let mut s = spawn_tui(home.path(), &cfg, 30, 110);
    s.wait_for_text("Configuration error", Duration::from_secs(15));
    s.wait_for_text("nonsense", Duration::from_secs(5));
    s.wait_for_text("/retry", Duration::from_secs(5));
    assert!(s.screen_text().contains("model.provider") || s.screen_text().contains("provider"));

    // chat is disabled while the config is broken
    s.send_keys("hello\r");
    s.wait_for_text("Chat is disabled", Duration::from_secs(5));

    // /settings opens on the offending setting
    s.send_keys("/settings\r");
    s.wait_for_text("Provider", Duration::from_secs(5));
    close_overlay(&mut s, "Soul & Brain");

    // fix the file, then retry: the gateway boots and chat works (the scripted provider has no
    // reply queued, so the model call fails, but a turn is started and ends)
    std::fs::write(&cfg, good).unwrap();
    s.send_keys("/retry\r");
    s.wait_for_absent("Configuration error", Duration::from_secs(15));
    s.send_keys("hello again\r");
    s.wait_for_text("Model call failed", Duration::from_secs(15));
    quit(s);
}

// ---- 2.8: attach to serve, detach, attach again ----------------------------------------------

#[test]
fn tui_attaches_to_serve_gets_replies_and_detaching_leaves_serve_running() {
    let home = temp_home();
    let fake = FakeProvider::start(&["fake-model"], &speak_reply_json("pong from serve"));
    let cfg = write_fake_provider_config(home.path(), &fake.base_url(), "");
    let mut serve = Serve::start(home.path(), &cfg);
    let sock = home.path().join("data").join("gateway.sock");
    assert!(sock.exists(), "serve must serve the admin socket");

    let mut tui = spawn_tui(home.path(), &cfg, 30, 110);
    tui.wait_for_text(
        &format!("Attached to the gateway hosted by pid {}", serve.pid()),
        Duration::from_secs(20),
    );
    tui.send_keys("hello over the socket\r");
    tui.wait_for_text("pong from serve", Duration::from_secs(25));
    quit(tui);

    // detaching leaves the daemon (same pid) and its socket in place
    assert!(serve.alive(), "serve must survive the TUI closing");
    assert!(sock.exists());

    // a later TUI attaches again, and chat still works
    let mut again = spawn_tui(home.path(), &cfg, 30, 110);
    again.wait_for_text(
        &format!("Attached to the gateway hosted by pid {}", serve.pid()),
        Duration::from_secs(20),
    );
    again.send_keys("second time\r");
    again.wait_for_text("pong from serve", Duration::from_secs(25));
    quit(again);
    assert!(serve.alive());
}

#[test]
fn hosting_tui_serves_the_socket_and_a_second_tui_attaches_to_it() {
    let home = temp_home();
    let fake = FakeProvider::start(&["fake-model"], &speak_reply_json("pong from host"));
    let cfg = write_fake_provider_config(home.path(), &fake.base_url(), "");

    let mut host = spawn_tui(home.path(), &cfg, 30, 110);
    host.wait_for_text("enter send", Duration::from_secs(20));
    let sock = home.path().join("data").join("gateway.sock");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !sock.exists() {
        assert!(Instant::now() < deadline, "hosting TUI must serve the admin socket");
        std::thread::sleep(Duration::from_millis(50));
    }

    let second = spawn_tui(home.path(), &cfg, 30, 110);
    second.wait_for_text("Attached to the gateway hosted by pid", Duration::from_secs(20));
    // the first TUI owns the terminal place, so the attached one is a monitor only
    second.wait_for_text("Chat is disabled", Duration::from_secs(10));
    quit(second);

    // the host keeps working after the second TUI detached
    host.send_keys("still here\r");
    host.wait_for_text("pong from host", Duration::from_secs(25));
    quit(host);
}

// ---- 2.8: actions for the TUI place while no TUI is attached --------------------------------

#[test]
fn chat_speak_to_the_terminal_place_without_a_tui_gets_a_refused_receipt() {
    use monoagens_gateway::AdminClient;
    use serde_json::{Value, json};

    let home = temp_home();
    let fake = FakeProvider::start(&["fake-model"], &speak_reply_json("nobody listens"));
    let cfg = write_fake_provider_config(home.path(), &fake.base_url(), "");
    let _serve = Serve::start(home.path(), &cfg);

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let mut client = AdminClient::connect(&home.path().join("data")).await.expect("connect");
        client
            .call("turn.submit", json!({ "text": "anybody there?", "place": "cli:terminal" }))
            .await
            .expect("submit");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let page = client.call("events.since", json!({ "from": 0 })).await.expect("events");
            let refused = page["events"].as_array().unwrap().iter().any(|e: &Value| {
                let d = &e["detail"];
                d["detail_kind"] == "receipt"
                    && d["capability_id"] == "chat.speak"
                    && d["outcome"] == "refused"
            });
            if refused {
                return;
            }
            assert!(Instant::now() < deadline, "no refused receipt for chat.speak: {page}");
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    });
}

// ---- 5.1 / 3.2 / 4.x: silent turn, plugin autostart, /log, crash and restart -----------------

#[test]
fn silent_turn_ends_busy_with_the_notice() {
    let home = temp_home();
    let fake = FakeProvider::start(&["fake-model"], &silent_reply_json());
    let cfg = write_fake_provider_config(home.path(), &fake.base_url(), "");
    let mut s = spawn_tui(home.path(), &cfg, 30, 110);
    s.wait_for_text("enter send", Duration::from_secs(20));
    s.send_keys("say nothing\r");
    s.wait_for_text("Mono chose not to reply to this message.", Duration::from_secs(25));
    // the TUI wrote turn.no_reply to the runtime log
    s.send_keys("/log\r");
    s.wait_for_text("turn.no_reply", Duration::from_secs(10));
    close_overlay(&mut s, "Runtime log");
    quit(s);
}

#[test]
fn plugin_autostarts_logs_stderr_and_recovers_after_a_crash() {
    let home = temp_home();
    let fake = FakeProvider::start(&["fake-model"], &silent_reply_json());
    let plugin = fake_plugin_bin();
    // The first run crashes after 2 s with `boom`; the supervisor restarts it after 1 s. A marker
    // file makes only the first run crash (a second run would loop forever otherwise).
    let marker = home.path().join("crashed-once");
    let script = home.path().join("flaky-plugin.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nif [ \"$1\" = status ]; then exit 2; fi\nif [ -e '{m}' ]; then exec '{p}' --body-id body-fake --stderr-lines 'hello-from-plugin'; fi\ntouch '{m}'\nexec '{p}' --body-id body-fake --stderr-lines 'hello-from-plugin|boom' --crash-after-ms 2500\n",
            m = marker.display(),
            p = plugin.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let extra = format!(
        "[plugins.fake]\nkind = \"adapter\"\nenabled = true\ntransport = \"stdio\"\ncommand = \"{}\"\nargs = []\n",
        script.display()
    );
    let cfg = write_fake_provider_config(home.path(), &fake.base_url(), &extra);

    // wide enough for the sidebar (Auto mode opens it from 120 columns)
    let mut s = spawn_tui(home.path(), &cfg, 36, 140);
    s.wait_for_text("fake", Duration::from_secs(20));
    s.wait_for_plugin_dot("fake", Dot::Green, Duration::from_secs(20)); // autostarted
    // crash -> red (reason exited 1: boom) -> green again after the first backoff
    s.wait_for_plugin_dot("fake", Dot::Red, Duration::from_secs(15));
    s.wait_for_plugin_dot("fake", Dot::Green, Duration::from_secs(15));

    s.send_keys("/log\r");
    s.wait_for_text("plugin:fake", Duration::from_secs(10));
    s.wait_for_text("hello-from-plugin", Duration::from_secs(5));
    s.wait_for_text("boom", Duration::from_secs(5));
    // filter to the plugin source: `s` cycles sources, `4` (error only) hides plugin lines
    s.send_keys("4");
    s.wait_for_absent("hello-from-plugin", Duration::from_secs(5));
    s.send_keys("1");
    s.wait_for_text("hello-from-plugin", Duration::from_secs(5));
    close_overlay(&mut s, "Runtime log");
    quit(s);

    // the plugin stderr line is also in the rotating file under the data dir
    let log = std::fs::read_to_string(home.path().join("data").join("logs").join("tui.log"))
        .expect("tui.log written");
    assert!(log.contains("plugin:fake") && log.contains("hello-from-plugin"), "{log}");
}

// ---- 2.7: serve uses the supervisor, the run log and serves the admin socket -------------------

#[test]
fn serve_supervises_plugins_logs_to_gateway_log_and_answers_on_the_socket() {
    use monoagens_gateway::AdminClient;
    use serde_json::{Value, json};

    let home = temp_home();
    let fake = FakeProvider::start(&["fake-model"], &silent_reply_json());
    let plugin = fake_plugin_bin();
    let extra = format!(
        "[plugins.fake]\nkind = \"adapter\"\nenabled = true\ntransport = \"stdio\"\ncommand = \"{}\"\nargs = [\"--body-id\", \"body-fake\", \"--stderr-lines\", \"serve-plugin-line\"]\n",
        plugin.display()
    );
    let cfg = write_fake_provider_config(home.path(), &fake.base_url(), &extra);
    let h = home.path().to_str().unwrap();
    let mut child = Command::new(bin())
        .args(["--config", cfg.to_str().unwrap(), "serve"])
        .env("HOME", h)
        .env("XDG_CONFIG_HOME", h)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn serve");
    let mut out = BufReader::new(child.stdout.take().unwrap());
    let mut banner = String::new();
    let mut plugin_line = String::new();
    out.read_line(&mut banner).unwrap();
    out.read_line(&mut plugin_line).unwrap();
    assert!(banner.contains("connected=1"), "{banner}");
    assert!(
        plugin_line.contains("plugin=fake") && plugin_line.contains("status=connected")
            && plugin_line.contains("body_id=body-fake"),
        "{plugin_line}"
    );

    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let mut c = AdminClient::connect(&home.path().join("data")).await.expect("connect");
        let status = c.call("status", Value::Null).await.unwrap();
        assert_eq!(status["owner"]["mode"], json!("serve"));
        assert_eq!(status["owner"]["pid"], json!(child.id()));
        let plugins = c.call("plugins.list", Value::Null).await.unwrap();
        assert_eq!(plugins["plugins"][0]["id"], "fake");
        assert_eq!(plugins["plugins"][0]["state"], "running");
        assert_eq!(plugins["plugins"][0]["stderr_tail"][0], "serve-plugin-line");
    });
    let log_path = home.path().join("data").join("logs").join("gateway.log");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let text = std::fs::read_to_string(&log_path).unwrap_or_default();
        if text.contains("plugin:fake") && text.contains("serve-plugin-line") {
            break;
        }
        assert!(Instant::now() < deadline, "gateway.log lacks the plugin line: {text}");
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
}

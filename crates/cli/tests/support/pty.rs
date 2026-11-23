//! PTY test driver: spawns the real `monoagens` binary in a pseudo-terminal and parses the
//! screen with a terminal emulator (`vt100`).

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

/// Creates a scripted-provider config in `home` and returns the config path.
pub fn write_scripted_config(home: &Path) -> PathBuf {
    let cfg_path = home.join("monoagens.toml");
    let data_dir = home.join("data");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    let toml = format!(
        "version = 1\ndata_dir = \"{}\"\n\n[soul]\nname = \"MonoPty\"\nseed = 1\n\n[model]\nprovider = \"scripted\"\nmodel = \"scripted-test\"\ndaily_token_budget = 10000\n\n[valve]\nmode = \"enforcing\"\nmax_chain_wakes = 8\n",
        data_dir.display()
    );
    std::fs::write(&cfg_path, toml).expect("write config");
    cfg_path
}

/// Config for the `openai_compatible` provider pointed at a fake provider server (no network),
/// with extra top-level TOML appended (for example `[plugins.x]` tables). Also writes a 0600
/// `secrets.toml` next to it.
pub fn write_fake_provider_config(home: &Path, base_url: &str, extra_toml: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let cfg_path = home.join("monoagens.toml");
    let data_dir = home.join("data");
    std::fs::create_dir_all(&data_dir).expect("mkdir data");
    let toml = format!(
        "version = 1\ndata_dir = \"{}\"\n\n[soul]\nname = \"MonoPty\"\nseed = 1\n\n[model]\nprovider = \"openai_compatible\"\nmodel = \"fake-model\"\ndaily_token_budget = 100000\n\n[valve]\nmode = \"enforcing\"\nmax_chain_wakes = 8\n\n{extra_toml}\n",
        data_dir.display()
    );
    std::fs::write(&cfg_path, toml).expect("write config");
    let secrets = home.join("secrets.toml");
    std::fs::write(
        &secrets,
        format!("[model]\napi_key = \"test-key-not-real\"\nendpoint = \"{base_url}/chat/completions\"\n"),
    )
    .expect("write secrets");
    std::fs::set_permissions(&secrets, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    cfg_path
}

/// Model output that makes the soul answer `text` at the TUI place.
pub fn speak_reply_json(text: &str) -> String {
    serde_json::json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [
            {
                "act": "think", "thought": "answer the owner", "motivation": 4,
                "confidence_0_20": 18, "used_ids": [], "importance": 0.6,
                "basis": "observed", "value": null, "cost": null, "domain": null
            },
            {
                "act": "act", "capability_id": "chat.speak", "places": ["cli:terminal"],
                "people": ["per_owner"], "payload": text.as_bytes(), "motivation": 4,
                "confidence_0_20": 18, "used_ids": [], "expected": null, "domain": null
            }
        ]
    })
    .to_string()
}

/// Model output with a thought but no `chat.speak` (a silent turn).
pub fn silent_reply_json() -> String {
    serde_json::json!({
        "protocol_version": { "major": 1, "minor": 0 },
        "acts": [{
            "act": "think", "thought": "nothing to say", "motivation": 2,
            "confidence_0_20": 18, "used_ids": [], "importance": 0.3,
            "basis": "observed", "value": null, "cost": null, "domain": null
        }]
    })
    .to_string()
}

/// Colour class of a status dot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dot {
    Green,
    Yellow,
    Red,
    Other,
}

fn nz(v: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(v).expect("non-zero size")
}

/// A running binary inside a PTY with a `vt100` screen kept up to date by a reader thread.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    child: Box<dyn Child + Send + Sync>,
    parser: Arc<Mutex<vt100::Parser>>,
    raw: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
}

impl PtySession {
    /// Spawns `program` with `args` and `env` in a `rows` x `cols` PTY.
    pub fn spawn(
        program: &str,
        args: &[&str],
        env: &[(&str, &str)],
        rows: u16,
        cols: u16,
    ) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pty");
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        cmd.env("TERM", "xterm-256color");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let child = pair.slave.spawn_command(cmd).expect("spawn in pty");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("clone reader");
        let writer: Arc<Mutex<Box<dyn Write + Send>>> =
            Arc::new(Mutex::new(pair.master.take_writer().expect("take writer")));
        let writer_bg = Arc::clone(&writer);
        let parser = Arc::new(Mutex::new(vt100::Parser::new(nz(rows), nz(cols), 0)));
        let parser_bg = Arc::clone(&parser);
        let raw = Arc::new(Mutex::new(Vec::new()));
        let raw_bg = Arc::clone(&raw);
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // A real terminal answers a cursor-position query (DSR 6); this one must too.
                        if buf[..n].windows(4).any(|w| w == b"\x1b[6n") {
                            let mut w = writer_bg.lock().expect("writer");
                            let _ = w.write_all(b"\x1b[1;1R");
                            let _ = w.flush();
                        }
                        raw_bg.lock().expect("raw").extend_from_slice(&buf[..n]);
                        parser_bg.lock().expect("parser").process(&buf[..n]);
                    }
                }
            }
        });
        Self {
            master: pair.master,
            writer,
            child,
            parser,
            raw,
            reader: Some(handle),
        }
    }

    /// Everything the program wrote to the terminal so far, escape sequences included.
    pub fn raw_output(&self) -> Vec<u8> {
        self.raw.lock().expect("raw").clone()
    }

    /// Writes raw bytes (keys, escape sequences) to the terminal input.
    pub fn send_keys(&mut self, keys: &str) {
        let mut w = self.writer.lock().expect("writer");
        w.write_all(keys.as_bytes()).expect("write keys");
        w.flush().expect("flush keys");
    }

    /// Current visible screen as text, one line per row, trailing blanks trimmed.
    pub fn screen_text(&self) -> String {
        let parser = self.parser.lock().expect("parser");
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        let mut out = Vec::new();
        for line in screen.rows(0, cols.get()).take(rows.get() as usize) {
            out.push(line.trim_end().to_owned());
        }
        out.join("\n")
    }

    /// Polls the screen until it contains `needle`; panics with the last screen on timeout.
    pub fn wait_for_text(&self, needle: &str, timeout: Duration) {
        if let Err(last) = self.try_wait_for_text(needle, timeout) {
            panic!("timed out after {timeout:?} waiting for {needle:?}; last screen:\n{last}");
        }
    }

    /// Like [`Self::wait_for_text`] but returns the last screen text on timeout.
    pub fn try_wait_for_text(&self, needle: &str, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        loop {
            let screen = self.screen_text();
            if screen.contains(needle) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(screen);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Polls until the screen no longer contains `needle`; panics with the last screen on timeout.
    pub fn wait_for_absent(&self, needle: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            let screen = self.screen_text();
            if !screen.contains(needle) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out after {timeout:?} waiting for {needle:?} to disappear; last screen:\n{screen}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Colour of the status dot on the sidebar row that shows `plugin_id` (`None` when no row).
    pub fn plugin_dot(&self, plugin_id: &str) -> Option<Dot> {
        let parser = self.parser.lock().expect("parser");
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        for row in 0..rows.get() {
            let text: String = screen.rows(0, cols.get()).nth(row as usize)?;
            if !text.contains(plugin_id) || !text.contains(" (") {
                continue;
            }
            for col in 0..cols.get() {
                let Some(cell) = screen.cell(row, col) else {
                    continue;
                };
                if matches!(cell.contents(), "●" | "◐" | "◉") {
                    return Some(match cell.fgcolor() {
                        vt100::Color::Idx(2) => Dot::Green,
                        vt100::Color::Idx(3) => Dot::Yellow,
                        vt100::Color::Idx(1) => Dot::Red,
                        _ => Dot::Other,
                    });
                }
            }
        }
        None
    }

    /// Polls until the dot of `plugin_id` has colour `want`; panics with the screen on timeout.
    pub fn wait_for_plugin_dot(&self, plugin_id: &str, want: Dot, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if self.plugin_dot(plugin_id) == Some(want) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "timed out after {timeout:?} waiting for {plugin_id}'s dot to be {want:?} (now {:?}); screen:\n{}",
                self.plugin_dot(plugin_id),
                self.screen_text()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Resizes the PTY and the emulator.
    pub fn resize(&self, rows: u16, cols: u16) {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("resize pty");
        self.parser
            .lock()
            .expect("parser")
            .screen_mut()
            .set_size(nz(rows), nz(cols));
    }

    /// Waits up to `timeout` for the child to exit and returns its exit code.
    pub fn wait_exit(&mut self, timeout: Duration) -> Option<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(status.exit_code());
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(h) = self.reader.take() {
            let _ = h.join();
        }
    }
}

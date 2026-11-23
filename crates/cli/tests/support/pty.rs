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

fn nz(v: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(v).expect("non-zero size")
}

/// A running binary inside a PTY with a `vt100` screen kept up to date by a reader thread.
pub struct PtySession {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    parser: Arc<Mutex<vt100::Parser>>,
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
        let writer = pair.master.take_writer().expect("take writer");
        let parser = Arc::new(Mutex::new(vt100::Parser::new(nz(rows), nz(cols), 0)));
        let parser_bg = Arc::clone(&parser);
        let handle = std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => parser_bg.lock().expect("parser").process(&buf[..n]),
                }
            }
        });
        Self {
            master: pair.master,
            writer,
            child,
            parser,
            reader: Some(handle),
        }
    }

    /// Writes raw bytes (keys, escape sequences) to the terminal input.
    pub fn send_keys(&mut self, keys: &str) {
        self.writer.write_all(keys.as_bytes()).expect("write keys");
        self.writer.flush().expect("flush keys");
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

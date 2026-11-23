//! Runtime log core: bounded in-memory ring, rotating file, secret masking and a live
//! subscription, shared by the daemon (`monoagens serve`) and any TUI hosting the gateway.
//!
//! Secret values are never stored: every message is masked through the gateway's
//! [`DiagnosticSink`] redaction (registered secrets plus `Bearer` / `sk-` token shapes)
//! before it reaches the ring, the file or a subscriber.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::clock::Clock;
use crate::logging::{DiagnosticSink, LogLevel as DiagLevel};

/// Capacity of the in-memory [`LogBuffer`].
pub const LOG_BUFFER_CAPACITY: usize = 5_000;
/// Rotation threshold of the log file.
pub const LOG_FILE_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Number of log files kept (the live file plus rotated copies).
pub const LOG_FILE_KEEP: usize = 3;
/// File name of the gateway log under `<data_dir>/logs/`.
pub const GATEWAY_LOG_FILENAME: &str = "gateway.log";

const SUBSCRIBER_CAPACITY: usize = 2_048;

/// Severity of a runtime log entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// Upper-case label used in the log file.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

/// One runtime log line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub ts_ms: u64,
    pub level: LogLevel,
    pub source: String,
    pub message: String,
}

/// Fixed-capacity ring of [`LogEntry`] values (oldest dropped first).
#[derive(Debug, Clone)]
pub struct LogBuffer {
    cap: usize,
    entries: VecDeque<LogEntry>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new(LOG_BUFFER_CAPACITY)
    }
}

impl LogBuffer {
    /// Creates a ring holding at most `cap` entries (minimum 1).
    #[must_use]
    pub fn new(cap: usize) -> Self {
        let cap = cap.max(1);
        Self {
            cap,
            entries: VecDeque::with_capacity(cap.min(1_024)),
        }
    }

    /// Appends `entry`, evicting the oldest one when full.
    pub fn push(&mut self, entry: LogEntry) {
        if self.entries.len() == self.cap {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Number of stored entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` when empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Snapshot of all entries, oldest first.
    #[must_use]
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.entries.iter().cloned().collect()
    }

    /// Snapshot of the newest `n` entries, oldest first.
    #[must_use]
    pub fn tail(&self, n: usize) -> Vec<LogEntry> {
        let skip = self.entries.len().saturating_sub(n);
        self.entries.iter().skip(skip).cloned().collect()
    }
}

/// Size-rotated append-only text log (`name`, `name.1`, `name.2`, ...).
#[derive(Debug)]
pub struct RotatingLogFile {
    path: PathBuf,
    max_bytes: u64,
    keep: usize,
    file: Option<File>,
    size: u64,
}

impl RotatingLogFile {
    /// Opens (creating parent directories) the live log file in append mode.
    pub fn open(path: PathBuf, max_bytes: u64, keep: usize) -> std::io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            path,
            max_bytes: max_bytes.max(1),
            keep: keep.max(1),
            file: Some(file),
            size,
        })
    }

    fn rotated(&self, idx: usize) -> PathBuf {
        let mut os = self.path.clone().into_os_string();
        os.push(format!(".{idx}"));
        PathBuf::from(os)
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        self.file = None;
        // Keep `keep` files in total: the live file plus `keep - 1` rotated copies.
        let rotated_keep = self.keep - 1;
        if rotated_keep == 0 {
            std::fs::remove_file(&self.path).ok();
        } else {
            std::fs::remove_file(self.rotated(rotated_keep)).ok();
            for idx in (1..rotated_keep).rev() {
                let from = self.rotated(idx);
                if from.exists() {
                    std::fs::rename(&from, self.rotated(idx + 1))?;
                }
            }
            std::fs::rename(&self.path, self.rotated(1))?;
        }
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?,
        );
        self.size = 0;
        Ok(())
    }

    /// Appends one line (a trailing newline is added), rotating first when it would overflow.
    pub fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        let needed = line.len() as u64 + 1;
        if self.size > 0 && self.size + needed > self.max_bytes {
            self.rotate()?;
        }
        if let Some(file) = self.file.as_mut() {
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
            self.size += needed;
        }
        Ok(())
    }

    /// Path of the live file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

struct RunLogInner {
    clock: Arc<dyn Clock>,
    sink: DiagnosticSink,
    buffer: Mutex<LogBuffer>,
    file: Mutex<Option<RotatingLogFile>>,
    tx: broadcast::Sender<LogEntry>,
}

/// Shared runtime log: ring buffer, rotating file and live subscription. Cheap to clone.
#[derive(Clone)]
pub struct RunLog {
    inner: Arc<RunLogInner>,
}

impl std::fmt::Debug for RunLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunLog").finish_non_exhaustive()
    }
}

impl RunLog {
    /// Creates a log masking through `sink`; when `file` is given entries are also appended
    /// to it (rotating at [`LOG_FILE_MAX_BYTES`], keeping [`LOG_FILE_KEEP`] files).
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>, sink: DiagnosticSink, file: Option<PathBuf>) -> Self {
        Self::with_limits(
            clock,
            sink,
            file,
            LOG_BUFFER_CAPACITY,
            LOG_FILE_MAX_BYTES,
            LOG_FILE_KEEP,
        )
    }

    /// Like [`RunLog::new`] with explicit ring capacity and rotation limits (used by tests).
    #[must_use]
    pub fn with_limits(
        clock: Arc<dyn Clock>,
        sink: DiagnosticSink,
        file: Option<PathBuf>,
        capacity: usize,
        max_bytes: u64,
        keep: usize,
    ) -> Self {
        let file = file.and_then(|p| RotatingLogFile::open(p, max_bytes, keep).ok());
        let (tx, _rx) = broadcast::channel(SUBSCRIBER_CAPACITY);
        Self {
            inner: Arc::new(RunLogInner {
                clock,
                sink,
                buffer: Mutex::new(LogBuffer::new(capacity)),
                file: Mutex::new(file),
                tx,
            }),
        }
    }

    /// Default log file location `<data_dir>/logs/gateway.log`.
    #[must_use]
    pub fn default_file(data_dir: &Path) -> PathBuf {
        data_dir.join("logs").join(GATEWAY_LOG_FILENAME)
    }

    /// Registers raw secret values to be masked in every future entry.
    pub fn register_secret_values<'a>(&self, values: impl IntoIterator<Item = &'a str>) {
        self.inner.sink.register_secret_values(values);
    }

    /// Masks secrets in `text` the same way log entries are masked.
    #[must_use]
    pub fn mask(&self, text: &str) -> String {
        self.inner.sink.redact(text)
    }

    /// Records one entry (message masked first).
    pub fn log(&self, level: LogLevel, source: &str, message: &str) {
        let entry = LogEntry {
            ts_ms: self.inner.clock.now().as_millis(),
            level,
            source: source.to_owned(),
            message: self.inner.sink.redact(message),
        };
        self.record(entry);
    }

    /// Convenience for [`LogLevel::Debug`].
    pub fn debug(&self, source: &str, message: &str) {
        self.log(LogLevel::Debug, source, message);
    }
    /// Convenience for [`LogLevel::Info`].
    pub fn info(&self, source: &str, message: &str) {
        self.log(LogLevel::Info, source, message);
    }
    /// Convenience for [`LogLevel::Warn`].
    pub fn warn(&self, source: &str, message: &str) {
        self.log(LogLevel::Warn, source, message);
    }
    /// Convenience for [`LogLevel::Error`].
    pub fn error(&self, source: &str, message: &str) {
        self.log(LogLevel::Error, source, message);
    }

    fn record(&self, entry: LogEntry) {
        if let Ok(mut file) = self.inner.file.lock()
            && let Some(f) = file.as_mut()
        {
            let line = format!(
                "{} {:<5} [{}] {}",
                entry.ts_ms,
                entry.level.label(),
                entry.source,
                entry.message.replace('\n', "\\n")
            );
            let _ = f.write_line(&line);
        }
        self.inner
            .buffer
            .lock()
            .expect("runlog buffer lock")
            .push(entry.clone());
        let _ = self.inner.tx.send(entry);
    }

    /// Snapshot of the whole ring, oldest first.
    #[must_use]
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.inner.buffer.lock().expect("runlog buffer lock").snapshot()
    }

    /// Snapshot of the newest `n` entries, oldest first.
    #[must_use]
    pub fn tail(&self, n: usize) -> Vec<LogEntry> {
        self.inner.buffer.lock().expect("runlog buffer lock").tail(n)
    }

    /// Subscribes to every entry recorded from now on.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.inner.tx.subscribe()
    }

    /// Mirrors every [`DiagnosticSink`] record (`gateway.*` components) into this log.
    ///
    /// `gateway.model` records become source `provider`; everything else becomes source
    /// `gateway` with the component name kept as a message prefix.
    pub fn attach_diagnostics(&self, sink: &DiagnosticSink) {
        let log = self.clone();
        sink.set_mirror(Arc::new(move |rec| {
            let level = match rec.level {
                DiagLevel::Info => LogLevel::Info,
                DiagLevel::Warn => LogLevel::Warn,
                DiagLevel::Error => LogLevel::Error,
            };
            let entry = LogEntry {
                ts_ms: rec.timestamp_ms.as_millis(),
                level,
                source: map_component_source(&rec.component).to_owned(),
                message: if rec.component == "gateway.model" {
                    rec.message.clone()
                } else {
                    format!("{}: {}", rec.component, rec.message)
                },
            };
            log.record(entry);
        }));
    }
}

fn map_component_source(component: &str) -> &'static str {
    if component == "gateway.model" {
        "provider"
    } else {
        "gateway"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::VirtualClock;
    use monoagens_protocol::Time;
    use monoagens_reason::ProviderSecrets;

    /// Scratch dir under the cargo target dir (never the tmpfs `/tmp`).
    fn scratch_dir() -> tempfile::TempDir {
        let base = std::env::current_exe()
            .unwrap()
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join("tmp");
        std::fs::create_dir_all(&base).unwrap();
        tempfile::Builder::new().prefix("runlog-").tempdir_in(base).unwrap()
    }

    fn entry(n: u64) -> LogEntry {
        LogEntry {
            ts_ms: n,
            level: LogLevel::Info,
            source: "t".to_owned(),
            message: format!("m{n}"),
        }
    }

    fn new_log(file: Option<PathBuf>, cap: usize, max: u64, keep: usize) -> (RunLog, DiagnosticSink) {
        let sink = DiagnosticSink::new(&ProviderSecrets::default());
        let clock = Arc::new(VirtualClock::new(Time::from_millis(42)));
        (RunLog::with_limits(clock, sink.clone(), file, cap, max, keep), sink)
    }

    #[test]
    fn ring_holds_exactly_five_thousand_and_drops_oldest() {
        let mut buf = LogBuffer::default();
        for n in 0..5_010 {
            buf.push(entry(n));
        }
        assert_eq!(buf.len(), LOG_BUFFER_CAPACITY);
        let snap = buf.snapshot();
        assert_eq!(snap.first().unwrap().ts_ms, 10);
        assert_eq!(snap.last().unwrap().ts_ms, 5_009);
        assert_eq!(buf.tail(2).len(), 2);
    }

    #[test]
    fn file_rotates_and_keeps_three_files() {
        let dir = scratch_dir();
        let path = dir.path().join("logs").join("gateway.log");
        let (log, _) = new_log(Some(path.clone()), 100, 200, 3);
        for n in 0..60 {
            log.info("src", &format!("line number {n} padding padding"));
        }
        assert!(path.exists());
        assert!(PathBuf::from(format!("{}.1", path.display())).exists());
        assert!(PathBuf::from(format!("{}.2", path.display())).exists());
        assert!(!PathBuf::from(format!("{}.3", path.display())).exists());
        assert!(std::fs::metadata(&path).unwrap().len() <= 200);
    }

    #[test]
    fn secrets_are_masked_in_ring_file_and_subscription() {
        let dir = scratch_dir();
        let path = dir.path().join("gateway.log");
        let (log, _) = new_log(Some(path.clone()), 10, 1 << 20, 3);
        log.register_secret_values(["hunter2-secret-value"]);
        let mut rx = log.subscribe();
        log.warn("plugin:x", "token=hunter2-secret-value end Bearer abcdef123");
        let snap = log.snapshot();
        assert!(!snap[0].message.contains("hunter2-secret-value"));
        assert!(!snap[0].message.contains("abcdef123"));
        let got = rx.try_recv().unwrap();
        assert_eq!(got, snap[0]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[REDACTED]"));
        assert!(!text.contains("hunter2-secret-value"));
        assert!(text.contains("[plugin:x]"));
    }

    #[test]
    fn diagnostics_are_mirrored() {
        let (log, sink) = new_log(None, 10, 1 << 20, 3);
        log.attach_diagnostics(&sink);
        sink.log(Time::from_millis(7), DiagLevel::Warn, "gateway.model", "slow");
        sink.log(Time::from_millis(8), DiagLevel::Error, "gateway.body", "gone");
        let snap = log.snapshot();
        assert_eq!(snap[0].source, "provider");
        assert_eq!(snap[0].message, "slow");
        assert_eq!(snap[1].source, "gateway");
        assert_eq!(snap[1].message, "gateway.body: gone");
        assert_eq!(snap[1].level, LogLevel::Error);
    }
}

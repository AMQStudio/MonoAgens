//! In-TUI pairing page.
//!
//! How the plugin emits the code (read from `plugins/whatsapp/src/main.rs`, 2026-11-23):
//! `body-whatsapp pair --db <path>` prints human text to **stderr** (a unicode QR block via
//! `render_qr_to_stderr_with_options`, or `=== WhatsApp Pair Code ... >>> CODE <<<` when paired by
//! phone number, plus `✅`/`❌` status lines) and exits `0` paired, `124` timed out, `130`
//! cancelled. There is no machine-readable form and no Waist-2 lifecycle event carries it, so the
//! page runs the same subprocess with piped output (no terminal takeover) and shows the lines as
//! the plugin emits them; the exit code is the final status.

use std::cell::Cell;
use std::process::Stdio;
use std::sync::Arc;

use monoagens_gateway::{
    BodyTransportConfig, GatewayConfig, ResolvedPaths, load_plugin_env_secrets,
};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{Notify, mpsc};

use crate::config::{config_dir_from_path, discover_builtin_binary, find_builtin_preset};
use crate::core::hitmap::{HitMap, HitTarget, ScrollId};
use crate::core::overlay::OverlayId;
use crate::core::theme::{ThemePalette, render_dimmed_backdrop};
use crate::core::viewport::{clamp_scroll_offset, max_scroll_offset};
use crate::ui_util::{boxed, centered, fill, trunc};
use crate::widgets::scrollbar::render_vertical_scrollbar;

/// Maximum lines kept for the page.
const MAX_LINES: usize = 400;

/// A resolved `<binary> pair ...` invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairCommand {
    pub binary: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Builds the pairing command of `plugin_id` (same resolution as `monoagens pair`).
pub fn pair_command(
    paths: &ResolvedPaths,
    cfg: &GatewayConfig,
    plugin_id: &str,
    phone: Option<&str>,
) -> Result<PairCommand, String> {
    let binary = if let Some(p) = cfg
        .effective_plugins()
        .into_iter()
        .find(|p| p.id == plugin_id)
    {
        match p.transport {
            BodyTransportConfig::Stdio { command, .. } => command,
            BodyTransportConfig::UnixSocket { path } => {
                return Err(format!(
                    "plugin '{plugin_id}' uses unix_socket ({}) and has no pair binary",
                    path.display()
                ));
            }
        }
    } else if let Some(preset) = find_builtin_preset(plugin_id) {
        discover_builtin_binary(preset)
            .ok_or_else(|| {
                format!(
                    "binary '{}' for '{plugin_id}' not found (build it first)",
                    preset.binary
                )
            })?
            .display()
            .to_string()
    } else {
        return Err(format!("plugin '{plugin_id}' not configured or built-in"));
    };
    let cfg_dir = config_dir_from_path(&paths.config_path);
    let state_dir = paths.data_dir.join("plugins").join(plugin_id);
    std::fs::create_dir_all(&state_dir)
        .map_err(|e| format!("failed to create plugin state dir: {e}"))?;
    let mut args = vec![
        "pair".to_owned(),
        "--db".to_owned(),
        state_dir.join("session.db").display().to_string(),
    ];
    if let Some(phone) = phone.map(str::trim).filter(|p| !p.is_empty()) {
        args.push("--phone".to_owned());
        args.push(phone.to_owned());
    }
    let mut env = vec![
        (
            "MONOAGENS_PLUGIN_CONFIG".to_owned(),
            cfg_dir
                .join("plugins")
                .join(format!("{plugin_id}.toml"))
                .display()
                .to_string(),
        ),
        (
            "MONOAGENS_PLUGIN_STATE_DIR".to_owned(),
            state_dir.display().to_string(),
        ),
        (
            "MONOAGENS_BODY_STATE_DIR".to_owned(),
            state_dir.display().to_string(),
        ),
    ];
    for (k, secret) in load_plugin_env_secrets(&paths.secrets_path).unwrap_or_default() {
        let v = secret.expose();
        if !v.is_empty() && std::env::var_os(&k).is_none() {
            env.push((k, v.to_owned()));
        }
    }
    Ok(PairCommand { binary, args, env })
}

/// Output of a running pairing process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingEvent {
    /// One line the plugin printed (ANSI stripped).
    Line(String),
    /// The plugin cleared its screen (a refreshed QR follows).
    Clear,
    /// The process ended with this exit code (`None` = killed by a signal).
    Exit(Option<i32>),
    /// The process could not be started.
    SpawnFailed(String),
}

/// Where the pairing stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingStatus {
    Waiting,
    /// The phone accepted the link; the plugin is saving the session.
    Scanned,
    Paired,
    TimedOut,
    Cancelled,
    Failed(String),
}

impl PairingStatus {
    /// Short label.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Waiting => "waiting for the phone".to_owned(),
            Self::Scanned => "scanned - finishing".to_owned(),
            Self::Paired => "paired".to_owned(),
            Self::TimedOut => "timed out".to_owned(),
            Self::Cancelled => "cancelled".to_owned(),
            Self::Failed(why) => format!("failed: {why}"),
        }
    }

    /// The process is over.
    #[must_use]
    pub fn is_done(&self) -> bool {
        matches!(
            self,
            Self::Paired | Self::TimedOut | Self::Cancelled | Self::Failed(_)
        )
    }
}

/// State of the pairing page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingState {
    pub plugin_id: String,
    pub lines: Vec<String>,
    pub status: PairingStatus,
    pub scroll_offset: Option<usize>,
    pub rendered_rows: Cell<usize>,
}

impl PairingState {
    /// New page in the waiting state.
    #[must_use]
    pub fn new(plugin_id: &str) -> Self {
        Self {
            plugin_id: plugin_id.to_owned(),
            lines: Vec::new(),
            status: PairingStatus::Waiting,
            scroll_offset: None,
            rendered_rows: Cell::new(0),
        }
    }

    /// Scrolls the pairing log by `delta` lines (negative = up, positive = down).
    pub fn scroll_by(&mut self, delta: isize) {
        let view = self.rendered_rows.get().max(1);
        let max_off = max_scroll_offset(self.lines.len(), view);
        let cur = self.scroll_offset.unwrap_or(max_off).min(max_off);
        let next = clamp_scroll_offset(cur.saturating_add_signed(delta), self.lines.len(), view);
        self.scroll_offset = if next >= max_off { None } else { Some(next) };
    }

    /// Jumps to a specific vertical scroll offset (e.g. from scrollbar click/drag).
    pub fn scroll_to(&mut self, offset: usize) {
        let view = self.rendered_rows.get().max(1);
        let max_off = max_scroll_offset(self.lines.len(), view);
        let next = clamp_scroll_offset(offset, self.lines.len(), view);
        self.scroll_offset = if next >= max_off { None } else { Some(next) };
    }

    /// Folds one event into the page.
    pub fn apply(&mut self, ev: PairingEvent) {
        match ev {
            PairingEvent::Line(l) => {
                if l.contains("Device linked") {
                    self.status = PairingStatus::Scanned;
                }
                self.lines.push(l);
                if self.lines.len() > MAX_LINES {
                    self.lines.drain(..self.lines.len() - MAX_LINES);
                }
            }
            PairingEvent::Clear => self.lines.clear(),
            PairingEvent::SpawnFailed(e) => self.status = PairingStatus::Failed(e),
            PairingEvent::Exit(code) => {
                self.status = match code {
                    Some(0) => PairingStatus::Paired,
                    Some(124) => PairingStatus::TimedOut,
                    Some(130) => PairingStatus::Cancelled,
                    Some(c) => PairingStatus::Failed(
                        self.lines
                            .iter()
                            .rev()
                            .find(|l| l.contains('❌') || l.to_lowercase().contains("error"))
                            .cloned()
                            .unwrap_or_else(|| format!("exit code {c}")),
                    ),
                    None => PairingStatus::Failed("killed".to_owned()),
                };
            }
        }
    }
}

/// Removes ANSI escape sequences; reports whether a clear-screen (`ESC[2J`) was present.
#[must_use]
pub fn strip_ansi(s: &str) -> (String, bool) {
    let mut out = String::with_capacity(s.len());
    let mut clear = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                let mut params = String::new();
                for n in chars.by_ref() {
                    if n.is_ascii_alphabetic() {
                        if n == 'J' && params == "2" {
                            clear = true;
                        }
                        break;
                    }
                    params.push(n);
                }
            }
        } else if c != '\r' {
            out.push(c);
        }
    }
    (out, clear)
}

/// Runs `cmd`, forwarding its output lines as events until it exits or `cancel` fires.
pub async fn run_pairing(
    cmd: PairCommand,
    tx: mpsc::UnboundedSender<PairingEvent>,
    cancel: Arc<Notify>,
) {
    let mut child = match tokio::process::Command::new(&cmd.binary)
        .args(&cmd.args)
        .envs(cmd.env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            let _ = tx.send(PairingEvent::SpawnFailed(format!(
                "failed to launch '{}': {e}",
                cmd.binary
            )));
            return;
        }
    };
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        readers.push(tokio::spawn(forward_lines(BufReader::new(out), tx.clone())));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(tokio::spawn(forward_lines(BufReader::new(err), tx.clone())));
    }
    let code = tokio::select! {
        status = child.wait() => status.ok().and_then(|s| s.code()),
        () = cancel.notified() => {
            let _ = child.kill().await;
            for r in readers.drain(..) {
                let _ = r.await;
            }
            let _ = tx.send(PairingEvent::Exit(Some(130)));
            return;
        }
    };
    for r in readers {
        let _ = r.await;
    }
    let _ = tx.send(PairingEvent::Exit(code));
}

async fn forward_lines<R: tokio::io::AsyncRead + Unpin>(
    reader: BufReader<R>,
    tx: mpsc::UnboundedSender<PairingEvent>,
) {
    let mut lines = reader.lines();
    while let Ok(Some(raw)) = lines.next_line().await {
        let (text, clear) = strip_ansi(&raw);
        if clear {
            let _ = tx.send(PairingEvent::Clear);
        }
        let _ = tx.send(PairingEvent::Line(crate::width::sanitize(&text, false)));
    }
}

/// Draws the pairing page and records backdrop/body/scrollbar regions in `hitmap`.
pub fn render_pairing_with_hitmap(
    area: Rect,
    buf: &mut Buffer,
    st: &PairingState,
    palette: &ThemePalette,
    hitmap: &mut HitMap,
) {
    render_dimmed_backdrop(buf, area, palette);
    hitmap.record(area, HitTarget::Backdrop(OverlayId::Settings));

    let bx = centered(area.width.min(100), area.height.min(34), area);
    let inner = boxed(bx, buf, &format!("Pair {}", st.plugin_id), palette);
    hitmap.record(bx, HitTarget::OverlayBody(OverlayId::Settings));
    if inner.height < 4 || inner.width < 16 {
        return;
    }
    let (color, glyph) = match st.status {
        PairingStatus::Paired => (palette.status_green, "●"),
        PairingStatus::Waiting | PairingStatus::Scanned => (palette.status_yellow, "◐"),
        _ => (palette.status_red, "●"),
    };
    buf.set_string(
        inner.x + 1,
        inner.y,
        trunc(
            &format!("{glyph} Status: {}", st.status.label()),
            inner.width.saturating_sub(2) as usize,
        ),
        Style::default().fg(color).bg(palette.modal_bg),
    );
    let body = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(3),
    );
    fill(buf, body, palette.modal_bg, palette.fg);
    let visible = body.height as usize;
    st.rendered_rows.set(visible.max(1));
    let max_off = max_scroll_offset(st.lines.len(), visible);
    let start = st.scroll_offset.unwrap_or(max_off).min(max_off);
    let has_sb = st.lines.len() > visible && body.width > 3;
    let content_w = if has_sb { body.width - 1 } else { body.width };
    for (i, line) in st.lines.iter().skip(start).take(visible).enumerate() {
        buf.set_string(
            body.x + 1,
            body.y + i as u16,
            crate::width::fit_prefix(line, content_w.saturating_sub(2) as usize),
            Style::default().fg(palette.fg).bg(palette.modal_bg),
        );
    }
    if has_sb && body.height > 0 {
        let track = Rect::new(body.x + body.width - 1, body.y, 1, body.height);
        render_vertical_scrollbar(
            buf,
            track,
            ScrollId::SettingsList,
            st.lines.len(),
            visible,
            start,
            hitmap,
            Style::default().fg(palette.dim).bg(palette.modal_bg),
            Style::default().fg(palette.accent).bg(palette.modal_bg),
        );
    }
    if st.lines.is_empty() && body.height > 0 {
        buf.set_string(
            body.x + 1,
            body.y,
            trunc(
                "Starting the pairing process...",
                content_w.saturating_sub(2) as usize,
            ),
            Style::default().fg(palette.dim).bg(palette.modal_bg),
        );
    }
    let hint = if st.status.is_done() {
        " Esc close · r retry"
    } else {
        " Esc cancel pairing"
    };
    buf.set_string(
        inner.x,
        inner.y + inner.height - 1,
        trunc(hint, usize::from(inner.width)),
        Style::default().fg(palette.dim).bg(palette.modal_bg),
    );
}

/// Draws the pairing page.
pub fn render_pairing(area: Rect, buf: &mut Buffer, st: &PairingState, palette: &ThemePalette) {
    let mut hitmap = HitMap::default();
    render_pairing_with_hitmap(area, buf, st, palette, &mut hitmap);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_map_to_status() {
        let mut st = PairingState::new("whatsapp");
        st.apply(PairingEvent::Line("code".into()));
        st.apply(PairingEvent::Line("✅ Device linked! Completing".into()));
        assert_eq!(st.status, PairingStatus::Scanned);
        st.apply(PairingEvent::Exit(Some(0)));
        assert_eq!(st.status, PairingStatus::Paired);
        let mut st = PairingState::new("w");
        st.apply(PairingEvent::Exit(Some(124)));
        assert_eq!(st.status, PairingStatus::TimedOut);
        let mut st = PairingState::new("w");
        st.apply(PairingEvent::Line("❌ WhatsApp pairing error".into()));
        st.apply(PairingEvent::Exit(Some(1)));
        assert!(matches!(&st.status, PairingStatus::Failed(m) if m.contains("pairing error")));
    }

    #[test]
    fn strips_ansi_and_detects_clear() {
        let (t, clear) = strip_ansi("\u{1b}[2J\u{1b}[H██ ▀▀\r");
        assert_eq!(t, "██ ▀▀");
        assert!(clear);
    }
}

//! Notifications when the terminal is unfocused and a reply or approval prompt arrives (bell,
//! OSC 9, OSC 777 and a title badge), plus the conversation preferences stored in `tui.toml`.
//!
//! Every OSC sequence ends with ST (`ESC \`), so a BEL byte in the output is always the bell.
//!
//! The notifier never touches the terminal itself: it collects the bytes to write in a buffer
//! that the event loop flushes after drawing, so tests assert on the exact escape sequences.

use serde::{Deserialize, Serialize};

/// `Appearance -> Notifications`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifyMode {
    Off,
    Bell,
    Desktop,
    #[default]
    Both,
}

impl NotifyMode {
    /// Parses `off|bell|desktop|both`.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "bell" => Some(Self::Bell),
            "desktop" => Some(Self::Desktop),
            "both" => Some(Self::Both),
            _ => None,
        }
    }

    /// Setting label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Bell => "bell",
            Self::Desktop => "desktop",
            Self::Both => "both",
        }
    }

    /// Next value when cycling in the settings.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Bell,
            Self::Bell => Self::Desktop,
            Self::Desktop => Self::Both,
            Self::Both => Self::Off,
        }
    }
}

const fn on() -> bool {
    true
}

/// Preferences of the conversation features (the `[conv]` table of `tui.toml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConvPrefs {
    /// `Appearance -> Notifications`.
    #[serde(default)]
    pub notifications: NotifyMode,
    /// `Appearance -> Live draft preview` (default on).
    #[serde(default = "on")]
    pub live_draft_preview: bool,
    /// Show inline previews of image attachments where the terminal supports it.
    #[serde(default = "on")]
    pub image_preview: bool,
}

impl Default for ConvPrefs {
    fn default() -> Self {
        Self {
            notifications: NotifyMode::default(),
            live_draft_preview: true,
            image_preview: true,
        }
    }
}

/// Window title while nothing is unread.
pub const BASE_TITLE: &str = "MonoAgens";

/// Focus tracking and the escape sequences of one notification.
#[derive(Debug, Clone)]
pub struct Notifier {
    pub mode: NotifyMode,
    focused: bool,
    unread: u32,
    out: Vec<u8>,
}

impl Default for Notifier {
    fn default() -> Self {
        Self {
            mode: NotifyMode::default(),
            focused: true,
            unread: 0,
            out: Vec::new(),
        }
    }
}

/// Text safe inside an OSC string: no control characters, no field separators, bounded.
fn osc_text(raw: &str, max_chars: usize) -> String {
    raw.chars()
        .map(|c| if c.is_control() || c == ';' { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

impl Notifier {
    /// Whether the terminal currently has focus (as far as focus reporting tells).
    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused
    }

    /// Unread notifications since focus was lost.
    #[must_use]
    pub const fn unread(&self) -> u32 {
        self.unread
    }

    /// Focus lost.
    pub fn focus_lost(&mut self) {
        self.focused = false;
    }

    /// Focus returned: the badge is removed.
    pub fn focus_gained(&mut self) {
        self.focused = true;
        if self.unread > 0 {
            self.unread = 0;
            if self.mode != NotifyMode::Off {
                self.out.extend_from_slice(Self::title_seq(BASE_TITLE).as_bytes());
            }
        }
    }

    fn title_seq(title: &str) -> String {
        format!("\x1b]2;{title}\x1b\\")
    }

    /// A reply arrived; `text` is its full text (the first line is used).
    pub fn reply(&mut self, text: &str) {
        let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
        self.notify("MonoAgens", first);
    }

    /// An approval prompt arrived.
    pub fn approval(&mut self, summary: &str) {
        self.notify("MonoAgens: approval needed", summary);
    }

    fn notify(&mut self, title: &str, body: &str) {
        if self.focused || self.mode == NotifyMode::Off {
            return;
        }
        self.unread += 1;
        let title = osc_text(title, 60);
        let body = osc_text(body, 120);
        if matches!(self.mode, NotifyMode::Bell | NotifyMode::Both) {
            self.out.push(0x07);
        }
        if matches!(self.mode, NotifyMode::Desktop | NotifyMode::Both) {
            self.out
                .extend_from_slice(format!("\x1b]9;{title}: {body}\x1b\\").as_bytes());
            self.out
                .extend_from_slice(format!("\x1b]777;notify;{title};{body}\x1b\\").as_bytes());
        }
        self.out
            .extend_from_slice(Self::title_seq(&format!("({}) {BASE_TITLE}", self.unread)).as_bytes());
    }

    /// Takes the pending bytes to write to the terminal.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn out(n: &mut Notifier) -> String {
        String::from_utf8(n.take_output()).unwrap()
    }

    #[test]
    fn both_writes_bell_osc9_osc777_and_badge_until_focus_returns() {
        let mut n = Notifier { mode: NotifyMode::Both, ..Notifier::default() };
        n.reply("anything while focused");
        assert!(n.take_output().is_empty(), "focused: nothing is written");
        n.focus_lost();
        n.reply("\nHalo; dunia\x1b[31m red\nsecond line");
        let s = out(&mut n);
        assert!(s.contains('\u{7}'), "bell");
        assert!(s.contains("\x1b]9;MonoAgens: Halo dunia [31m red\x1b\\"), "osc 9 with the first line: {s:?}");
        assert!(s.contains("\x1b]777;notify;MonoAgens;Halo dunia [31m red\x1b\\"), "osc 777: {s:?}");
        assert!(s.contains("\x1b]2;(1) MonoAgens\x1b\\"), "title badge");
        n.reply("again");
        assert!(out(&mut n).contains("(2) MonoAgens"));
        n.focus_gained();
        assert!(out(&mut n).contains("\x1b]2;MonoAgens\x1b\\"), "badge removed on focus");
        assert_eq!(n.unread(), 0);
    }

    #[test]
    fn off_writes_nothing_and_bell_or_desktop_write_only_their_part() {
        let mut n = Notifier { mode: NotifyMode::Off, ..Notifier::default() };
        n.focus_lost();
        n.reply("hi");
        n.approval("hold");
        n.focus_gained();
        assert!(n.take_output().is_empty(), "off: no sequence is written, not even a title");

        let mut b = Notifier { mode: NotifyMode::Bell, ..Notifier::default() };
        b.focus_lost();
        b.reply("hi");
        let s = out(&mut b);
        assert!(s.contains('\u{7}') && !s.contains("\x1b]9;") && !s.contains("\x1b]777;"));

        let mut d = Notifier { mode: NotifyMode::Desktop, ..Notifier::default() };
        d.focus_lost();
        d.approval("write file");
        let s = out(&mut d);
        assert!(!s.contains('\u{7}') && s.contains("\x1b]9;") && s.contains("\x1b]777;notify;"));
    }

    #[test]
    fn modes_parse_and_cycle() {
        assert_eq!(NotifyMode::parse("Both"), Some(NotifyMode::Both));
        assert_eq!(NotifyMode::parse("x"), None);
        assert_eq!(NotifyMode::Both.next(), NotifyMode::Off);
    }
}

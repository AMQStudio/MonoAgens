//! Text input widget state, paste sanitization/collapse, clipboard port, and clock port.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::core::overlay::{WidgetOutcome, is_key_press};
use crate::core::viewport::horizontal_cursor_window;

const MAX_UNDO_HISTORY: usize = 128;

static CLIPBOARD_LOCK: Mutex<()> = Mutex::new(());

/// Trait for widgets capable of receiving bracketed or system paste text.
pub trait PasteTarget {
    /// Sanitizes and inserts `raw` pasted text, returning `true` if handled/inserted.
    fn paste_text(&mut self, raw: &str) -> bool;
}

/// Sanitizes pasted text for either multiline or single-line inputs.
///
/// Pipeline:
/// 1. Decode/strip tmux extended-keys sequences (`\x1b[<cp>;<mod>u`, `\x1b[27;<mod>;<cp>~`)
///    and other ANSI CSI escape sequences so raw terminal escape debris is never inserted.
/// 2. Normalize `\r\n` and `\r` to `\n`. When `!multiline`, trim surrounding newlines and
///    collapse internal newline runs into a single `' '`.
/// 3. Normalize Unicode to NFC.
/// 4. Expand `\t` (`"    "` if `multiline`, `" "` otherwise) and strip C0 (`0x00..=0x1F`
///    except `\n` when `multiline`) plus DEL (`0x7F`).
#[must_use]
pub fn sanitize_paste(raw: &str, multiline: bool) -> String {
    let decoded = decode_and_strip_escapes(raw);

    let normalized_newlines = decoded.replace("\r\n", "\n").replace('\r', "\n");
    let line_adjusted = if multiline {
        normalized_newlines
    } else {
        collapse_single_line_newlines(&normalized_newlines)
    };

    let nfc: String = line_adjusted.nfc().collect();

    let mut out = String::with_capacity(nfc.len());
    for ch in nfc.chars() {
        match ch {
            '\t' => out.push_str(if multiline { "    " } else { " " }),
            '\n' if multiline => out.push('\n'),
            c if (c as u32) <= 0x1F || c == '\x7F' => {}
            c => out.push(c),
        }
    }
    out
}

fn collapse_single_line_newlines(input: &str) -> String {
    let trimmed = input.trim_matches('\n');
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_newline = false;
    for ch in trimmed.chars() {
        if ch == '\n' {
            if !prev_newline {
                out.push(' ');
                prev_newline = true;
            }
        } else {
            prev_newline = false;
            out.push(ch);
        }
    }
    out
}

fn decode_and_strip_escapes(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == 0x1B {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                // Scan CSI sequence: ESC [ <params 0x30..=0x3F>* <interm 0x20..=0x2F>* <final 0x40..=0x7E>
                let seq_start = i + 2;
                let mut j = seq_start;
                while j < bytes.len() && (0x20..=0x3F).contains(&bytes[j]) {
                    j += 1;
                }
                if j < bytes.len() && (0x40..=0x7E).contains(&bytes[j]) {
                    let params = &raw[seq_start..j];
                    let final_byte = bytes[j];
                    if let Some(ch) = decode_extended_key(params, final_byte) {
                        out.push(ch);
                    }
                    i = j + 1;
                    continue;
                }
            }
            // Drop bare ESC or malformed escape prefix.
            i += 1;
            continue;
        }

        let ch = raw[i..].chars().next().expect("valid utf-8 boundary");
        out.push(ch);
        i += ch.len_utf8();
    }

    out
}

fn decode_extended_key(params: &str, final_byte: u8) -> Option<char> {
    let (cp_val, mod_val) = match final_byte {
        b'u' => {
            // CSI u: <cp> or <cp>;<mod>[:<event>]
            let mut parts = params.split(';');
            let cp_part = parts.next()?.split(':').next()?;
            let cp = cp_part.parse::<u32>().ok()?;
            let modifier = match parts.next() {
                Some(m) if !m.is_empty() => m.split(':').next()?.parse::<u32>().ok()?,
                _ => 1,
            };
            if parts.next().is_some() {
                return None;
            }
            (cp, modifier)
        }
        b'~' => {
            // xterm modifyOtherKeys: 27;<mod>;<cp>
            let mut parts = params.split(';');
            let prefix = parts.next()?;
            if prefix != "27" {
                return None;
            }
            let modifier = parts.next()?.parse::<u32>().ok()?;
            let cp = parts.next()?.parse::<u32>().ok()?;
            if parts.next().is_some() {
                return None;
            }
            (cp, modifier)
        }
        _ => return None,
    };

    // Modifier bitmask in xterm/kitty encoding is `(mod_val - 1)`:
    // bit 1 (value 2) = Alt, bit 2 (value 4) = Ctrl, bit 3 (value 8) = Super.
    // Ignore control/meta shortcuts so Ctrl+C / Alt+X never leak characters.
    let mod_bits = mod_val.saturating_sub(1);
    if (mod_bits & (2 | 4 | 8)) != 0 {
        return None;
    }

    match cp_val {
        10 | 13 => Some('\n'),
        9 => Some('\t'),
        cp => char::from_u32(cp).filter(|c| !c.is_control() && *c != '\x7F'),
    }
}

/// Reusable single-line or multiline text input with grapheme-safe cursor and undo stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextInput {
    pub value: String,
    pub cursor_byte: usize,
    pub multiline: bool,
    pub masked: bool,
    pub placeholder: String,
    undo_stack: Vec<(String, usize)>,
}

impl TextInput {
    /// Creates an empty [`TextInput`].
    #[must_use]
    pub fn new(multiline: bool) -> Self {
        Self {
            value: String::new(),
            cursor_byte: 0,
            multiline,
            masked: false,
            placeholder: String::new(),
            undo_stack: Vec::new(),
        }
    }

    /// Builder method setting initial text and moving the cursor to the end.
    #[must_use]
    pub fn with_value(mut self, v: impl Into<String>) -> Self {
        self.value = v.into();
        self.cursor_byte = self.value.len();
        self
    }

    /// Builder method setting placeholder text.
    #[must_use]
    pub fn with_placeholder(mut self, p: impl Into<String>) -> Self {
        self.placeholder = p.into();
        self
    }

    /// Builder method enabling or disabling secret masking (`•`).
    #[must_use]
    pub fn with_masked(mut self, m: bool) -> Self {
        self.masked = m;
        self
    }

    /// Replaces the text value and places the cursor at the end.
    pub fn set_value(&mut self, v: impl Into<String>) {
        let next = v.into();
        if next != self.value {
            self.push_undo();
            self.value = next;
        }
        self.cursor_byte = self.value.len();
    }

    /// Clears text, cursor, and undo history.
    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor_byte = 0;
        self.undo_stack.clear();
    }

    /// Returns `true` when the underlying value is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Returns the underlying text slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }

    /// Returns the current cursor byte offset within [`Self::value`].
    #[must_use]
    pub fn cursor_byte(&self) -> usize {
        self.cursor_byte
    }

    /// Inserts a single character at the cursor position.
    pub fn insert_char(&mut self, ch: char) {
        if !self.multiline && (ch == '\n' || ch == '\r') {
            return;
        }
        self.push_undo();
        let pos = self.snap_cursor();
        self.value.insert(pos, ch);
        self.cursor_byte = pos + ch.len_utf8();
    }

    /// Inserts `s` at the cursor position.
    pub fn insert_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.push_undo();
        let pos = self.snap_cursor();
        self.value.insert_str(pos, s);
        self.cursor_byte = pos + s.len();
    }

    /// Deletes the grapheme cluster immediately before the cursor.
    pub fn backspace(&mut self) {
        let pos = self.snap_cursor();
        if pos == 0 {
            self.cursor_byte = 0;
            return;
        }
        let prev = self.prev_grapheme_boundary(pos);
        self.push_undo();
        self.value.drain(prev..pos);
        self.cursor_byte = prev;
    }

    /// Deletes the grapheme cluster at the cursor.
    pub fn delete(&mut self) {
        let pos = self.snap_cursor();
        if pos >= self.value.len() {
            self.cursor_byte = self.value.len();
            return;
        }
        let next = self.next_grapheme_boundary(pos);
        self.push_undo();
        self.value.drain(pos..next);
        self.cursor_byte = pos;
    }

    /// Deletes the word immediately before the cursor (readline `Ctrl+W` semantics).
    pub fn delete_word_back(&mut self) {
        let pos = self.snap_cursor();
        if pos == 0 {
            self.cursor_byte = 0;
            return;
        }
        let graphemes: Vec<(usize, &str)> = self.value[..pos].grapheme_indices(true).collect();
        let mut idx = graphemes.len();
        while idx > 0 && graphemes[idx - 1].1.chars().all(char::is_whitespace) {
            idx -= 1;
        }
        while idx > 0 && !graphemes[idx - 1].1.chars().all(char::is_whitespace) {
            idx -= 1;
        }
        let new_pos = if idx < graphemes.len() {
            graphemes[idx].0
        } else {
            0
        };
        if new_pos < pos {
            self.push_undo();
            self.value.drain(new_pos..pos);
            self.cursor_byte = new_pos;
        }
    }

    /// Deletes everything from the start of the input up to the cursor (`Ctrl+U`).
    pub fn kill_to_start(&mut self) {
        let pos = self.snap_cursor();
        if pos == 0 {
            self.cursor_byte = 0;
            return;
        }
        self.push_undo();
        self.value.drain(..pos);
        self.cursor_byte = 0;
    }

    /// Deletes everything from the cursor to the end of the input (`Ctrl+K`).
    pub fn kill_to_end(&mut self) {
        let pos = self.snap_cursor();
        if pos >= self.value.len() {
            self.cursor_byte = self.value.len();
            return;
        }
        self.push_undo();
        self.value.truncate(pos);
        self.cursor_byte = pos;
    }

    /// Moves the cursor one grapheme cluster to the left.
    pub fn move_left(&mut self) {
        let pos = self.snap_cursor();
        self.cursor_byte = self.prev_grapheme_boundary(pos);
    }

    /// Moves the cursor one grapheme cluster to the right.
    pub fn move_right(&mut self) {
        let pos = self.snap_cursor();
        self.cursor_byte = self.next_grapheme_boundary(pos);
    }

    /// Moves the cursor to byte `0`.
    pub fn move_home(&mut self) {
        self.cursor_byte = 0;
    }

    /// Moves the cursor to the end of [`Self::value`].
    pub fn move_end(&mut self) {
        self.cursor_byte = self.value.len();
    }

    /// Restores the previous value and cursor position from the undo stack.
    pub fn undo(&mut self) -> bool {
        if let Some((prev_value, prev_cursor)) = self.undo_stack.pop() {
            self.value = prev_value;
            self.cursor_byte = prev_cursor.min(self.value.len());
            self.cursor_byte = self.snap_cursor();
            true
        } else {
            false
        }
    }

    /// Returns the display string (`•` per grapheme cluster when [`Self::masked`] is true).
    #[must_use]
    pub fn display_text(&self) -> String {
        if self.masked {
            "•".repeat(self.value.graphemes(true).count())
        } else {
            self.value.clone()
        }
    }

    /// Returns the horizontally scrolled visible slice fitting within `width` columns and the
    /// cursor's display column relative to the start of that slice.
    #[must_use]
    pub fn visible_slice(&self, width: usize) -> (String, usize) {
        let pos = self.snap_cursor();
        if self.masked {
            let masked = self.display_text();
            let grapheme_idx = self.value[..pos].graphemes(true).count();
            let bullet_len = "•".len();
            let masked_cursor = grapheme_idx * bullet_len;
            let (start, end) = horizontal_cursor_window(&masked, masked_cursor, width);
            let cursor_col = UnicodeWidthStr::width(&masked[start..masked_cursor.min(end)]);
            (masked[start..end].to_owned(), cursor_col)
        } else {
            let (start, end) = horizontal_cursor_window(&self.value, pos, width);
            let cursor_col = UnicodeWidthStr::width(&self.value[start..pos.min(end)]);
            (self.value[start..end].to_owned(), cursor_col)
        }
    }

    /// Processes a single terminal key event for editing and cursor navigation.
    pub fn handle_key(&mut self, key: KeyEvent) -> WidgetOutcome {
        if !is_key_press(key) {
            return WidgetOutcome::Ignored;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);

        match key.code {
            KeyCode::Esc | KeyCode::Tab | KeyCode::BackTab => WidgetOutcome::Ignored,
            KeyCode::Enter => {
                if self.multiline && !ctrl && !alt {
                    self.insert_char('\n');
                    WidgetOutcome::Consumed
                } else {
                    WidgetOutcome::Ignored
                }
            }
            KeyCode::Up | KeyCode::Down => WidgetOutcome::Ignored,
            KeyCode::Backspace => {
                if ctrl || alt {
                    self.delete_word_back();
                } else {
                    self.backspace();
                }
                WidgetOutcome::Consumed
            }
            KeyCode::Delete => {
                self.delete();
                WidgetOutcome::Consumed
            }
            KeyCode::Left => {
                self.move_left();
                WidgetOutcome::Consumed
            }
            KeyCode::Right => {
                self.move_right();
                WidgetOutcome::Consumed
            }
            KeyCode::Home => {
                self.move_home();
                WidgetOutcome::Consumed
            }
            KeyCode::End => {
                self.move_end();
                WidgetOutcome::Consumed
            }
            KeyCode::Char(c) if ctrl && !alt => match c {
                'a' | 'A' => {
                    self.move_home();
                    WidgetOutcome::Consumed
                }
                'e' | 'E' => {
                    self.move_end();
                    WidgetOutcome::Consumed
                }
                'u' | 'U' => {
                    self.kill_to_start();
                    WidgetOutcome::Consumed
                }
                'k' | 'K' => {
                    self.kill_to_end();
                    WidgetOutcome::Consumed
                }
                'w' | 'W' => {
                    self.delete_word_back();
                    WidgetOutcome::Consumed
                }
                'z' | 'Z' => {
                    self.undo();
                    WidgetOutcome::Consumed
                }
                _ => WidgetOutcome::Ignored,
            },
            KeyCode::Char(c) if !ctrl && !alt => {
                self.insert_char(c);
                WidgetOutcome::Consumed
            }
            _ => WidgetOutcome::Ignored,
        }
    }

    fn push_undo(&mut self) {
        if self.undo_stack.len() >= MAX_UNDO_HISTORY {
            self.undo_stack.remove(0);
        }
        self.undo_stack
            .push((self.value.clone(), self.snap_cursor()));
    }

    fn snap_cursor(&self) -> usize {
        let clamped = self.cursor_byte.min(self.value.len());
        if clamped == self.value.len() {
            return clamped;
        }
        for (start, g) in self.value.grapheme_indices(true) {
            let end = start + g.len();
            if clamped >= start && clamped < end {
                return start;
            }
        }
        self.value.len()
    }

    fn prev_grapheme_boundary(&self, byte: usize) -> usize {
        let pos = byte.min(self.value.len());
        self.value[..pos]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(idx, _)| idx)
    }

    fn next_grapheme_boundary(&self, byte: usize) -> usize {
        let pos = byte.min(self.value.len());
        if pos >= self.value.len() {
            return self.value.len();
        }
        self.value[pos..]
            .graphemes(true)
            .next()
            .map_or(self.value.len(), |g| pos + g.len())
    }
}

impl PasteTarget for TextInput {
    fn paste_text(&mut self, raw: &str) -> bool {
        let clean = sanitize_paste(raw, self.multiline);
        if clean.is_empty() {
            return false;
        }
        self.insert_str(&clean);
        true
    }
}

/// Stores large pasted payloads behind compact `[Paste #N, +L lines]` tokens in the composer.
#[derive(Debug, Clone, Default)]
pub struct PasteCollapseStore {
    next_id: usize,
    entries: Vec<(String, String)>,
}

impl PasteCollapseStore {
    /// Collapses `sanitized` into a `[Paste #N, +L lines]` token when `lines > 10` or
    /// `chars > 1000`.
    pub fn collapse_if_large(&mut self, sanitized: &str) -> String {
        let lines = sanitized.split('\n').count();
        let chars = sanitized.chars().count();
        if lines > 10 || chars > 1000 {
            self.next_id += 1;
            let token = format!("[Paste #{}, +{} lines]", self.next_id, lines);
            self.entries.push((token.clone(), sanitized.to_owned()));
            token
        } else {
            sanitized.to_owned()
        }
    }

    /// Expands all stored paste tokens inside `text` back to their full payloads.
    #[must_use]
    pub fn expand(&self, text: &str) -> String {
        if self.entries.is_empty() || !text.contains("[Paste #") {
            return text.to_owned();
        }
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while !rest.is_empty() {
            if let Some((token, body)) = self
                .entries
                .iter()
                .find(|(tok, _)| rest.starts_with(tok.as_str()))
            {
                out.push_str(body);
                rest = &rest[token.len()..];
            } else {
                let ch = rest.chars().next().expect("non-empty rest");
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
        out
    }

    /// Clears all collapsed paste entries and resets the paste counter.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.next_id = 0;
    }
}

/// Abstraction over system clipboard reads and writes for testability.
pub trait ClipboardPort: Send + Sync {
    fn read_text(&self) -> Result<Option<String>, String>;
    fn write_text(&self, text: &str) -> Result<(), String>;
}

/// Production [`ClipboardPort`] backed by [`crate::clipboard`] and guarded by a mutex.
#[derive(Debug, Default)]
pub struct SystemClipboard;

impl ClipboardPort for SystemClipboard {
    fn read_text(&self) -> Result<Option<String>, String> {
        let _guard = CLIPBOARD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match crate::clipboard::read_clipboard_image_or_text() {
            Some(crate::clipboard::ClipboardContent::Text(text)) => Ok(Some(text)),
            Some(crate::clipboard::ClipboardContent::ImagePng(_)) | None => Ok(None),
        }
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        let _guard = CLIPBOARD_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::clipboard::copy_to_clipboard(text)
    }
}

/// Abstraction over wall-clock milliseconds for deterministic testing.
pub trait ClockPort: Send + Sync {
    fn now_ms(&self) -> u64;
}

/// Production [`ClockPort`] backed by [`SystemTime::UNIX_EPOCH`].
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl ClockPort for SystemClock {
    fn now_ms(&self) -> u64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyEventKind;

    use super::*;

    #[test]
    fn sanitize_paste_handles_escapes_newlines_nfc_and_tabs() {
        // Single-line trims surrounding newlines, collapses internal newline runs to a space,
        // strips tmux/CSI control debris, normalizes NFD -> NFC, and converts tabs to single spaces.
        let raw = "\r\n\x1b[200~e\u{0301}\r\n\r\nworld\t!\x1b[99;5u\x07\x7F\n";
        assert_eq!(sanitize_paste(raw, false), "\u{00E9} world !");

        // Extended keys: Shift+Enter (\x1b[27;2;13~) and plain 'A' (\x1b[65;1u) in multiline.
        let ext = "line1\x1b[27;2;13~line2\t\x1b[65;1u\x1b[3;5u";
        assert_eq!(sanitize_paste(ext, true), "line1\nline2    A");
    }

    #[test]
    fn text_input_editing_undo_mask_and_keys() {
        let mut input = TextInput::new(false).with_placeholder("type here");
        assert!(input.is_empty());

        input.insert_str("hello world");
        assert_eq!(input.as_str(), "hello world");
        assert_eq!(input.cursor_byte(), 11);

        // Ctrl+W deletes trailing word.
        let ctrl_w = KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL);
        assert_eq!(input.handle_key(ctrl_w), WidgetOutcome::Consumed);
        assert_eq!(input.as_str(), "hello ");

        // Ctrl+Z undoes delete_word_back.
        let ctrl_z = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL);
        assert_eq!(input.handle_key(ctrl_z), WidgetOutcome::Consumed);
        assert_eq!(input.as_str(), "hello world");

        // Ctrl+A moves home, Ctrl+K kills to end.
        assert_eq!(
            input.handle_key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            WidgetOutcome::Consumed
        );
        assert_eq!(input.cursor_byte(), 0);
        assert_eq!(
            input.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL)),
            WidgetOutcome::Consumed
        );
        assert_eq!(input.as_str(), "");
        assert!(input.undo());
        assert_eq!(input.as_str(), "hello world");

        // Esc, Tab, Enter (when !multiline), Up/Down, and KeyEventKind::Release are ignored.
        assert_eq!(
            input.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            WidgetOutcome::Ignored
        );
        assert_eq!(
            input.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            WidgetOutcome::Ignored
        );
        assert_eq!(
            input.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            WidgetOutcome::Ignored
        );
        assert_eq!(
            input.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            WidgetOutcome::Ignored
        );
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('x'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );
        assert_eq!(input.handle_key(release), WidgetOutcome::Ignored);

        // Masking renders bullets per grapheme and computes visible slice safely.
        let masked = TextInput::new(false).with_value("s3cr3t").with_masked(true);
        assert_eq!(masked.display_text(), "••••••");
        let (vis, col) = masked.visible_slice(4);
        assert_eq!(vis, "••••");
        assert_eq!(col, 4);

        // PasteTarget sanitizes and inserts.
        let mut target = TextInput::new(false);
        assert!(target.paste_text("\nfoo\nbar\n"));
        assert_eq!(target.as_str(), "foo bar");
        assert!(!target.paste_text("\x1b[99;5u"));
    }

    #[test]
    fn paste_collapse_store_collapses_and_expands() {
        let mut store = PasteCollapseStore::default();
        assert_eq!(store.collapse_if_large("short\ntext"), "short\ntext");

        let large_lines = (1..=12)
            .map(|n| format!("row {n}"))
            .collect::<Vec<_>>()
            .join("\n");
        let token = store.collapse_if_large(&large_lines);
        assert_eq!(token, "[Paste #1, +12 lines]");

        let composed = format!("prefix {token} suffix");
        assert_eq!(
            store.expand(&composed),
            format!("prefix {large_lines} suffix")
        );

        store.clear();
        assert_eq!(store.expand(&composed), composed);
    }
}

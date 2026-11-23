//! Shared deterministic test doubles, `TestBackend` harness, and event constructors
//! for the consolidated `monoagens-tui` integration contract tests.

#![allow(dead_code)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use monoagens_tui::core::input::{ClipboardPort, ClockPort};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;

/// Deterministic monotonic millisecond clock double for contract tests.
#[derive(Debug, Default)]
pub struct FakeClock {
    now_ms: AtomicU64,
}

impl FakeClock {
    /// Creates a clock initialized at `initial_ms`.
    #[must_use]
    pub const fn new(initial_ms: u64) -> Self {
        Self {
            now_ms: AtomicU64::new(initial_ms),
        }
    }

    /// Advances the clock forward by `ms` milliseconds.
    pub fn advance(&self, ms: u64) {
        self.now_ms.fetch_add(ms, Ordering::SeqCst);
    }

    /// Reads the current timestamp in milliseconds.
    #[must_use]
    pub fn now(&self) -> u64 {
        self.now_ms.load(Ordering::SeqCst)
    }
}

impl ClockPort for FakeClock {
    fn now_ms(&self) -> u64 {
        self.now()
    }
}

/// Deterministic in-memory clipboard double for contract tests.
#[derive(Debug, Default)]
pub struct FakeClipboard {
    text: Mutex<Option<String>>,
}

impl FakeClipboard {
    /// Creates a clipboard double preloaded with `initial`.
    #[must_use]
    pub fn new(initial: Option<&str>) -> Self {
        Self {
            text: Mutex::new(initial.map(str::to_owned)),
        }
    }

    /// Replaces the current clipboard text (`None` simulates an empty/non-text clipboard).
    pub fn set_text(&self, s: Option<&str>) {
        let mut guard = self.text.lock().expect("fake clipboard mutex poisoned");
        *guard = s.map(str::to_owned);
    }
}

impl ClipboardPort for FakeClipboard {
    fn read_text(&self) -> Result<Option<String>, String> {
        Ok(self
            .text
            .lock()
            .expect("fake clipboard mutex poisoned")
            .clone())
    }

    fn write_text(&self, text: &str) -> Result<(), String> {
        self.set_text(Some(text));
        Ok(())
    }
}

/// Headless Ratatui `TestBackend` terminal wrapper with buffer inspection and invariant assertions.
pub struct TuiTestHarness {
    terminal: Terminal<TestBackend>,
}

impl TuiTestHarness {
    /// Creates a `TestBackend` terminal of `width x height` cells.
    #[must_use]
    pub fn new(width: u16, height: u16) -> Self {
        let backend = TestBackend::new(width, height);
        let terminal = Terminal::new(backend).expect("initialize ratatui TestBackend");
        Self { terminal }
    }

    /// Returns the full terminal viewport `Rect`.
    #[must_use]
    pub fn area(&self) -> Rect {
        *self.terminal.backend().buffer().area()
    }

    /// Renders one frame using `f(frame_area, buf)`.
    pub fn draw_with<F>(&mut self, f: F)
    where
        F: FnOnce(Rect, &mut Buffer),
    {
        self.terminal
            .draw(|frame| {
                let area = frame.area();
                f(area, frame.buffer_mut());
            })
            .expect("render frame to TestBackend");
    }

    /// Returns a reference to the current rendered `Buffer`.
    #[must_use]
    pub fn buffer(&self) -> &Buffer {
        self.terminal.backend().buffer()
    }

    /// Extracts the full terminal buffer text, one trimmed line per row.
    #[must_use]
    pub fn buffer_text(&self) -> String {
        self.rect_text(self.area())
    }

    /// Extracts the text on buffer row `y` (trimmed of trailing spaces).
    #[must_use]
    pub fn row_text(&self, y: u16) -> String {
        let area = self.area();
        if y < area.y || y >= area.bottom() {
            return String::new();
        }
        self.rect_text(Rect::new(area.x, y, area.width, 1))
    }

    /// Extracts all visible text inside `rect`, clipped to the terminal bounds.
    #[must_use]
    pub fn rect_text(&self, rect: Rect) -> String {
        extract_rect_text(self.buffer(), rect)
    }

    /// Asserts that `needle` appears inside `rect`.
    #[track_caller]
    pub fn assert_text_in_rect(&self, rect: Rect, needle: &str) {
        let actual = self.rect_text(rect);
        assert!(
            actual.contains(needle),
            "expected {needle:?} inside rect {rect:?}, got:\n{actual}"
        );
    }

    /// Asserts that `forbidden` does not appear anywhere inside `rect`.
    #[track_caller]
    pub fn assert_text_absent_in_rect(&self, rect: Rect, forbidden: &str) {
        let actual = self.rect_text(rect);
        assert!(
            !actual.contains(forbidden),
            "unexpected {forbidden:?} found inside rect {rect:?}:\n{actual}"
        );
    }

    /// Asserts that `forbidden` does not bleed anywhere outside `modal`.
    #[track_caller]
    pub fn assert_no_bleed_outside(&self, modal: Rect, forbidden: &str) {
        let area = self.area();
        let buf = self.buffer();
        for y in area.y..area.bottom() {
            if y < modal.y || y >= modal.bottom() {
                let row = extract_rect_text(buf, Rect::new(area.x, y, area.width, 1));
                assert!(
                    !row.contains(forbidden),
                    "forbidden text {forbidden:?} bled outside modal {modal:?} on row {y}: {row:?}"
                );
            } else {
                if modal.x > area.x {
                    let left = extract_rect_text(buf, Rect::new(area.x, y, modal.x - area.x, 1));
                    assert!(
                        !left.contains(forbidden),
                        "forbidden text {forbidden:?} bled left of modal {modal:?} on row {y}: {left:?}"
                    );
                }
                if area.right() > modal.right() {
                    let right = extract_rect_text(
                        buf,
                        Rect::new(modal.right(), y, area.right() - modal.right(), 1),
                    );
                    assert!(
                        !right.contains(forbidden),
                        "forbidden text {forbidden:?} bled right of modal {modal:?} on row {y}: {right:?}"
                    );
                }
            }
        }
    }

    /// Asserts that row `row` within `modal_or_screen` contains every substring in `expected_substrings`.
    #[track_caller]
    pub fn assert_footer_intact(
        &self,
        modal_or_screen: Rect,
        row: u16,
        expected_substrings: &[&str],
    ) {
        assert!(
            row >= modal_or_screen.y && row < modal_or_screen.bottom(),
            "footer row {row} is outside bounds {modal_or_screen:?}"
        );
        let footer_rect = Rect::new(modal_or_screen.x, row, modal_or_screen.width, 1);
        let actual = self.rect_text(footer_rect);
        for sub in expected_substrings {
            assert!(
                actual.contains(sub),
                "expected footer substring {sub:?} on row {row} ({footer_rect:?}), got: {actual:?}"
            );
        }
    }
}

fn extract_rect_text(buf: &Buffer, rect: Rect) -> String {
    let clipped = rect.intersection(*buf.area());
    if clipped.is_empty() {
        return String::new();
    }
    let mut lines = Vec::with_capacity(usize::from(clipped.height));
    for y in clipped.y..clipped.bottom() {
        let mut line = String::new();
        let mut skip = 0usize;
        for x in clipped.x..clipped.right() {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            let sym = buf.cell((x, y)).expect("buffer cell in clipped rect").symbol();
            let width = sym.width();
            if width > 1 {
                skip = width - 1;
            }
            line.push_str(sym);
        }
        lines.push(line.trim_end().to_owned());
    }
    lines.join("\n")
}

/// Constructs a `KeyEvent` with `KeyEventKind::Press`.
#[must_use]
pub fn key_press(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

/// Constructs a `KeyEvent` with `KeyEventKind::Repeat`.
#[must_use]
pub fn key_repeat(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    let mut ev = KeyEvent::new(code, mods);
    ev.kind = KeyEventKind::Repeat;
    ev
}

/// Constructs a `KeyEvent` with `KeyEventKind::Release`.
#[must_use]
pub fn key_release(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    let mut ev = KeyEvent::new(code, mods);
    ev.kind = KeyEventKind::Release;
    ev
}

/// Constructs a bracketed paste `Event::Paste`.
#[must_use]
pub fn paste_event(s: &str) -> Event {
    Event::Paste(s.to_owned())
}

/// Constructs a left-button mouse press event at `(col, row)`.
#[must_use]
pub const fn mouse_click(col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Constructs a mouse wheel scroll-up event at `(col, row)`.
#[must_use]
pub const fn mouse_scroll_up(col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Constructs a mouse wheel scroll-down event at `(col, row)`.
#[must_use]
pub const fn mouse_scroll_down(col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::ScrollDown,
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Constructs a horizontal trackpad scroll-left event at `(col, row)`.
#[must_use]
pub const fn mouse_scroll_left(col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::ScrollLeft,
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

/// Constructs a left-button mouse drag event at `(col, row)`.
#[must_use]
pub const fn mouse_drag(col: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind: MouseEventKind::Drag(MouseButton::Left),
        column: col,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

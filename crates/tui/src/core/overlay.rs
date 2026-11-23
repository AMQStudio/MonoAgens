//! Overlay stack, focus scopes, widget event outcomes, and key-press filtering.

use crossterm::event::{KeyEvent, KeyEventKind};

/// Canonical identifier for modal and floating overlays in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayId {
    Help,
    Palette,
    Log,
    Catalog,
    Governance,
    Settings,
    Wizard,
    ConvModal,
}

/// Active keyboard focus target derived from the overlay stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusScope {
    ChatComposer,
    Overlay(OverlayId),
    OverlaySubInput(OverlayId),
}

/// Single entry on the [`OverlayStack`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayEntry {
    pub id: OverlayId,
    pub blocks_chat: bool,
    pub dim_backdrop: bool,
    pub sub_input_active: bool,
}

/// Ordered stack of active overlays (index `0` is bottom, last element is top).
#[derive(Debug, Clone, Default)]
pub struct OverlayStack {
    entries: Vec<OverlayEntry>,
}

impl OverlayStack {
    /// Pushes an overlay entry onto the top of the stack.
    pub fn push(&mut self, entry: OverlayEntry) {
        self.entries.push(entry);
    }

    /// Removes any existing entry with the same [`OverlayId`] and pushes `entry` on top.
    pub fn raise_or_push(&mut self, entry: OverlayEntry) {
        self.remove(entry.id);
        self.entries.push(entry);
    }

    /// Pops and returns the topmost overlay entry, if any.
    pub fn pop(&mut self) -> Option<OverlayEntry> {
        self.entries.pop()
    }

    /// Removes all entries matching `id`, returning `true` if at least one was removed.
    pub fn remove(&mut self, id: OverlayId) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != before
    }

    /// Removes all entries from the stack.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Returns a reference to the topmost overlay entry.
    #[must_use]
    pub fn top(&self) -> Option<&OverlayEntry> {
        self.entries.last()
    }

    /// Returns a mutable reference to the topmost overlay entry.
    pub fn top_mut(&mut self) -> Option<&mut OverlayEntry> {
        self.entries.last_mut()
    }

    /// Updates `sub_input_active` for any entry matching `id`.
    pub fn set_sub_input_active(&mut self, id: OverlayId, active: bool) {
        for entry in &mut self.entries {
            if entry.id == id {
                entry.sub_input_active = active;
            }
        }
    }

    /// Returns `true` if an overlay with `id` is currently on the stack.
    #[must_use]
    pub fn contains(&self, id: OverlayId) -> bool {
        self.entries.iter().any(|entry| entry.id == id)
    }

    /// Returns `true` when no overlays are open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of entries on the stack.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Iterates over overlays from bottom (oldest/lowest) to top (active).
    pub fn iter_bottom_to_top(&self) -> impl Iterator<Item = &OverlayEntry> {
        self.entries.iter()
    }

    /// Returns `true` if any active overlay blocks interaction with the main chat pane.
    #[must_use]
    pub fn blocks_chat(&self) -> bool {
        self.entries.iter().any(|entry| entry.blocks_chat)
    }

    /// Computes the current [`FocusScope`] from the topmost overlay entry.
    #[must_use]
    pub fn focus_scope(&self) -> FocusScope {
        match self.top() {
            None => FocusScope::ChatComposer,
            Some(entry) if entry.sub_input_active => FocusScope::OverlaySubInput(entry.id),
            Some(entry) => FocusScope::Overlay(entry.id),
        }
    }
}

/// Outcome returned by a widget or overlay input handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WidgetOutcome<A = ()> {
    Consumed,
    Ignored,
    Close,
    Action(A),
}

/// Returns `true` for key press and repeat events, filtering out [`KeyEventKind::Release`].
#[must_use]
pub fn is_key_press(key: KeyEvent) -> bool {
    key.kind != KeyEventKind::Release
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use super::*;

    fn entry(id: OverlayId, blocks_chat: bool, sub_input_active: bool) -> OverlayEntry {
        OverlayEntry {
            id,
            blocks_chat,
            dim_backdrop: blocks_chat,
            sub_input_active,
        }
    }

    #[test]
    fn stack_push_raise_remove_and_focus_scope() {
        let mut stack = OverlayStack::default();
        assert!(stack.is_empty());
        assert_eq!(stack.len(), 0);
        assert!(!stack.blocks_chat());
        assert_eq!(stack.focus_scope(), FocusScope::ChatComposer);

        stack.push(entry(OverlayId::Help, false, false));
        assert_eq!(stack.len(), 1);
        assert!(!stack.blocks_chat());
        assert_eq!(stack.focus_scope(), FocusScope::Overlay(OverlayId::Help));

        stack.push(entry(OverlayId::Settings, true, false));
        assert_eq!(stack.len(), 2);
        assert!(stack.blocks_chat());
        assert_eq!(
            stack.focus_scope(),
            FocusScope::Overlay(OverlayId::Settings)
        );

        stack.set_sub_input_active(OverlayId::Settings, true);
        assert_eq!(
            stack.focus_scope(),
            FocusScope::OverlaySubInput(OverlayId::Settings)
        );

        // Raising an existing overlay moves it to top without duplicating it.
        stack.raise_or_push(entry(OverlayId::Help, true, false));
        assert_eq!(stack.len(), 2);
        let ids: Vec<OverlayId> = stack.iter_bottom_to_top().map(|e| e.id).collect();
        assert_eq!(ids, vec![OverlayId::Settings, OverlayId::Help]);
        assert_eq!(stack.focus_scope(), FocusScope::Overlay(OverlayId::Help));

        assert!(stack.contains(OverlayId::Settings));
        assert!(stack.remove(OverlayId::Settings));
        assert!(!stack.remove(OverlayId::Settings));
        assert_eq!(stack.len(), 1);

        let popped = stack.pop().expect("top overlay should pop");
        assert_eq!(popped.id, OverlayId::Help);
        assert!(stack.is_empty());
        assert_eq!(stack.focus_scope(), FocusScope::ChatComposer);
    }

    #[test]
    fn key_press_filters_release_events() {
        let press =
            KeyEvent::new_with_kind(KeyCode::Char('a'), KeyModifiers::NONE, KeyEventKind::Press);
        let repeat =
            KeyEvent::new_with_kind(KeyCode::Char('a'), KeyModifiers::NONE, KeyEventKind::Repeat);
        let release = KeyEvent::new_with_kind(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            KeyEventKind::Release,
        );

        assert!(is_key_press(press));
        assert!(is_key_press(repeat));
        assert!(!is_key_press(release));
    }
}

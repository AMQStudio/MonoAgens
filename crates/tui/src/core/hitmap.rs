//! Z-ordered hit-testing map for mouse clicks, hover, and scrollbar drags.

use ratatui::layout::Rect;

use crate::core::overlay::OverlayId;

/// Identifies a scrollable region associated with a rendered scrollbar track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScrollId {
    Chat,
    Sidebar,
    SettingsRail,
    SettingsList,
    RbacRail,
    RbacList,
    CatalogList,
    GovernanceList,
    Help,
    Palette,
    Log,
    Wizard,
    ConvModal,
}

/// Interactive UI element recorded during rendering for mouse hit-testing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitTarget {
    ChatFeed,
    ChatMessage(usize),
    Composer,
    SidebarChat(String),
    SidebarPlugin(String),
    SidebarActor(String),
    Tab(usize),
    RailRow(usize),
    ListRow(usize),
    ToggleRow(usize),
    Scrollbar {
        id: ScrollId,
        track: Rect,
        total_rows: usize,
        viewport_rows: usize,
    },
    FooterChip(usize),
    Button(String),
    OverlayBody(OverlayId),
    Backdrop(OverlayId),
}

/// Paint-ordered hit-test table; reverse iteration ensures the topmost rendered target wins.
#[derive(Debug, Clone, Default)]
pub struct HitMap {
    entries: Vec<(Rect, HitTarget)>,
}

impl HitMap {
    /// Clears all recorded regions at the start of a frame.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Records a clickable/scrollable region, ignoring zero-area rectangles.
    pub fn record(&mut self, rect: Rect, target: HitTarget) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        self.entries.push((rect, target));
    }

    /// Returns the topmost [`HitTarget`] containing `(col, row)`, if any.
    #[must_use]
    pub fn hit_test(&self, col: u16, row: u16) -> Option<&HitTarget> {
        self.entries
            .iter()
            .rev()
            .find(|(rect, _)| rect_contains(*rect, col, row))
            .map(|(_, target)| target)
    }

    /// Returns scrollbar metadata if the topmost target at `(col, row)` is a [`HitTarget::Scrollbar`].
    #[must_use]
    pub fn scrollbar_at(&self, col: u16, row: u16) -> Option<(ScrollId, Rect, usize, usize)> {
        match self.hit_test(col, row) {
            Some(HitTarget::Scrollbar {
                id,
                track,
                total_rows,
                viewport_rows,
            }) => Some((*id, *track, *total_rows, *viewport_rows)),
            _ => None,
        }
    }
}

#[must_use]
const fn rect_contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x
        && col < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_recorded_wins_and_boundaries_are_exclusive() {
        let mut map = HitMap::default();
        map.record(Rect::new(0, 0, 0, 10), HitTarget::ChatFeed);
        map.record(Rect::new(0, 0, 10, 0), HitTarget::ChatFeed);
        assert_eq!(map.hit_test(0, 0), None);

        let outer = Rect::new(2, 3, 10, 5);
        let inner = Rect::new(4, 4, 4, 2);
        map.record(outer, HitTarget::OverlayBody(OverlayId::Settings));
        map.record(inner, HitTarget::Button("save".to_owned()));

        assert_eq!(
            map.hit_test(2, 3),
            Some(&HitTarget::OverlayBody(OverlayId::Settings))
        );
        assert_eq!(
            map.hit_test(4, 4),
            Some(&HitTarget::Button("save".to_owned()))
        );
        // Exclusive right/bottom edges (`x + width`, `y + height`).
        assert_eq!(map.hit_test(12, 3), None);
        assert_eq!(map.hit_test(2, 8), None);
        assert_eq!(
            map.hit_test(11, 7),
            Some(&HitTarget::OverlayBody(OverlayId::Settings))
        );
    }

    #[test]
    fn scrollbar_lookup_respects_z_order() {
        let mut map = HitMap::default();
        let track = Rect::new(20, 2, 1, 10);
        map.record(
            track,
            HitTarget::Scrollbar {
                id: ScrollId::Chat,
                track,
                total_rows: 100,
                viewport_rows: 10,
            },
        );
        assert_eq!(
            map.scrollbar_at(20, 5),
            Some((ScrollId::Chat, track, 100, 10))
        );

        // Modal backdrop above the scrollbar shadows it.
        map.record(
            Rect::new(0, 0, 40, 20),
            HitTarget::Backdrop(OverlayId::Palette),
        );
        assert_eq!(map.scrollbar_at(20, 5), None);
    }
}

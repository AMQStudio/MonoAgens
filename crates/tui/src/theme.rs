//! Theme presets (`Titanium`, `Slate`, `Terracotta`, `Light`) and `NO_COLOR` support.

use ratatui::style::Color;
use serde::{Deserialize, Serialize};

/// Visual theme preset selectable in `/settings -> Appearance` and persisted in `tui.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreset {
    #[default]
    Titanium,
    Slate,
    Terracotta,
    Light,
}

impl ThemePreset {
    /// All available presets in cycle order.
    pub const ALL: [Self; 4] = [Self::Titanium, Self::Slate, Self::Terracotta, Self::Light];

    /// Display label for UI menus and previews.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Titanium => "Titanium (Dark)",
            Self::Slate => "Slate (Mono Dark)",
            Self::Terracotta => "Terracotta (Warm Dark)",
            Self::Light => "Light",
        }
    }

    /// Canonical identifier stored in `tui.toml`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Titanium => "titanium",
            Self::Slate => "slate",
            Self::Terracotta => "terracotta",
            Self::Light => "light",
        }
    }

    /// Cycles to the next preset.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Titanium => Self::Slate,
            Self::Slate => Self::Terracotta,
            Self::Terracotta => Self::Light,
            Self::Light => Self::Titanium,
        }
    }
}

/// Returns `true` when the `NO_COLOR` environment variable is set to a non-empty value.
#[must_use]
pub fn no_color_active() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty())
}

/// Resolved color palette for rendering the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    pub no_color: bool,
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub muted: Color,
    pub accent: Color,
    pub accent_fg: Color,
    pub bubble_bg: Color,
    pub bubble_fg: Color,
    pub sidebar_bg: Color,
    pub status_bg: Color,
    pub modal_bg: Color,
    pub selection_bg: Color,
    pub separator: Color,
    pub status_green: Color,
    pub status_yellow: Color,
    pub status_red: Color,
    pub lab_badge_bg: Color,
    pub lab_badge_fg: Color,
    pub approval_bar: Color,
    pub shimmer_hi: Color,
    pub shimmer_lo: Color,
}

impl ThemePalette {
    /// Builds a palette for `preset` honoring the process `NO_COLOR` environment variable.
    #[must_use]
    pub fn for_preset(preset: ThemePreset) -> Self {
        Self::for_preset_with_no_color(preset, no_color_active())
    }

    /// Builds a palette for `preset` with an explicit `no_color` flag (deterministic in tests).
    #[must_use]
    pub const fn for_preset_with_no_color(preset: ThemePreset, no_color: bool) -> Self {
        if no_color {
            return Self {
                no_color: true,
                bg: Color::Reset,
                fg: Color::Reset,
                dim: Color::Reset,
                muted: Color::Reset,
                accent: Color::Reset,
                accent_fg: Color::Reset,
                bubble_bg: Color::Reset,
                bubble_fg: Color::Reset,
                sidebar_bg: Color::Reset,
                status_bg: Color::Reset,
                modal_bg: Color::Reset,
                selection_bg: Color::Reset,
                separator: Color::Reset,
                status_green: Color::Reset,
                status_yellow: Color::Reset,
                status_red: Color::Reset,
                lab_badge_bg: Color::Reset,
                lab_badge_fg: Color::Reset,
                approval_bar: Color::Reset,
                shimmer_hi: Color::Reset,
                shimmer_lo: Color::Reset,
            };
        }

        match preset {
            ThemePreset::Titanium => Self {
                no_color: false,
                bg: Color::Rgb(15, 17, 23),
                fg: Color::Rgb(226, 232, 240),
                dim: Color::Rgb(100, 116, 139),
                muted: Color::Rgb(148, 163, 184),
                accent: Color::Rgb(56, 189, 248),
                accent_fg: Color::Rgb(15, 23, 42),
                bubble_bg: Color::Rgb(30, 35, 48),
                bubble_fg: Color::Rgb(241, 245, 249),
                sidebar_bg: Color::Rgb(18, 21, 29),
                status_bg: Color::Rgb(21, 25, 34),
                modal_bg: Color::Rgb(22, 26, 36),
                selection_bg: Color::Rgb(38, 49, 68),
                separator: Color::Rgb(51, 65, 85),
                status_green: Color::Green,
                status_yellow: Color::Yellow,
                status_red: Color::Red,
                lab_badge_bg: Color::Rgb(180, 83, 9),
                lab_badge_fg: Color::Rgb(254, 243, 199),
                approval_bar: Color::Rgb(250, 204, 21),
                shimmer_hi: Color::Rgb(125, 211, 252),
                shimmer_lo: Color::Rgb(56, 189, 248),
            },
            ThemePreset::Slate => Self {
                no_color: false,
                bg: Color::Rgb(18, 19, 22),
                fg: Color::Rgb(229, 231, 235),
                dim: Color::Rgb(107, 114, 128),
                muted: Color::Rgb(156, 163, 175),
                accent: Color::Rgb(203, 213, 225),
                accent_fg: Color::Rgb(17, 24, 39),
                bubble_bg: Color::Rgb(39, 42, 48),
                bubble_fg: Color::Rgb(243, 244, 246),
                sidebar_bg: Color::Rgb(22, 24, 28),
                status_bg: Color::Rgb(26, 28, 33),
                modal_bg: Color::Rgb(26, 28, 33),
                selection_bg: Color::Rgb(55, 65, 81),
                separator: Color::Rgb(63, 68, 78),
                status_green: Color::Green,
                status_yellow: Color::Yellow,
                status_red: Color::Red,
                lab_badge_bg: Color::Rgb(75, 85, 99),
                lab_badge_fg: Color::Rgb(249, 250, 251),
                approval_bar: Color::Rgb(229, 231, 235),
                shimmer_hi: Color::Rgb(243, 244, 246),
                shimmer_lo: Color::Rgb(156, 163, 175),
            },
            ThemePreset::Terracotta => Self {
                no_color: false,
                bg: Color::Rgb(23, 19, 18),
                fg: Color::Rgb(242, 233, 228),
                dim: Color::Rgb(140, 122, 114),
                muted: Color::Rgb(182, 163, 154),
                accent: Color::Rgb(224, 122, 95),
                accent_fg: Color::Rgb(23, 19, 18),
                bubble_bg: Color::Rgb(46, 36, 33),
                bubble_fg: Color::Rgb(250, 242, 238),
                sidebar_bg: Color::Rgb(28, 22, 20),
                status_bg: Color::Rgb(32, 26, 24),
                modal_bg: Color::Rgb(33, 27, 24),
                selection_bg: Color::Rgb(68, 49, 43),
                separator: Color::Rgb(79, 61, 55),
                status_green: Color::Green,
                status_yellow: Color::Yellow,
                status_red: Color::Red,
                lab_badge_bg: Color::Rgb(154, 52, 18),
                lab_badge_fg: Color::Rgb(255, 237, 213),
                approval_bar: Color::Rgb(245, 158, 11),
                shimmer_hi: Color::Rgb(244, 162, 97),
                shimmer_lo: Color::Rgb(224, 122, 95),
            },
            ThemePreset::Light => Self {
                no_color: false,
                bg: Color::Rgb(248, 250, 252),
                fg: Color::Rgb(15, 23, 42),
                dim: Color::Rgb(100, 116, 139),
                muted: Color::Rgb(71, 85, 105),
                accent: Color::Rgb(2, 132, 199),
                accent_fg: Color::Rgb(248, 250, 252),
                bubble_bg: Color::Rgb(226, 232, 240),
                bubble_fg: Color::Rgb(15, 23, 42),
                sidebar_bg: Color::Rgb(241, 245, 249),
                status_bg: Color::Rgb(226, 232, 240),
                modal_bg: Color::Rgb(255, 255, 255),
                selection_bg: Color::Rgb(186, 230, 253),
                separator: Color::Rgb(203, 213, 225),
                status_green: Color::Green,
                status_yellow: Color::Yellow,
                status_red: Color::Red,
                lab_badge_bg: Color::Rgb(217, 119, 6),
                lab_badge_fg: Color::Rgb(255, 255, 255),
                approval_bar: Color::Rgb(217, 119, 6),
                shimmer_hi: Color::Rgb(14, 165, 233),
                shimmer_lo: Color::Rgb(2, 132, 199),
            },
        }
    }
}

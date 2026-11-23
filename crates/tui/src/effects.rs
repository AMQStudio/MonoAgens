//! `tachyonfx` effect manager, braille spinner, and status-line shimmer helpers.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use tachyonfx::{Duration as FxDuration, EffectManager, EffectTimer, Interpolation, fx};

use crate::config::{MotionMode, ShimmerMode};
use crate::theme::ThemePalette;

/// Unique keys for cancellable `tachyonfx` effects.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum EffectKey {
    #[default]
    MessageFadeIn,
    ModalDissolve,
    ToastFade,
}

/// Manages frame-delta `tachyonfx` effects, spinner cadence, and thinking shimmer.
#[derive(Debug, Default)]
pub struct TuiEffects {
    manager: EffectManager<EffectKey>,
    spinner_tick: u64,
    shimmer_phase: usize,
}

const BRAILLE_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

impl TuiEffects {
    /// Creates an empty [`TuiEffects`] processor.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when any `tachyonfx` effect is actively running.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.manager.is_running()
    }

    /// Advances spinner and shimmer counters on an animation tick.
    pub fn advance_tick(&mut self) {
        self.spinner_tick = self.spinner_tick.wrapping_add(1);
        self.shimmer_phase = self.shimmer_phase.wrapping_add(1);
    }

    /// Returns the current braille spinner glyph (or static glyph when motion is `Off`).
    #[must_use]
    pub fn spinner_glyph(&self, motion: MotionMode) -> &'static str {
        if motion == MotionMode::Off {
            "◐"
        } else {
            let idx = (self.spinner_tick as usize) % BRAILLE_FRAMES.len();
            BRAILLE_FRAMES[idx]
        }
    }

    /// Triggers a `160 ms` message fade-in effect on the chat area.
    pub fn trigger_message_fade_in(&mut self, motion: MotionMode, palette: &ThemePalette) {
        if palette.no_color || motion == MotionMode::Off {
            return;
        }
        let ms = match motion {
            MotionMode::Full => 160,
            MotionMode::Reduced => 60,
            MotionMode::Off => return,
        };
        let timer = EffectTimer::from_ms(ms, Interpolation::CubicOut);
        self.manager
            .add_unique_effect(EffectKey::MessageFadeIn, fx::fade_from_fg(palette.dim, timer));
    }

    /// Triggers a `120 ms` modal dissolve-in effect.
    pub fn trigger_modal_dissolve(&mut self, motion: MotionMode, palette: &ThemePalette) {
        if palette.no_color || motion == MotionMode::Off {
            return;
        }
        let ms = match motion {
            MotionMode::Full => 120,
            MotionMode::Reduced => 50,
            MotionMode::Off => return,
        };
        let timer = EffectTimer::from_ms(ms, Interpolation::QuadOut);
        self.manager
            .add_unique_effect(EffectKey::ModalDissolve, fx::coalesce(timer));
    }

    /// Triggers a `2.5 s` toast fade-out effect.
    pub fn trigger_toast(&mut self, motion: MotionMode, palette: &ThemePalette) {
        if palette.no_color || motion == MotionMode::Off {
            return;
        }
        let ms = match motion {
            MotionMode::Full => 2500,
            MotionMode::Reduced => 1200,
            MotionMode::Off => return,
        };
        let timer = EffectTimer::from_ms(ms, Interpolation::Linear);
        self.manager
            .add_unique_effect(EffectKey::ToastFade, fx::fade_to_fg(palette.dim, timer));
    }

    /// Processes active effects onto `buf` for the given `elapsed_ms`.
    pub fn process(&mut self, elapsed_ms: u32, buf: &mut Buffer, area: Rect) {
        if !self.manager.is_running() || area.width == 0 || area.height == 0 {
            return;
        }
        let dt = FxDuration::from_millis(elapsed_ms.max(1));
        self.manager.process_effects(dt, buf, area);
    }

    /// Computes a per-character style for the thinking shimmer indicator.
    #[must_use]
    pub fn shimmer_style(
        &self,
        char_index: usize,
        shimmer: ShimmerMode,
        motion: MotionMode,
        palette: &ThemePalette,
    ) -> Style {
        if palette.no_color || shimmer == ShimmerMode::Off || motion == MotionMode::Off {
            return Style::default().fg(palette.dim);
        }
        let wave_pos = self.shimmer_phase % 16;
        let dist = wave_pos.abs_diff(char_index % 16);
        let fg = match shimmer {
            ShimmerMode::Classic => {
                if dist <= 1 {
                    palette.accent
                } else if dist == 2 {
                    palette.shimmer_lo
                } else {
                    palette.dim
                }
            }
            ShimmerMode::Subtle => {
                if dist <= 1 {
                    palette.shimmer_lo
                } else {
                    palette.dim
                }
            }
            ShimmerMode::Off => palette.dim,
        };
        let mut style = Style::default().fg(fg);
        if shimmer == ShimmerMode::Classic && dist == 0 && fg != Color::Reset {
            style = style.add_modifier(Modifier::BOLD);
        }
        style
    }
}

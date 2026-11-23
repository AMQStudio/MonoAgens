//! Capability catalog parsed from `capabilities.list` JSON with graceful fallback
//! to `Core Agent` (`agent.converse`) and configured [`PluginUiEntry`] items.

use std::collections::BTreeSet;

use monoagens_valve::rbac::RoleDefinition;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::core::hitmap::{HitMap, HitTarget, ScrollId};
use crate::core::labels::{friendly_capability, friendly_plugin};
use crate::core::theme::ThemePalette;
use crate::core::viewport::clamp_scroll_offset;
use crate::settings::{PluginUiEntry, PluginUiState};
use crate::views::rbac::matrix::capability_state;
use crate::views::rbac::{RbacPaneFocus, RbacViewState, ToggleState};
use crate::widgets::scrollbar::render_vertical_scrollbar;

/// Risk/effect flags attached to a capability entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CapabilityFlagsInfo {
    pub communicative: bool,
    pub identity_changing: bool,
    pub irreversible: bool,
}

/// Single capability descriptor within a [`CapabilityGroup`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityEntry {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub risk_tier: String,
    pub owner_only: bool,
    pub flags: CapabilityFlagsInfo,
}

impl CapabilityEntry {
    /// Formats `"{display_name} ({id})"` for matrix rows and search matching.
    #[must_use]
    pub fn inline_label(&self) -> String {
        format!("{} ({})", self.display_name, self.id)
    }
}

/// Logical capability domain (Core Agent or a configured/running plugin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityGroup {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub running: bool,
    pub body_id: Option<String>,
    pub capabilities: Vec<CapabilityEntry>,
}

/// Complete capability catalog shown in the RBAC toggle matrix.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CapabilityCatalog {
    pub groups: Vec<CapabilityGroup>,
}

impl CapabilityCatalog {
    /// Builds a [`CapabilityCatalog`] from optional `capabilities.list` JSON and `plugins`.
    #[must_use]
    pub fn from_json_and_plugins(caps_json: Option<&Value>, plugins: &[PluginUiEntry]) -> Self {
        let Some(root) = caps_json else {
            return Self::fallback(plugins);
        };
        let (mut groups, mut seen) = (Vec::new(), BTreeSet::new());
        for raw in root.get("groups").and_then(Value::as_array).into_iter().flatten() {
            if let Some(g) = parse_group(raw) && seen.insert(g.id.clone()) {
                groups.push(g);
            }
        }
        if groups.is_empty() && let Some(caps) = root.get("capabilities").and_then(Value::as_array) {
            groups = group_flat_caps(caps, plugins, &mut seen);
        }
        if groups.is_empty() {
            return Self::fallback(plugins);
        }
        if !seen.contains("core") {
            groups.insert(0, default_core_group());
            seen.insert("core".into());
        }
        for p in plugins {
            if seen.insert(p.id.clone()) {
                groups.push(plugin_group(p, Vec::new()));
            }
        }
        Self { groups }
    }

    /// Builds a minimal offline catalog containing `Core Agent` (`agent.converse`) plus `plugins`.
    #[must_use]
    pub fn fallback(plugins: &[PluginUiEntry]) -> Self {
        let mut groups = vec![default_core_group()];
        let mut seen = BTreeSet::from(["core".to_owned()]);
        for p in plugins {
            if seen.insert(p.id.clone()) {
                groups.push(plugin_group(p, Vec::new()));
            }
        }
        Self { groups }
    }

    /// Looks up a group by canonical `id`.
    #[must_use]
    pub fn group(&self, id: &str) -> Option<&CapabilityGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Returns `(enabled, total)` capabilities inside `group` for `role`.
    #[must_use]
    pub fn enabled_count(&self, role: Option<&RoleDefinition>, grp: &CapabilityGroup) -> (usize, usize) {
        let on = role.map_or(0, |r| grp.capabilities.iter().filter(|c| capability_state(r, c) == ToggleState::On).count());
        (on, grp.capabilities.len())
    }
}

fn default_core_group() -> CapabilityGroup {
    let conv = CapabilityEntry {
        id: "agent.converse".into(),
        display_name: friendly_capability("agent.converse").label,
        description: "Allow incoming messages to wake the agent.".into(),
        risk_tier: "R0".into(),
        owner_only: false,
        flags: CapabilityFlagsInfo::default(),
    };
    CapabilityGroup { id: "core".into(), label: "Core Agent".into(), kind: "core".into(), running: true, body_id: None, capabilities: vec![conv] }
}

fn plugin_group(p: &PluginUiEntry, capabilities: Vec<CapabilityEntry>) -> CapabilityGroup {
    CapabilityGroup {
        id: p.id.clone(),
        label: friendly_plugin(&p.id).label,
        kind: p.kind.as_str().into(),
        running: matches!(p.state, PluginUiState::Running),
        body_id: None,
        capabilities,
    }
}

fn jstr<'a>(v: &'a Value, k: &str) -> &'a str { v.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("") }
fn jbool(v: &Value, k: &str) -> bool { v.get(k).and_then(Value::as_bool).unwrap_or(false) }

fn parse_group(raw: &Value) -> Option<CapabilityGroup> {
    let id = jstr(raw, "id");
    if id.is_empty() { return None; }
    let lbl = jstr(raw, "label");
    let label = if !lbl.is_empty() { lbl.into() } else if id == "core" { "Core Agent".into() } else { friendly_plugin(id).label };
    let kind = match jstr(raw, "kind") { "" => "adapter", k => k }.into();
    let capabilities = raw.get("capabilities").and_then(Value::as_array).map(|a| a.iter().filter_map(parse_cap).collect()).unwrap_or_default();
    Some(CapabilityGroup { id: id.into(), label, kind, running: jbool(raw, "running"), body_id: raw.get("body_id").and_then(Value::as_str).map(str::to_owned), capabilities })
}

fn parse_cap(raw: &Value) -> Option<CapabilityEntry> {
    let id = jstr(raw, "id");
    if id.is_empty() { return None; }
    let disp = jstr(raw, "display_name");
    let display_name = if disp.is_empty() { friendly_capability(id).label } else { disp.into() };
    let flags = raw.get("flags").map_or_else(CapabilityFlagsInfo::default, |f| CapabilityFlagsInfo {
        communicative: jbool(f, "communicative"), identity_changing: jbool(f, "identity_changing"), irreversible: jbool(f, "irreversible"),
    });
    let risk_tier = match jstr(raw, "risk_tier") { "" => "R0", r => r }.into();
    let owner_only = raw.get("owner_only").and_then(Value::as_bool).unwrap_or(flags.identity_changing);
    Some(CapabilityEntry { id: id.into(), display_name, description: jstr(raw, "description").into(), risk_tier, owner_only, flags })
}

fn group_flat_caps(caps: &[Value], plugins: &[PluginUiEntry], seen: &mut BTreeSet<String>) -> Vec<CapabilityGroup> {
    let mut groups: Vec<CapabilityGroup> = Vec::new();
    for raw in caps {
        let Some(cap) = parse_cap(raw) else { continue };
        let pid = match jstr(raw, "plugin_id") { "" => "core", p => p };
        if let Some(g) = groups.iter_mut().find(|g| g.id == pid) { g.capabilities.push(cap); continue; }
        seen.insert(pid.into());
        if let Some(pl) = plugins.iter().find(|p| p.id == pid) {
            groups.push(plugin_group(pl, vec![cap]));
        } else {
            let label = if pid == "core" { "Core Agent".into() } else { friendly_plugin(pid).label };
            groups.push(CapabilityGroup { id: pid.into(), label, kind: "adapter".into(), running: pid == "core", body_id: None, capabilities: vec![cap] });
        }
    }
    groups
}

pub(super) type PaneRow = (String, bool, bool);
pub(super) struct PaneData { pub rail: Vec<PaneRow>, pub matrix: Vec<PaneRow>, pub header: Option<String>, pub insp1: String, pub insp2: String, pub insp_color: Option<Color> }

#[allow(clippy::too_many_arguments)]
pub(super) fn render_panes_and_inspector(
    st: &RbacViewState, rail_area: Option<Rect>, main_area: Rect, insp_area: Option<Rect>,
    buf: &mut Buffer, data: &PaneData, pal: &ThemePalette, hm: &mut HitMap,
) {
    let r_foc = st.focus == RbacPaneFocus::Rail;
    if let Some(r) = rail_area {
        st.viewport_rail_rows.set(usize::from(r.height));
        render_rows(r, buf, &data.rail, st.rail_idx, st.rail_scroll, r_foc, true, pal, hm);
    }
    if rail_area.is_none() && r_foc {
        st.viewport_rail_rows.set(usize::from(main_area.height));
        render_rows(main_area, buf, &data.rail, st.rail_idx, st.rail_scroll, true, true, pal, hm);
    } else {
        let la = if let Some(hdr) = &data.header && main_area.height > 1 {
            let s = Style::default().fg(pal.accent).bg(pal.modal_bg).add_modifier(Modifier::BOLD);
            buf.set_line(main_area.x, main_area.y, &Line::from(Span::styled(clip_text(hdr, usize::from(main_area.width)), s)), main_area.width);
            Rect::new(main_area.x, main_area.y + 1, main_area.width, main_area.height - 1)
        } else { main_area };
        st.viewport_matrix_rows.set(usize::from(la.height));
        render_rows(la, buf, &data.matrix, st.matrix_idx, st.matrix_scroll, !r_foc, false, pal, hm);
    }
    if let Some(a) = insp_area && a.width > 0 && a.height > 0 {
        let w = usize::from(a.width);
        buf.set_line(a.x, a.y, &Line::from(Span::styled("─".repeat(w), Style::default().fg(pal.separator).bg(pal.modal_bg))), a.width);
        if a.height >= 2 { buf.set_line(a.x, a.y + 1, &Line::from(Span::styled(clip_text(&data.insp1, w), Style::default().fg(pal.fg).bg(pal.modal_bg))), a.width); }
        if a.height >= 3 { buf.set_line(a.x, a.y + 2, &Line::from(Span::styled(clip_text(&data.insp2, w), Style::default().fg(data.insp_color.unwrap_or(pal.muted)).bg(pal.modal_bg).add_modifier(Modifier::BOLD))), a.width); }
    }
}

fn row_style(ok: bool, sel: bool, foc: bool, hi: bool, pal: &ThemePalette) -> Style {
    let base = Style::default().bg(pal.modal_bg);
    if !ok { return base.fg(pal.muted).add_modifier(Modifier::BOLD); }
    if sel && foc {
        return if pal.no_color { Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD) } else { Style::default().fg(pal.accent_fg).bg(pal.accent).add_modifier(Modifier::BOLD) };
    }
    if sel { return Style::default().fg(pal.fg).bg(pal.selection_bg).add_modifier(Modifier::BOLD); }
    base.fg(if hi { pal.fg } else { pal.dim })
}

#[allow(clippy::too_many_arguments)]
fn render_rows(area: Rect, buf: &mut Buffer, rows: &[PaneRow], sel: usize, raw_scroll: usize, foc: bool, is_rail: bool, pal: &ThemePalette, hm: &mut HitMap) {
    if area.width == 0 || area.height == 0 { return; }
    let vp = usize::from(area.height);
    let scroll = clamp_scroll_offset(raw_scroll, rows.len(), vp);
    let has_sb = rows.len() > vp && area.width > 4;
    let cw = if has_sb { area.width - 1 } else { area.width };
    for row_off in 0..vp {
        let idx = scroll + row_off;
        let Some((text, ok, hi)) = rows.get(idx) else { break };
        let y = area.y + row_off as u16;
        let is_sel = *ok && idx == sel;
        let pfx = if !*ok { "" } else if is_sel { "› " } else { "  " };
        let full = clip_text(&format!("{pfx}{text}"), usize::from(cw));
        let pad = usize::from(cw).saturating_sub(UnicodeWidthStr::width(full.as_str()));
        let style = row_style(*ok, is_sel, foc, *hi, pal);
        buf.set_line(area.x, y, &Line::from(vec![Span::styled(full, style), Span::styled(" ".repeat(pad), style)]), cw);
        if *ok { hm.record(Rect::new(area.x, y, cw, 1), if is_rail { HitTarget::RailRow(idx) } else { HitTarget::ToggleRow(idx) }); }
    }
    if has_sb {
        let id = if is_rail { ScrollId::RbacRail } else { ScrollId::RbacList };
        render_vertical_scrollbar(buf, Rect::new(area.x + cw, area.y, 1, area.height), id, rows.len(), vp, scroll, hm, Style::default().fg(pal.separator).bg(pal.modal_bg), Style::default().fg(pal.accent).bg(pal.modal_bg));
    }
}

pub(super) fn clip_text(text: &str, max_w: usize) -> String {
    let (mut out, mut used) = (String::new(), 0usize);
    for g in text.graphemes(true) {
        let w = UnicodeWidthStr::width(g);
        if used + w > max_w { break; }
        out.push_str(g);
        used += w;
    }
    out
}

pub(super) fn move_idx<T>(items: &[T], idx: &mut usize, dir: isize, is_sel: impl Fn(&T) -> bool) {
    if items.is_empty() { *idx = 0; return; }
    let cur = (*idx).min(items.len() - 1);
    if dir < 0 { if let Some(p) = (0..cur).rev().find(|&i| is_sel(&items[i])) { *idx = p; } }
    else if let Some(n) = ((cur + 1)..items.len()).find(|&i| is_sel(&items[i])) { *idx = n; }
}

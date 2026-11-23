//! RBAC Simple ON/OFF Toggle Matrix view (`Roles`, `Actors`, and `Places & Bases`)
//! rendered inside [`HubFrame`].

pub mod actors_places;
pub mod catalog;
pub mod matrix;

use std::cell::Cell;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use monoagens_valve::rbac::RbacConfig;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::{Value, json};

pub use catalog::{CapabilityCatalog, CapabilityEntry, CapabilityGroup};

use crate::core::hitmap::{HitMap, HitTarget, ScrollId};
use crate::core::input::{PasteTarget, TextInput};
use crate::core::labels::matches_friendly_query;
use crate::core::overlay::{OverlayId, WidgetOutcome, is_key_press};
use crate::core::theme::ThemePalette;
use crate::core::viewport::{
    clamp_scroll_offset, is_horizontal_scroll, offset_from_scrollbar_y, scroll_offset_for_row,
};
use crate::settings::PluginUiEntry;
use crate::widgets::hub_frame::{FooterChip, FooterMode, HubFrameConfig, render_hub_frame};
use crate::widgets::tab_bar::{TabBar, TabItem};

use actors_places::{
    ActorRailItem, PlaceBaseRailItem, activate_place_base, actor_rail_items, actors_pane_data,
    apply, handle_actors_key, handle_places_bases_key, nav_delta, open_prompt, place_base_rows,
    places_bases_pane_data, places_bases_rail_items, selected_actor, step_idx, submit_prompt,
    toast, toggle_actor_role,
};
use catalog::{PaneData, PaneRow, move_idx, render_panes_and_inspector};
use matrix::{
    CUSTOM_DOMAIN_ID, approval_mode_label, capability_state, cycle_role_approval, cycle_role_base,
    delete_role_with_confirm, evaluate_inspector, group_state,
    toggle_capability_patch_with_catalog, toggle_group_patch_with_catalog,
};

/// Active sub-tab inside the RBAC hub view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RbacSubTab {
    #[default]
    Roles,
    Actors,
    PlacesBases,
}

impl RbacSubTab {
    const fn index(self) -> usize {
        match self {
            Self::Roles => 0,
            Self::Actors => 1,
            Self::PlacesBases => 2,
        }
    }

    const fn from_index(idx: usize) -> Option<Self> {
        match idx {
            0 => Some(Self::Roles),
            1 => Some(Self::Actors),
            2 => Some(Self::PlacesBases),
            _ => None,
        }
    }
}

/// Focused pane inside the 2-pane RBAC layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RbacPaneFocus {
    #[default]
    Rail,
    Matrix,
}

/// Visual ON/OFF/Partial/Owner-locked state of a capability or group toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleState {
    On,
    Off,
    Partial,
    LockedOwner,
}

impl ToggleState {
    /// Bracketed checkbox glyph rendered in the toggle matrix.
    #[must_use]
    pub const fn glyph(self) -> &'static str {
        match self {
            Self::On => "[✓]",
            Self::Off => "[ ]",
            Self::Partial => "[–]",
            Self::LockedOwner => "[🔒]",
        }
    }
}

/// Action emitted by [`RbacViewState`] to the host overlay/settings controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RbacAction {
    ApplyPatch { patch: Value, toast: String },
    Toast { message: String, error: bool },
    Close,
}

/// Target for an inline footer [`TextInput`] prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RbacPromptKind {
    AddRole,
    EditRoleLabel(String),
    AddCustomPattern,
    AddActor,
    AddPlace,
    SetPlaceAllow(String),
    SetPlaceDeny(String),
    AddPermissionBase,
}

/// Active inline text prompt displayed in the pinned footer strip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RbacInlinePrompt {
    pub kind: RbacPromptKind,
    pub input: TextInput,
    pub prompt: String,
    pub hint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RoleRailEntry {
    Heading,
    Role(String),
    AddRole,
    Domain(String),
    CustomPatterns,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RoleMatrixRow {
    GroupMaster,
    Capability(CapabilityEntry),
    CustomAllow(String),
    CustomDeny(String),
    AddCustomPattern,
}

/// Stateful controller and renderer for the Wave 2 RBAC toggle matrix.
#[derive(Debug, Clone)]
pub struct RbacViewState {
    pub sub_tab: RbacSubTab,
    pub focus: RbacPaneFocus,
    pub catalog: CapabilityCatalog,
    pub selected_role: String,
    pub selected_domain_id: String,
    pub rail_idx: usize,
    pub rail_scroll: usize,
    pub matrix_idx: usize,
    pub matrix_scroll: usize,
    pub viewport_rail_rows: Cell<usize>,
    pub viewport_matrix_rows: Cell<usize>,
    pub filter_input: TextInput,
    pub filter_active: bool,
    pub inline_input: Option<RbacInlinePrompt>,
    pub delete_confirm_role: Option<String>,
    pub seen_actors: Vec<String>,
}

impl Default for RbacViewState {
    fn default() -> Self {
        Self {
            sub_tab: RbacSubTab::Roles,
            focus: RbacPaneFocus::Rail,
            catalog: CapabilityCatalog::fallback(&[]),
            selected_role: "guest".to_owned(),
            selected_domain_id: "core".to_owned(),
            rail_idx: 1,
            rail_scroll: 0,
            matrix_idx: 0,
            matrix_scroll: 0,
            viewport_rail_rows: Cell::new(0),
            viewport_matrix_rows: Cell::new(0),
            filter_input: TextInput::new(false).with_placeholder("filter capabilities..."),
            filter_active: false,
            inline_input: None,
            delete_confirm_role: None,
            seen_actors: Vec::new(),
        }
    }
}

impl RbacViewState {
    /// Synchronizes catalog, actors, and selection bounds from gateway config & capabilities.
    pub fn sync_from_config(
        &mut self,
        rbac: &RbacConfig,
        caps_json: Option<&Value>,
        plugins: &[PluginUiEntry],
        seen_actors: &[String],
    ) {
        self.catalog = CapabilityCatalog::from_json_and_plugins(caps_json, plugins);
        self.seen_actors = seen_actors.to_vec();
        if !rbac.roles.contains_key(&self.selected_role) {
            self.selected_role = if rbac.roles.contains_key("guest") {
                "guest".into()
            } else if let Some(fb) = &rbac.fallback_role
                && rbac.roles.contains_key(fb)
            {
                fb.clone()
            } else {
                rbac.roles
                    .keys()
                    .next()
                    .cloned()
                    .unwrap_or_else(|| "guest".into())
            };
        }
        if self.selected_domain_id != CUSTOM_DOMAIN_ID
            && self.catalog.group(&self.selected_domain_id).is_none()
        {
            self.selected_domain_id = self
                .catalog
                .groups
                .first()
                .map_or_else(|| CUSTOM_DOMAIN_ID.into(), |g| g.id.clone());
        }
        let (rl, ml) = (self.rail_len(rbac), self.matrix_len(rbac));
        self.rail_idx = if rl == 0 {
            0
        } else {
            self.rail_idx.min(rl - 1)
        };
        self.matrix_idx = if ml == 0 {
            0
        } else {
            self.matrix_idx.min(ml - 1)
        };
    }

    /// Returns `true` when either live `/` filtering or an inline prompt owns text input.
    #[must_use]
    pub fn has_active_input(&self) -> bool {
        self.filter_active || self.inline_input.is_some()
    }

    /// Handles bracketed/system paste only when an inline prompt or live filter is active.
    pub fn handle_paste(&mut self, raw: &str) -> bool {
        if let Some(p) = &mut self.inline_input {
            return p.input.paste_text(raw);
        }
        if self.filter_active {
            self.reset_matrix_cursor();
            return self.filter_input.paste_text(raw);
        }
        false
    }

    /// Processes a keyboard event for navigation, toggles, filtering, and inline prompts.
    pub fn handle_key(
        &mut self,
        key: KeyEvent,
        rbac: &RbacConfig,
    ) -> WidgetOutcome<Vec<RbacAction>> {
        if !is_key_press(key) {
            return WidgetOutcome::Ignored;
        }
        if self.inline_input.is_some() {
            return self.handle_inline_key(key, rbac);
        }
        if self.filter_active {
            return self.handle_filter_key(key);
        }
        match key.code {
            KeyCode::Esc => {
                if self.delete_confirm_role.take().is_some() {
                    return WidgetOutcome::Consumed;
                }
                if !self.filter_input.is_empty() {
                    self.filter_input.clear();
                    self.reset_matrix_cursor();
                    return WidgetOutcome::Consumed;
                }
                return WidgetOutcome::Close;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = if self.focus == RbacPaneFocus::Rail {
                    RbacPaneFocus::Matrix
                } else {
                    RbacPaneFocus::Rail
                };
                return WidgetOutcome::Consumed;
            }
            KeyCode::Left => {
                self.focus = RbacPaneFocus::Rail;
                return WidgetOutcome::Consumed;
            }
            KeyCode::Right => {
                self.focus = RbacPaneFocus::Matrix;
                return WidgetOutcome::Consumed;
            }
            KeyCode::Char(c @ ('1' | '2' | '3'))
                if !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if let Some(tab) = RbacSubTab::from_index((c as usize) - ('1' as usize)) {
                    self.switch_sub_tab(tab);
                }
                return WidgetOutcome::Consumed;
            }
            _ => {}
        }
        let out = match self.sub_tab {
            RbacSubTab::Roles => self.handle_roles_key(key, rbac),
            RbacSubTab::Actors => handle_actors_key(self, key, rbac),
            RbacSubTab::PlacesBases => handle_places_bases_key(self, key, rbac),
        };
        let (rl, ml) = (self.rail_len(rbac), self.matrix_len(rbac));
        self.rail_scroll = scroll_offset_for_row(
            self.rail_idx,
            self.rail_scroll,
            self.viewport_rail_rows.get(),
            rl,
        );
        self.matrix_scroll = scroll_offset_for_row(
            self.matrix_idx,
            self.matrix_scroll,
            self.viewport_matrix_rows.get(),
            ml,
        );
        out
    }

    /// Processes mouse clicks, vertical scroll wheel, and scrollbar drags via `hitmap`.
    pub fn handle_mouse(
        &mut self,
        mouse: MouseEvent,
        hitmap: &HitMap,
        rbac: &RbacConfig,
    ) -> WidgetOutcome<Vec<RbacAction>> {
        if is_horizontal_scroll(mouse.kind) {
            return WidgetOutcome::Ignored;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let d: isize = if matches!(mouse.kind, MouseEventKind::ScrollUp) {
                    -1
                } else {
                    1
                };
                let on_rail = matches!(
                    hitmap.hit_test(mouse.column, mouse.row),
                    Some(
                        HitTarget::RailRow(_)
                            | HitTarget::Scrollbar {
                                id: ScrollId::RbacRail,
                                ..
                            }
                    )
                );
                if on_rail {
                    self.rail_scroll = clamp_scroll_offset(
                        self.rail_scroll.saturating_add_signed(d),
                        self.rail_len(rbac),
                        self.viewport_rail_rows.get(),
                    );
                } else {
                    self.matrix_scroll = clamp_scroll_offset(
                        self.matrix_scroll.saturating_add_signed(d),
                        self.matrix_len(rbac),
                        self.viewport_matrix_rows.get(),
                    );
                }
                WidgetOutcome::Consumed
            }
            MouseEventKind::Down(MouseButton::Left) | MouseEventKind::Drag(MouseButton::Left)
                if hitmap.scrollbar_at(mouse.column, mouse.row).is_some() =>
            {
                if let Some((id, trk, tot, vp)) = hitmap.scrollbar_at(mouse.column, mouse.row) {
                    let off = offset_from_scrollbar_y(trk.y, trk.height, mouse.row, tot, vp);
                    if id == ScrollId::RbacRail {
                        self.rail_scroll = off;
                    } else if id == ScrollId::RbacList {
                        self.matrix_scroll = off;
                    }
                }
                WidgetOutcome::Consumed
            }
            MouseEventKind::Down(MouseButton::Left) => {
                match hitmap.hit_test(mouse.column, mouse.row).cloned() {
                    Some(HitTarget::Tab(i)) => {
                        if let Some(t) = RbacSubTab::from_index(i) {
                            self.switch_sub_tab(t);
                        }
                        WidgetOutcome::Consumed
                    }
                    Some(HitTarget::RailRow(i)) => self.handle_rail_click(i, rbac),
                    Some(HitTarget::ToggleRow(i) | HitTarget::ListRow(i)) => {
                        self.handle_matrix_click(i, rbac)
                    }
                    Some(HitTarget::FooterChip(i)) => self.handle_chip_click(i, rbac),
                    Some(HitTarget::Backdrop(_)) => WidgetOutcome::Close,
                    _ => WidgetOutcome::Ignored,
                }
            }
            _ => WidgetOutcome::Ignored,
        }
    }

    /// Renders the RBAC hub modal inside `area`, recording interactive regions into `hitmap`.
    pub fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        rbac: &RbacConfig,
        palette: &ThemePalette,
        hitmap: &mut HitMap,
    ) {
        let tab_bar = TabBar {
            items: vec![
                TabItem {
                    label: "Roles & Matrix".into(),
                    short_label: "Roles".into(),
                    enabled: true,
                    badge: Some(rbac.roles.len().to_string()),
                },
                TabItem {
                    label: "Actors".into(),
                    short_label: "Actors".into(),
                    enabled: true,
                    badge: None,
                },
                TabItem {
                    label: "Places & Bases".into(),
                    short_label: "Places".into(),
                    enabled: true,
                    badge: None,
                },
            ],
            active: self.sub_tab.index(),
        };
        let cfg = HubFrameConfig {
            overlay_id: OverlayId::Settings,
            title: format!("RBAC Access Control — role: {}", self.selected_role),
            max_width: 108,
            max_height: 32,
            width_pct: 92,
            height_pct: 88,
            tab_bar: Some(&tab_bar),
            rail_width: 28,
            inspector_height: 3,
        };
        let chips = self.footer_chips();
        let footer = if let Some(p) = &self.inline_input {
            FooterMode::InlineInput {
                prompt: &p.prompt,
                input: &p.input,
                hint: &p.hint,
            }
        } else if self.filter_active {
            FooterMode::InlineInput {
                prompt: "/ Filter: ",
                input: &self.filter_input,
                hint: "Enter apply  Esc clear",
            }
        } else {
            FooterMode::Chips(&chips)
        };
        let areas = render_hub_frame(area, buf, &cfg, footer, palette, hitmap);
        let data = match self.sub_tab {
            RbacSubTab::Roles => self.roles_pane_data(rbac, palette),
            RbacSubTab::Actors => actors_pane_data(self, rbac),
            RbacSubTab::PlacesBases => places_bases_pane_data(self, rbac),
        };
        render_panes_and_inspector(
            self,
            areas.rail,
            areas.main,
            areas.inspector,
            buf,
            &data,
            palette,
            hitmap,
        );
    }

    pub(super) fn reset_matrix_cursor(&mut self) {
        self.matrix_idx = 0;
        self.matrix_scroll = 0;
    }

    fn switch_sub_tab(&mut self, tab: RbacSubTab) {
        if self.sub_tab == tab {
            return;
        }
        self.sub_tab = tab;
        self.rail_idx = 1;
        self.rail_scroll = 0;
        self.reset_matrix_cursor();
        self.filter_active = false;
        self.inline_input = None;
        self.delete_confirm_role = None;
    }

    fn current_group(&self) -> Option<&CapabilityGroup> {
        (self.selected_domain_id != CUSTOM_DOMAIN_ID)
            .then(|| {
                self.catalog
                    .group(&self.selected_domain_id)
                    .or_else(|| self.catalog.groups.first())
            })
            .flatten()
    }

    fn role_rail_items(&self, rbac: &RbacConfig) -> Vec<(PaneRow, RoleRailEntry)> {
        let role_def = rbac.roles.get(&self.selected_role);
        let mut out = vec![(("ROLES".into(), false, false), RoleRailEntry::Heading)];
        for (name, def) in &rbac.roles {
            let act = name == &self.selected_role;
            let fb = if rbac.fallback_role.as_deref() == Some(name.as_str()) {
                " [fb]"
            } else {
                ""
            };
            out.push((
                (
                    format!(
                        "{}{name} ({}){fb}",
                        if act { "*" } else { " " },
                        def.base.as_str()
                    ),
                    true,
                    act,
                ),
                RoleRailEntry::Role(name.clone()),
            ));
        }
        out.push((("+ New role".into(), true, false), RoleRailEntry::AddRole));
        out.push((("DOMAINS".into(), false, false), RoleRailEntry::Heading));
        for g in &self.catalog.groups {
            let (on, tot) = self.catalog.enabled_count(role_def, g);
            let txt = format!(
                "{} {} [{on}/{tot}]",
                if g.running { "●" } else { "○" },
                g.label
            );
            out.push((
                (txt, true, self.selected_domain_id == g.id),
                RoleRailEntry::Domain(g.id.clone()),
            ));
        }
        let cnt = role_def.map_or(0, |r| r.allow.len() + r.deny.len());
        out.push((
            (
                format!("* Custom patterns [{cnt}]"),
                true,
                self.selected_domain_id == CUSTOM_DOMAIN_ID,
            ),
            RoleRailEntry::CustomPatterns,
        ));
        out
    }

    fn role_matrix_items(&self, rbac: &RbacConfig) -> Vec<(PaneRow, RoleMatrixRow)> {
        let role_def = rbac.roles.get(&self.selected_role);
        let mut out = Vec::new();
        if self.selected_domain_id == CUSTOM_DOMAIN_ID {
            if let Some(r) = role_def {
                for p in &r.allow {
                    out.push((
                        (format!("[✓] Allow: {p}"), true, true),
                        RoleMatrixRow::CustomAllow(p.clone()),
                    ));
                }
                for p in &r.deny {
                    out.push((
                        (format!("[!] Deny (legacy): {p}"), true, false),
                        RoleMatrixRow::CustomDeny(p.clone()),
                    ));
                }
            }
            out.push((
                ("+ Add pattern".into(), true, false),
                RoleMatrixRow::AddCustomPattern,
            ));
            return out;
        }
        let Some(grp) = self.current_group() else {
            return out;
        };
        let q = self.filter_input.as_str();
        if q.trim().is_empty() && !grp.capabilities.is_empty() {
            let st = role_def.map_or(ToggleState::Off, |rd| group_state(rd, grp));
            let (on, tot) = self.catalog.enabled_count(role_def, grp);
            out.push((
                (
                    format!("{} All in {} ({on}/{tot})", st.glyph(), grp.label),
                    true,
                    true,
                ),
                RoleMatrixRow::GroupMaster,
            ));
        }
        for c in &grp.capabilities {
            if matches_friendly_query(q, &c.display_name, &c.id) {
                let st = role_def.map_or(ToggleState::Off, |rd| capability_state(rd, c));
                out.push((
                    (
                        format!("{} {} [{}]", st.glyph(), c.inline_label(), c.risk_tier),
                        true,
                        st == ToggleState::On,
                    ),
                    RoleMatrixRow::Capability(c.clone()),
                ));
            }
        }
        out
    }

    fn rail_len(&self, rbac: &RbacConfig) -> usize {
        match self.sub_tab {
            RbacSubTab::Roles => self.role_rail_items(rbac).len(),
            RbacSubTab::Actors => actor_rail_items(rbac, &self.seen_actors).len(),
            RbacSubTab::PlacesBases => places_bases_rail_items(rbac).len(),
        }
    }

    fn matrix_len(&self, rbac: &RbacConfig) -> usize {
        match self.sub_tab {
            RbacSubTab::Roles => self.role_matrix_items(rbac).len(),
            RbacSubTab::Actors => rbac.roles.len(),
            RbacSubTab::PlacesBases => {
                place_base_rows(rbac, places_bases_rail_items(rbac).get(self.rail_idx)).len()
            }
        }
    }

    fn handle_filter_key(&mut self, key: KeyEvent) -> WidgetOutcome<Vec<RbacAction>> {
        match key.code {
            KeyCode::Enter => {
                self.filter_active = false;
                WidgetOutcome::Consumed
            }
            KeyCode::Esc => {
                self.filter_active = false;
                self.filter_input.clear();
                self.reset_matrix_cursor();
                WidgetOutcome::Consumed
            }
            _ => {
                if self.filter_input.handle_key(key) == WidgetOutcome::Ignored {
                    WidgetOutcome::Ignored
                } else {
                    self.reset_matrix_cursor();
                    WidgetOutcome::Consumed
                }
            }
        }
    }

    fn handle_inline_key(
        &mut self,
        key: KeyEvent,
        rbac: &RbacConfig,
    ) -> WidgetOutcome<Vec<RbacAction>> {
        if key.code == KeyCode::Esc {
            self.inline_input = None;
            return WidgetOutcome::Consumed;
        }
        if key.code != KeyCode::Enter {
            if let Some(p) = &mut self.inline_input {
                let _ = p.input.handle_key(key);
            }
            return WidgetOutcome::Consumed;
        }
        let Some(prompt) = self.inline_input.take() else {
            return WidgetOutcome::Consumed;
        };
        let text = prompt.input.as_str().trim().to_owned();
        if text.is_empty() {
            toast("Value must not be empty", true)
        } else {
            submit_prompt(self, prompt.kind, &text, rbac)
        }
    }

    fn handle_roles_key(
        &mut self,
        key: KeyEvent,
        rbac: &RbacConfig,
    ) -> WidgetOutcome<Vec<RbacAction>> {
        let rail = self.role_rail_items(rbac);
        let rows = self.role_matrix_items(rbac);
        if let Some(d) = nav_delta(key.code) {
            if self.focus == RbacPaneFocus::Rail {
                move_idx(&rail, &mut self.rail_idx, d, |(_, e)| {
                    !matches!(e, RoleRailEntry::Heading)
                });
            } else {
                self.matrix_idx = step_idx(self.matrix_idx, d, rows.len());
            }
            return WidgetOutcome::Consumed;
        }
        match key.code {
            KeyCode::Char('/') => {
                self.filter_active = true;
                WidgetOutcome::Consumed
            }
            KeyCode::Char('n') => {
                open_prompt(
                    self,
                    RbacPromptKind::AddRole,
                    "New role: ",
                    "a-z, 0-9, '_', '-'",
                );
                WidgetOutcome::Consumed
            }
            KeyCode::Char('b') => cycle_role_base(rbac, &self.selected_role),
            KeyCode::Char('a') if self.selected_domain_id == CUSTOM_DOMAIN_ID => {
                open_prompt(
                    self,
                    RbacPromptKind::AddCustomPattern,
                    "Allow pattern: ",
                    "e.g. chat.*",
                );
                WidgetOutcome::Consumed
            }
            KeyCode::Char('a') | KeyCode::Char('m') => {
                cycle_role_approval(rbac, &self.selected_role)
            }
            KeyCode::Char('l') => {
                open_prompt(
                    self,
                    RbacPromptKind::EditRoleLabel(self.selected_role.clone()),
                    "Role label: ",
                    "Enter confirm",
                );
                WidgetOutcome::Consumed
            }
            KeyCode::Char(' ') | KeyCode::Enter if self.focus == RbacPaneFocus::Rail => {
                self.activate_role_rail(self.rail_idx, rbac)
            }
            KeyCode::Char(' ') | KeyCode::Enter => self.activate_role_matrix(self.matrix_idx, rbac),
            KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
                if self.focus == RbacPaneFocus::Matrix
                    && self.selected_domain_id == CUSTOM_DOMAIN_ID
                {
                    self.delete_custom_row(self.matrix_idx, rbac)
                } else {
                    delete_role_with_confirm(
                        rbac,
                        &self.selected_role,
                        &mut self.delete_confirm_role,
                    )
                }
            }
            _ => WidgetOutcome::Ignored,
        }
    }

    fn activate_role_rail(
        &mut self,
        idx: usize,
        rbac: &RbacConfig,
    ) -> WidgetOutcome<Vec<RbacAction>> {
        match self.role_rail_items(rbac).get(idx).map(|(_, e)| e) {
            Some(RoleRailEntry::Role(n)) => {
                self.selected_role = n.clone();
                self.delete_confirm_role = None;
                self.reset_matrix_cursor();
            }
            Some(RoleRailEntry::AddRole) => open_prompt(
                self,
                RbacPromptKind::AddRole,
                "New role: ",
                "a-z, 0-9, '_', '-'",
            ),
            Some(RoleRailEntry::Domain(id)) => {
                self.selected_domain_id = id.clone();
                self.reset_matrix_cursor();
                self.focus = RbacPaneFocus::Matrix;
            }
            Some(RoleRailEntry::CustomPatterns) => {
                self.selected_domain_id = CUSTOM_DOMAIN_ID.into();
                self.reset_matrix_cursor();
                self.focus = RbacPaneFocus::Matrix;
            }
            _ => {}
        }
        WidgetOutcome::Consumed
    }

    fn activate_role_matrix(
        &mut self,
        idx: usize,
        rbac: &RbacConfig,
    ) -> WidgetOutcome<Vec<RbacAction>> {
        let items = self.role_matrix_items(rbac);
        let (Some((_, row)), Some(role)) = (items.get(idx), rbac.roles.get(&self.selected_role))
        else {
            return WidgetOutcome::Consumed;
        };
        match row {
            RoleMatrixRow::GroupMaster => {
                let Some(grp) = self.current_group() else {
                    return WidgetOutcome::Consumed;
                };
                if group_state(role, grp) == ToggleState::LockedOwner {
                    return toast("Group has only owner-only capabilities", true);
                }
                let (_, _, patch) =
                    toggle_group_patch_with_catalog(&self.selected_role, role, grp, &self.catalog);
                apply(
                    patch,
                    format!("Updated '{}' for '{}'", grp.label, self.selected_role),
                )
            }
            RoleMatrixRow::Capability(c) if c.owner_only => toast(
                format!("'{}' is owner-only on '{}'", c.id, self.selected_role),
                true,
            ),
            RoleMatrixRow::Capability(c) => {
                let Some(grp) = self.current_group() else {
                    return WidgetOutcome::Consumed;
                };
                let (_, _, patch) = toggle_capability_patch_with_catalog(
                    &self.selected_role,
                    role,
                    c,
                    grp,
                    &self.catalog,
                );
                let v = if capability_state(role, c) == ToggleState::Off {
                    "Enabled"
                } else {
                    "Disabled"
                };
                apply(
                    patch,
                    format!("{v} '{}' on '{}'", c.display_name, self.selected_role),
                )
            }
            RoleMatrixRow::CustomAllow(_) | RoleMatrixRow::CustomDeny(_) => {
                self.delete_custom_row(idx, rbac)
            }
            RoleMatrixRow::AddCustomPattern => {
                open_prompt(
                    self,
                    RbacPromptKind::AddCustomPattern,
                    "Allow pattern: ",
                    "e.g. chat.*",
                );
                WidgetOutcome::Consumed
            }
        }
    }

    fn delete_custom_row(&self, idx: usize, rbac: &RbacConfig) -> WidgetOutcome<Vec<RbacAction>> {
        let Some(role) = rbac.roles.get(&self.selected_role) else {
            return WidgetOutcome::Consumed;
        };
        match self.role_matrix_items(rbac).get(idx).map(|(_, r)| r) {
            Some(RoleMatrixRow::CustomAllow(p)) => apply(
                json!({ "roles": { &self.selected_role: { "allow": role.allow.iter().filter(|x| *x != p).cloned().collect::<Vec<_>>() } } }),
                format!("Removed allow '{p}'"),
            ),
            Some(RoleMatrixRow::CustomDeny(p)) => apply(
                json!({ "roles": { &self.selected_role: { "deny": role.deny.iter().filter(|x| *x != p).cloned().collect::<Vec<_>>() } } }),
                format!("Removed deny '{p}'"),
            ),
            _ => WidgetOutcome::Consumed,
        }
    }

    fn handle_rail_click(
        &mut self,
        idx: usize,
        rbac: &RbacConfig,
    ) -> WidgetOutcome<Vec<RbacAction>> {
        self.focus = RbacPaneFocus::Rail;
        self.rail_idx = idx;
        match self.sub_tab {
            RbacSubTab::Roles => self.activate_role_rail(idx, rbac),
            RbacSubTab::Actors => {
                if matches!(
                    actor_rail_items(rbac, &self.seen_actors).get(idx),
                    Some(ActorRailItem::AddActor)
                ) {
                    open_prompt(
                        self,
                        RbacPromptKind::AddActor,
                        "Add actor ID: ",
                        "Enter confirm",
                    );
                } else {
                    self.reset_matrix_cursor();
                }
                WidgetOutcome::Consumed
            }
            RbacSubTab::PlacesBases => {
                match places_bases_rail_items(rbac).get(idx) {
                    Some(PlaceBaseRailItem::AddPlace) => open_prompt(
                        self,
                        RbacPromptKind::AddPlace,
                        "Place ID: ",
                        "e.g. telegram:chat_1",
                    ),
                    Some(PlaceBaseRailItem::AddBase) => open_prompt(
                        self,
                        RbacPromptKind::AddPermissionBase,
                        "Capability ID: ",
                        "e.g. chat.speak",
                    ),
                    _ => self.reset_matrix_cursor(),
                }
                WidgetOutcome::Consumed
            }
        }
    }

    fn handle_matrix_click(
        &mut self,
        idx: usize,
        rbac: &RbacConfig,
    ) -> WidgetOutcome<Vec<RbacAction>> {
        self.focus = RbacPaneFocus::Matrix;
        self.matrix_idx = idx;
        match self.sub_tab {
            RbacSubTab::Roles => self.activate_role_matrix(idx, rbac),
            RbacSubTab::Actors => {
                let rail = actor_rail_items(rbac, &self.seen_actors);
                let roles: Vec<String> = rbac.roles.keys().cloned().collect();
                if let (Some(a), Some(r)) = (selected_actor(&rail, self.rail_idx), roles.get(idx)) {
                    toggle_actor_role(rbac, a, r)
                } else {
                    WidgetOutcome::Consumed
                }
            }
            RbacSubTab::PlacesBases => {
                let rail = places_bases_rail_items(rbac);
                let sel = rail.get(self.rail_idx);
                activate_place_base(self, sel, place_base_rows(rbac, sel).get(idx))
            }
        }
    }

    fn footer_chips(&self) -> Vec<FooterChip> {
        let mk = |l: &str, active: bool, danger: bool| FooterChip {
            label: l.into(),
            active,
            danger,
        };
        match self.sub_tab {
            RbacSubTab::Roles if self.selected_domain_id == CUSTOM_DOMAIN_ID => vec![
                mk("+ Pattern", false, false),
                mk("Del Pattern", false, true),
                mk("+ Role", false, false),
                mk("Close", false, false),
            ],
            RbacSubTab::Roles => vec![
                mk("Toggle", false, false),
                mk("+ Role", false, false),
                mk("Base", false, false),
                mk("Approval", false, false),
                mk("/ Filter", !self.filter_input.is_empty(), false),
                mk(
                    if self.delete_confirm_role.is_some() {
                        "Confirm Del"
                    } else {
                        "Del Role"
                    },
                    self.delete_confirm_role.is_some(),
                    true,
                ),
                mk("Close", false, false),
            ],
            RbacSubTab::Actors => vec![
                mk("Toggle Role", false, false),
                mk("+ Actor", false, false),
                mk("Close", false, false),
            ],
            RbacSubTab::PlacesBases => vec![
                mk("+ Place", false, false),
                mk("+ Base", false, false),
                mk("Cycle/Add", false, false),
                mk("Close", false, false),
            ],
        }
    }

    fn handle_chip_click(
        &mut self,
        idx: usize,
        rbac: &RbacConfig,
    ) -> WidgetOutcome<Vec<RbacAction>> {
        match (
            self.sub_tab,
            self.selected_domain_id == CUSTOM_DOMAIN_ID,
            idx,
        ) {
            (RbacSubTab::Roles, false, 0)
            | (RbacSubTab::Actors, _, 0)
            | (RbacSubTab::PlacesBases, _, 2) => self.handle_matrix_click(self.matrix_idx, rbac),
            (RbacSubTab::Roles, false, 1) | (RbacSubTab::Roles, true, 2) => {
                open_prompt(
                    self,
                    RbacPromptKind::AddRole,
                    "New role: ",
                    "a-z, 0-9, '_', '-'",
                );
                WidgetOutcome::Consumed
            }
            (RbacSubTab::Roles, false, 2) => cycle_role_base(rbac, &self.selected_role),
            (RbacSubTab::Roles, false, 3) => cycle_role_approval(rbac, &self.selected_role),
            (RbacSubTab::Roles, false, 4) => {
                self.filter_active = true;
                WidgetOutcome::Consumed
            }
            (RbacSubTab::Roles, false, 5) => {
                delete_role_with_confirm(rbac, &self.selected_role, &mut self.delete_confirm_role)
            }
            (RbacSubTab::Roles, false, 6)
            | (RbacSubTab::Roles, true, 3)
            | (RbacSubTab::Actors, _, 2)
            | (RbacSubTab::PlacesBases, _, 3) => WidgetOutcome::Close,
            (RbacSubTab::Roles, true, 0) => {
                open_prompt(
                    self,
                    RbacPromptKind::AddCustomPattern,
                    "Allow pattern: ",
                    "e.g. chat.*",
                );
                WidgetOutcome::Consumed
            }
            (RbacSubTab::Roles, true, 1) => self.delete_custom_row(self.matrix_idx, rbac),
            (RbacSubTab::Actors, _, 1) => {
                open_prompt(
                    self,
                    RbacPromptKind::AddActor,
                    "Add actor ID: ",
                    "Enter confirm",
                );
                WidgetOutcome::Consumed
            }
            (RbacSubTab::PlacesBases, _, 0) => {
                open_prompt(
                    self,
                    RbacPromptKind::AddPlace,
                    "Place ID: ",
                    "e.g. telegram:chat_1",
                );
                WidgetOutcome::Consumed
            }
            (RbacSubTab::PlacesBases, _, 1) => {
                open_prompt(
                    self,
                    RbacPromptKind::AddPermissionBase,
                    "Capability ID: ",
                    "e.g. chat.speak",
                );
                WidgetOutcome::Consumed
            }
            _ => WidgetOutcome::Ignored,
        }
    }

    fn roles_pane_data(&self, rbac: &RbacConfig, pal: &ThemePalette) -> PaneData {
        let role_def = rbac.roles.get(&self.selected_role);
        let rail: Vec<PaneRow> = self
            .role_rail_items(rbac)
            .into_iter()
            .map(|(r, _)| r)
            .collect();
        let items = self.role_matrix_items(rbac);
        let cap = match items.get(self.matrix_idx) {
            Some((_, RoleMatrixRow::Capability(c))) => Some(c.clone()),
            _ => None,
        };
        let matrix: Vec<PaneRow> = items.into_iter().map(|(r, _)| r).collect();
        let filt = if self.filter_input.is_empty() {
            String::new()
        } else {
            format!(" | Filter: {}", self.filter_input.as_str())
        };
        let header = Some(format!(
            "Role: {} | Base: {} | Approval: {}{filt}",
            self.selected_role,
            role_def.map_or("guest", |r| r.base.as_str()),
            approval_mode_label(role_def.and_then(|r| r.approval)),
        ));
        let v = evaluate_inspector(rbac, &self.selected_role, cap.as_ref());
        let insp1 = cap.as_ref().map_or_else(
            || format!("Inspector: role '{}'", self.selected_role),
            |c| format!("Inspector: {}  Risk: {}", c.inline_label(), c.risk_tier),
        );
        let insp_color = Some(if !v.allowed {
            pal.status_red
        } else if v.hold {
            pal.status_yellow
        } else {
            pal.status_green
        });
        PaneData {
            rail,
            matrix,
            header,
            insp1,
            insp2: format!("Verdict: {} — {}", v.status_label, v.detail),
            insp_color,
        }
    }
}

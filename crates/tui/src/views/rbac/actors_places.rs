//! Actors, Places, and Permission Bases sub-tabs rendered inside the RBAC `HubFrame`.

use std::collections::BTreeSet;

use crossterm::event::{KeyCode, KeyEvent};
use monoagens_protocol::{ActorId, PlaceId};
use monoagens_valve::rbac::{RbacConfig, RoleTier};
use serde_json::{Value, json};

use crate::core::input::TextInput;
use crate::core::labels::friendly_capability;
use crate::core::overlay::WidgetOutcome;
use crate::views::rbac::catalog::{PaneData, PaneRow, move_idx};
use crate::views::rbac::{
    RbacAction, RbacInlinePrompt, RbacPaneFocus, RbacPromptKind, RbacViewState,
};

/// Rail entry for the `Actors` sub-tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActorRailItem {
    Heading(&'static str),
    Actor(String),
    AddActor,
}

/// Rail entry for the `Places & Bases` sub-tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceBaseRailItem {
    Heading(&'static str),
    Place(String),
    AddPlace,
    Base(String),
    AddBase,
}

/// Matrix row for the `Places & Bases` sub-tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceBaseRow {
    PlaceAllow(String),
    PlaceDeny(String),
    AddPlaceAllow,
    AddPlaceDeny,
    BaseTier { cap: String, tier: Option<RoleTier> },
    DeleteItem,
}

/// Collects sorted, deduplicated actors from `rbac.actor_roles` and `seen_actors`.
#[must_use]
pub fn actor_rail_items(rbac: &RbacConfig, seen_actors: &[String]) -> Vec<ActorRailItem> {
    let mut set: BTreeSet<String> = rbac.actor_roles.keys().map(|a| a.as_str().into()).collect();
    for a in seen_actors {
        let t = a.trim();
        if !t.is_empty() {
            set.insert(t.into());
        }
    }
    let mut out = vec![ActorRailItem::Heading("ACTORS")];
    out.extend(set.into_iter().map(ActorRailItem::Actor));
    out.push(ActorRailItem::AddActor);
    out
}

/// Collects rail items for `Places` and `Permission Bases`.
#[must_use]
pub fn places_bases_rail_items(rbac: &RbacConfig) -> Vec<PlaceBaseRailItem> {
    let mut out = vec![PlaceBaseRailItem::Heading("PLACES")];
    for p in rbac.places.keys() {
        out.push(PlaceBaseRailItem::Place(p.as_str().into()));
    }
    out.push(PlaceBaseRailItem::AddPlace);
    out.push(PlaceBaseRailItem::Heading("PERMISSION BASES"));
    for c in rbac.permission_bases.keys() {
        out.push(PlaceBaseRailItem::Base(c.clone()));
    }
    out.push(PlaceBaseRailItem::AddBase);
    out
}

/// Builds a `person` JSON patch assigning `new_roles` to `target_actor`.
#[must_use]
pub fn build_actor_roles_patch(
    rbac: &RbacConfig,
    target_actor: &str,
    new_roles: &[String],
) -> Value {
    let mut entries = Vec::new();
    let mut seen = false;
    for (actor, roles) in &rbac.actor_roles {
        let rs = if actor.as_str() == target_actor {
            seen = true;
            new_roles
        } else {
            roles.as_slice()
        };
        if !rs.is_empty() {
            entries.push(json!({ "actor_id": actor.as_str(), "roles": rs }));
        }
    }
    if !seen && !new_roles.is_empty() {
        entries.push(json!({ "actor_id": target_actor, "roles": new_roles }));
    }
    if entries.is_empty() {
        json!({ "person": null })
    } else {
        json!({ "person": entries })
    }
}

/// Cycles a `[permission_bases]` tier (`owner-only -> guest -> member -> trusted -> admin -> owner`).
#[must_use]
pub const fn next_permission_base(current: Option<RoleTier>) -> Option<RoleTier> {
    match current {
        None => Some(RoleTier::Guest),
        Some(RoleTier::Guest) => Some(RoleTier::Member),
        Some(RoleTier::Member) => Some(RoleTier::Trusted),
        Some(RoleTier::Trusted) => Some(RoleTier::Admin),
        Some(RoleTier::Admin) => Some(RoleTier::Owner),
        Some(RoleTier::Owner) => None,
    }
}

/// Canonical TOML/JSON value for a `[permission_bases]` tier.
#[must_use]
pub const fn permission_base_str(tier: Option<RoleTier>) -> &'static str {
    match tier {
        None => "owner-only",
        Some(t) => t.as_str(),
    }
}

pub(super) fn apply(patch: Value, msg: impl Into<String>) -> WidgetOutcome<Vec<RbacAction>> {
    WidgetOutcome::Action(vec![RbacAction::ApplyPatch {
        patch,
        toast: msg.into(),
    }])
}

pub(super) fn toast(msg: impl Into<String>, error: bool) -> WidgetOutcome<Vec<RbacAction>> {
    WidgetOutcome::Action(vec![RbacAction::Toast {
        message: msg.into(),
        error,
    }])
}

pub(super) fn selected_actor(items: &[ActorRailItem], idx: usize) -> Option<&str> {
    if let Some(ActorRailItem::Actor(a)) = items.get(idx) {
        return Some(a.as_str());
    }
    items.iter().find_map(|it| match it {
        ActorRailItem::Actor(a) => Some(a.as_str()),
        _ => None,
    })
}

pub(super) fn place_base_rows(
    rbac: &RbacConfig,
    item: Option<&PlaceBaseRailItem>,
) -> Vec<PlaceBaseRow> {
    match item {
        Some(PlaceBaseRailItem::Place(id)) => {
            let mut rows = Vec::new();
            if let Some(r) = rbac.places.get(&PlaceId::new(id)) {
                rows.extend(r.allow.iter().cloned().map(PlaceBaseRow::PlaceAllow));
                rows.extend(r.deny.iter().cloned().map(PlaceBaseRow::PlaceDeny));
            }
            rows.extend([
                PlaceBaseRow::AddPlaceAllow,
                PlaceBaseRow::AddPlaceDeny,
                PlaceBaseRow::DeleteItem,
            ]);
            rows
        }
        Some(PlaceBaseRailItem::Base(c)) => vec![
            PlaceBaseRow::BaseTier {
                cap: c.clone(),
                tier: rbac.permission_bases.get(c).copied().flatten(),
            },
            PlaceBaseRow::DeleteItem,
        ],
        _ => Vec::new(),
    }
}

pub(super) fn handle_actors_key(
    st: &mut RbacViewState,
    key: KeyEvent,
    rbac: &RbacConfig,
) -> WidgetOutcome<Vec<RbacAction>> {
    let rail = actor_rail_items(rbac, &st.seen_actors);
    let roles: Vec<String> = rbac.roles.keys().cloned().collect();
    if let Some(d) = nav_delta(key.code) {
        if st.focus == RbacPaneFocus::Rail {
            move_idx(&rail, &mut st.rail_idx, d, |it| {
                !matches!(it, ActorRailItem::Heading(_))
            });
            st.reset_matrix_cursor();
        } else {
            st.matrix_idx = step_idx(st.matrix_idx, d, roles.len());
        }
        return WidgetOutcome::Consumed;
    }
    if key.code == KeyCode::Char('a') {
        open_prompt(
            st,
            RbacPromptKind::AddActor,
            "Add actor ID: ",
            "Enter confirm",
        );
        return WidgetOutcome::Consumed;
    }
    if !matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
        return WidgetOutcome::Ignored;
    }
    if st.focus == RbacPaneFocus::Rail {
        if matches!(rail.get(st.rail_idx), Some(ActorRailItem::AddActor)) {
            open_prompt(
                st,
                RbacPromptKind::AddActor,
                "Add actor ID: ",
                "Enter confirm",
            );
        } else {
            st.focus = RbacPaneFocus::Matrix;
        }
        return WidgetOutcome::Consumed;
    }
    if let (Some(a), Some(r)) = (selected_actor(&rail, st.rail_idx), roles.get(st.matrix_idx)) {
        return toggle_actor_role(rbac, a, r);
    }
    WidgetOutcome::Consumed
}

pub(super) fn toggle_actor_role(
    rbac: &RbacConfig,
    actor: &str,
    role: &str,
) -> WidgetOutcome<Vec<RbacAction>> {
    let mut cur = rbac
        .actor_roles
        .get(&ActorId::new(actor))
        .cloned()
        .unwrap_or_default();
    let add = if let Some(p) = cur.iter().position(|r| r == role) {
        cur.remove(p);
        false
    } else {
        cur.push(role.into());
        true
    };
    let verb = if add { "Assigned" } else { "Removed" };
    let prep = if add { "to" } else { "from" };
    apply(
        build_actor_roles_patch(rbac, actor, &cur),
        format!("{verb} '{role}' {prep} '{actor}'"),
    )
}

pub(super) fn handle_places_bases_key(
    st: &mut RbacViewState,
    key: KeyEvent,
    rbac: &RbacConfig,
) -> WidgetOutcome<Vec<RbacAction>> {
    let rail = places_bases_rail_items(rbac);
    let sel = rail.get(st.rail_idx);
    let rows = place_base_rows(rbac, sel);
    if let Some(d) = nav_delta(key.code) {
        if st.focus == RbacPaneFocus::Rail {
            move_idx(&rail, &mut st.rail_idx, d, |it| {
                !matches!(it, PlaceBaseRailItem::Heading(_))
            });
            st.reset_matrix_cursor();
        } else {
            st.matrix_idx = step_idx(st.matrix_idx, d, rows.len());
        }
        return WidgetOutcome::Consumed;
    }
    if st.focus == RbacPaneFocus::Rail {
        if !matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
            return WidgetOutcome::Ignored;
        }
        match sel {
            Some(PlaceBaseRailItem::AddPlace) => open_prompt(
                st,
                RbacPromptKind::AddPlace,
                "Place ID: ",
                "e.g. telegram:chat_1",
            ),
            Some(PlaceBaseRailItem::AddBase) => open_prompt(
                st,
                RbacPromptKind::AddPermissionBase,
                "Capability ID: ",
                "e.g. chat.speak",
            ),
            _ => st.focus = RbacPaneFocus::Matrix,
        }
        return WidgetOutcome::Consumed;
    }
    match key.code {
        KeyCode::Enter | KeyCode::Char(' ') => {
            activate_place_base(st, sel, rows.get(st.matrix_idx))
        }
        KeyCode::Char('a') => {
            if let Some(PlaceBaseRailItem::Place(p)) = sel {
                open_prompt(
                    st,
                    RbacPromptKind::SetPlaceAllow(p.clone()),
                    "Allow pattern: ",
                    "e.g. chat.*",
                );
            }
            WidgetOutcome::Consumed
        }
        KeyCode::Char('x') => {
            if let Some(PlaceBaseRailItem::Place(p)) = sel {
                open_prompt(
                    st,
                    RbacPromptKind::SetPlaceDeny(p.clone()),
                    "Deny pattern: ",
                    "e.g. fs.*",
                );
            }
            WidgetOutcome::Consumed
        }
        KeyCode::Char('d') | KeyCode::Delete | KeyCode::Backspace => {
            delete_place_base(rbac, sel, rows.get(st.matrix_idx))
        }
        _ => WidgetOutcome::Ignored,
    }
}

pub(super) fn activate_place_base(
    st: &mut RbacViewState,
    sel: Option<&PlaceBaseRailItem>,
    row: Option<&PlaceBaseRow>,
) -> WidgetOutcome<Vec<RbacAction>> {
    match (sel, row) {
        (Some(PlaceBaseRailItem::Place(p)), Some(PlaceBaseRow::AddPlaceAllow)) => {
            open_prompt(
                st,
                RbacPromptKind::SetPlaceAllow(p.clone()),
                "Allow pattern: ",
                "e.g. chat.*",
            );
            WidgetOutcome::Consumed
        }
        (Some(PlaceBaseRailItem::Place(p)), Some(PlaceBaseRow::AddPlaceDeny)) => {
            open_prompt(
                st,
                RbacPromptKind::SetPlaceDeny(p.clone()),
                "Deny pattern: ",
                "e.g. fs.*",
            );
            WidgetOutcome::Consumed
        }
        (_, Some(PlaceBaseRow::BaseTier { cap, tier })) => {
            let lbl = permission_base_str(next_permission_base(*tier));
            apply(
                json!({ "permission_bases": { cap: lbl } }),
                format!("Set '{cap}' base to '{lbl}'"),
            )
        }
        (_, Some(PlaceBaseRow::DeleteItem)) => match sel {
            Some(PlaceBaseRailItem::Place(p)) => apply(
                json!({ "places": { p: null } }),
                format!("Deleted place '{p}'"),
            ),
            Some(PlaceBaseRailItem::Base(c)) => apply(
                json!({ "permission_bases": { c: null } }),
                format!("Removed base '{c}'"),
            ),
            _ => WidgetOutcome::Consumed,
        },
        _ => WidgetOutcome::Consumed,
    }
}

fn delete_place_base(
    rbac: &RbacConfig,
    sel: Option<&PlaceBaseRailItem>,
    row: Option<&PlaceBaseRow>,
) -> WidgetOutcome<Vec<RbacAction>> {
    if let Some(PlaceBaseRailItem::Place(p)) = sel
        && let Some(r) = rbac.places.get(&PlaceId::new(p))
    {
        let (mut allow, mut deny) = (r.allow.clone(), r.deny.clone());
        match row {
            Some(PlaceBaseRow::PlaceAllow(pat)) => {
                allow.retain(|x| x != pat);
                return apply(
                    json!({ "places": { p: { "allow": allow, "deny": deny } } }),
                    format!("Removed '{pat}'"),
                );
            }
            Some(PlaceBaseRow::PlaceDeny(pat)) => {
                deny.retain(|x| x != pat);
                return apply(
                    json!({ "places": { p: { "allow": allow, "deny": deny } } }),
                    format!("Removed '{pat}'"),
                );
            }
            _ => {}
        }
    }
    match sel {
        Some(PlaceBaseRailItem::Place(p)) => apply(
            json!({ "places": { p: null } }),
            format!("Deleted place '{p}'"),
        ),
        Some(PlaceBaseRailItem::Base(c)) => apply(
            json!({ "permission_bases": { c: null } }),
            format!("Removed base '{c}'"),
        ),
        _ => WidgetOutcome::Consumed,
    }
}

pub(super) fn submit_prompt(
    st: &mut RbacViewState,
    kind: RbacPromptKind,
    text: &str,
    rbac: &RbacConfig,
) -> WidgetOutcome<Vec<RbacAction>> {
    match kind {
        RbacPromptKind::AddRole
            if !text
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) =>
        {
            toast("Role name must use letters, digits, '_' or '-'", true)
        }
        RbacPromptKind::AddRole => {
            st.selected_role = text.into();
            apply(
                json!({ "roles": { text: { "base": "guest", "allow": [], "deny": [] } } }),
                format!("Created role '{text}'"),
            )
        }
        RbacPromptKind::EditRoleLabel(role) => apply(
            json!({ "roles": { role.clone(): { "label": text } } }),
            format!("Updated '{role}' label"),
        ),
        RbacPromptKind::AddCustomPattern => {
            let mut allow = rbac
                .roles
                .get(&st.selected_role)
                .map_or_else(Vec::new, |r| r.allow.clone());
            if !allow.iter().any(|p| p == text) {
                allow.push(text.into());
            }
            apply(
                json!({ "roles": { st.selected_role.clone(): { "allow": allow } } }),
                format!("Added '{text}'"),
            )
        }
        RbacPromptKind::AddActor => {
            if !st.seen_actors.iter().any(|a| a == text) {
                st.seen_actors.push(text.into());
            }
            let fb = rbac
                .fallback_role
                .clone()
                .or_else(|| rbac.roles.keys().next().cloned())
                .unwrap_or_else(|| "guest".into());
            apply(
                build_actor_roles_patch(rbac, text, std::slice::from_ref(&fb)),
                format!("Added actor '{text}' ({fb})"),
            )
        }
        RbacPromptKind::AddPlace => apply(
            json!({ "places": { text: { "allow": [], "deny": [] } } }),
            format!("Created place '{text}'"),
        ),
        RbacPromptKind::SetPlaceAllow(ref p) | RbacPromptKind::SetPlaceDeny(ref p) => {
            let cur = rbac.places.get(&PlaceId::new(p));
            let (mut a, mut d) = (
                cur.map_or_else(Vec::new, |r| r.allow.clone()),
                cur.map_or_else(Vec::new, |r| r.deny.clone()),
            );
            let tgt = if matches!(kind, RbacPromptKind::SetPlaceAllow(_)) {
                &mut a
            } else {
                &mut d
            };
            if !tgt.iter().any(|x| x == text) {
                tgt.push(text.into());
            }
            apply(
                json!({ "places": { p.clone(): { "allow": a, "deny": d } } }),
                format!("Updated place '{p}'"),
            )
        }
        RbacPromptKind::AddPermissionBase => apply(
            json!({ "permission_bases": { text: "owner-only" } }),
            format!("Added base '{text}'"),
        ),
    }
}

pub(super) fn actors_pane_data(st: &RbacViewState, rbac: &RbacConfig) -> PaneData {
    let rail_items = actor_rail_items(rbac, &st.seen_actors);
    let mut rail: Vec<PaneRow> = Vec::with_capacity(rail_items.len());
    for it in &rail_items {
        match it {
            ActorRailItem::Heading(h) => rail.push(((*h).into(), false, false)),
            ActorRailItem::Actor(a) => {
                let n = rbac.actor_roles.get(&ActorId::new(a)).map_or(0, Vec::len);
                rail.push((format!("{a} [{n}]"), true, false));
            }
            ActorRailItem::AddActor => rail.push(("+ Add actor".into(), true, true)),
        }
    }
    let actor = selected_actor(&rail_items, st.rail_idx);
    let assigned = actor
        .and_then(|a| rbac.actor_roles.get(&ActorId::new(a)))
        .cloned()
        .unwrap_or_default();
    let matrix = rbac
        .roles
        .iter()
        .map(|(r, d)| {
            let mk = if assigned.iter().any(|x| x == r) {
                "[✓]"
            } else {
                "[ ]"
            };
            (format!("{mk} {r} ({})", d.base.as_str()), true, false)
        })
        .collect();
    let insp1 = actor.map_or_else(|| "No actor selected".into(), |a| format!("Actor: {a}"));
    let insp2 = if assigned.is_empty() {
        format!(
            "Effective: fallback -> {}",
            rbac.fallback_role.as_deref().unwrap_or("(none)")
        )
    } else {
        format!("Assigned roles: {}", assigned.join(", "))
    };
    PaneData {
        rail,
        matrix,
        header: None,
        insp1,
        insp2,
        insp_color: None,
    }
}

pub(super) fn places_bases_pane_data(st: &RbacViewState, rbac: &RbacConfig) -> PaneData {
    let rail_items = places_bases_rail_items(rbac);
    let mut rail: Vec<PaneRow> = Vec::with_capacity(rail_items.len());
    for it in &rail_items {
        match it {
            PlaceBaseRailItem::Heading(h) => rail.push(((*h).into(), false, false)),
            PlaceBaseRailItem::Place(p) => rail.push((p.clone(), true, false)),
            PlaceBaseRailItem::AddPlace => rail.push(("+ Add place".into(), true, true)),
            PlaceBaseRailItem::Base(c) => rail.push((
                format!("{} ({c})", friendly_capability(c).label),
                true,
                false,
            )),
            PlaceBaseRailItem::AddBase => rail.push(("+ Add permission base".into(), true, true)),
        }
    }
    let sel = rail_items.get(st.rail_idx);
    let mut matrix: Vec<PaneRow> = Vec::new();
    for r in place_base_rows(rbac, sel) {
        let txt = match r {
            PlaceBaseRow::PlaceAllow(pat) => format!("[✓] Allow: {pat}"),
            PlaceBaseRow::PlaceDeny(pat) => format!("[!] Deny: {pat}"),
            PlaceBaseRow::AddPlaceAllow => "+ Add allow pattern".into(),
            PlaceBaseRow::AddPlaceDeny => "+ Add deny pattern".into(),
            PlaceBaseRow::BaseTier { cap, tier } => format!(
                "{} ({cap}) -> [{}]",
                friendly_capability(&cap).label,
                permission_base_str(tier)
            ),
            PlaceBaseRow::DeleteItem => "Delete selected item".into(),
        };
        matrix.push((txt, true, false));
    }
    let (insp1, insp2) = match sel {
        Some(PlaceBaseRailItem::Place(p)) => (
            format!("Place rule: {p}"),
            "Place allow/deny narrows non-owners before role checks".into(),
        ),
        Some(PlaceBaseRailItem::Base(c)) => (
            format!("Permission base: {c}"),
            "Space/Enter cycles tier; owner-only restricts to owner".into(),
        ),
        _ => (
            "Places & Permission Bases".into(),
            "Select a place or permission base from the rail".into(),
        ),
    };
    PaneData {
        rail,
        matrix,
        header: None,
        insp1,
        insp2,
        insp_color: None,
    }
}

pub(super) fn open_prompt(st: &mut RbacViewState, kind: RbacPromptKind, prompt: &str, hint: &str) {
    st.filter_active = false;
    st.inline_input = Some(RbacInlinePrompt {
        kind,
        input: TextInput::new(false),
        prompt: prompt.into(),
        hint: hint.into(),
    });
}

pub(super) const fn nav_delta(code: KeyCode) -> Option<isize> {
    match code {
        KeyCode::Up | KeyCode::Char('k') => Some(-1),
        KeyCode::Down | KeyCode::Char('j') => Some(1),
        _ => None,
    }
}

pub(super) fn step_idx(cur: usize, delta: isize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        cur.saturating_add_signed(delta).min(len - 1)
    }
}

//! RBAC editor: fallback role, roles (create / edit / delete with reference check), allow / deny
//! lists with a capability picker, actor role assignment, place rules and permission bases.
//!
//! Everything is written to the tables the gateway parser reads (`[roles]`, `[roles.<name>]`,
//! `[places."<id>"]`, `[permission_bases]` and the `roles` of `[[person]]` entries) with
//! `config.apply`; the valve stays deterministic and reloads them (the editor adds no
//! permission path of its own).

use monoagens_gateway::GatewayConfig;
use monoagens_valve::rbac::{ApprovalMode, RoleTier};
use serde_json::{Map, Value, json};

use crate::list_editor::ItemEditor;
use crate::selector::{SelectOption, Selector};
use crate::settings::{
    ConfirmAction, ConfirmState, InlineEditState, InlineEditTarget, SettingRow, SettingValueKind,
};
use crate::state::{AppAction, TuiAppState};

/// A sub-page of the RBAC tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RbacPage {
    Role(String),
    Actor(String),
    Place(String),
    Base(String),
}

/// Value of the "owner-only" entry of the permission base selector.
pub const OWNER_ONLY: &str = "owner-only";

/// Why role `name` cannot be deleted (where it is referenced), if it cannot.
#[must_use]
pub fn role_reference(cfg: &GatewayConfig, name: &str) -> Option<String> {
    let rbac = cfg.rbac.as_ref();
    if rbac.and_then(|r| r.fallback_role.as_deref()) == Some(name) {
        return Some(format!("'{name}' is the fallback role"));
    }
    let actors: Vec<&str> = cfg
        .persons
        .iter()
        .filter(|p| p.roles.iter().any(|r| r == name))
        .map(|p| p.actor_id.as_str())
        .collect();
    if actors.is_empty() {
        None
    } else {
        Some(format!("'{name}' is assigned to actor(s) {}", actors.join(", ")))
    }
}

/// The `person` array of `cfg` with actor `actor` holding `roles` (the entry is created when
/// missing and dropped when it ends up empty). `None` means "remove the whole key".
#[must_use]
pub fn persons_patch(cfg: &GatewayConfig, actor: &str, roles: &[String]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    let mut seen = false;
    for p in &cfg.persons {
        let is_target = p.actor_id.as_str() == actor;
        seen |= is_target;
        let rs: &[String] = if is_target { roles } else { &p.roles };
        if rs.is_empty() && p.accounts.is_empty() && !p.is_owner {
            continue; // an entry that says nothing
        }
        let mut o = Map::new();
        o.insert("actor_id".to_owned(), json!(p.actor_id.as_str()));
        if p.is_owner {
            o.insert("owner".to_owned(), json!(true));
        }
        if !p.accounts.is_empty() {
            o.insert(
                "accounts".to_owned(),
                json!(p
                    .accounts
                    .iter()
                    .map(|a| json!({ "body_id": a.body_id.as_str(), "account_id": a.account_id }))
                    .collect::<Vec<_>>()),
            );
        }
        if !rs.is_empty() {
            o.insert("roles".to_owned(), json!(rs));
        }
        out.push(Value::Object(o));
    }
    if !seen && !roles.is_empty() {
        out.push(json!({ "actor_id": actor, "roles": roles }));
    }
    if out.is_empty() {
        json!({ "person": null })
    } else {
        json!({ "person": out })
    }
}

fn valid_name(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("name must not be empty".to_owned());
    }
    if !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) {
        return Err("use letters, digits, '_' or '-' only".to_owned());
    }
    Ok(())
}

fn tier_options(with_owner_only: bool) -> Vec<SelectOption> {
    let mut v = vec![
        SelectOption::plain("guest", "Lowest tier: strangers"),
        SelectOption::plain("member", "Known people"),
        SelectOption::plain("trusted", "Close people"),
        SelectOption::plain("admin", "Can administer"),
        SelectOption::plain("owner", "Owner tier"),
    ];
    if with_owner_only {
        v.push(SelectOption::plain(OWNER_ONLY, "Only the owner may use it; nobody else, ever"));
    }
    v
}

fn role_settings_entries(role: &monoagens_valve::rbac::RoleDefinition) -> Vec<(String, String)> {
    role.settings
        .iter()
        .map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), str::to_owned)))
        .collect()
}

/// Parses a settings map value: JSON when it is one (numbers, booleans), else a string.
fn settings_value(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(|v| !v.is_object() && !v.is_null())
        .unwrap_or_else(|| json!(raw))
}

impl TuiAppState {
    /// Rows of the open RBAC sub-page.
    #[must_use]
    pub fn rbac_page_rows(&self) -> Vec<SettingRow> {
        use SettingValueKind as V;
        let Some(page) = &self.settings.rbac_page else {
            return Vec::new();
        };
        let row = |id: String, label: &str, desc: &str, value: V| SettingRow {
            id,
            section: "RBAC",
            label: label.to_owned(),
            description: desc.to_owned(),
            value,
        };
        let rbac = self.gateway_cfg.rbac.as_ref();
        match page {
            RbacPage::Role(n) => {
                let Some(role) = rbac.and_then(|r| r.roles.get(n)) else {
                    return Vec::new();
                };
                let p = |f: &str| format!("rbac.role.{n}.{f}");
                vec![
                    row(p("base"), "Base tier", "Tier the role grants; permission bases compare against it", V::Select(role.base.as_str().to_owned())),
                    row(p("approval"), "Approval mode", "yolo runs freely, default asks per risk, strict always asks; inherit uses the actor's other roles", V::Select(role.approval.map_or("inherit", |a| a.as_str()).to_owned())),
                    row(p("label"), "Label", "Display name of the role", V::Text(role.label.clone().unwrap_or_else(|| "(none)".to_owned()))),
                    row(p("context"), "Context", "Text added to the soul's context for actors with this role", V::Text(role.context.clone().unwrap_or_else(|| "(none)".to_owned()))),
                    row(p("allow"), "Allow", "Capability patterns the role may use (p picks from connected bodies)", V::List(role.allow.clone())),
                    row(p("deny"), "Deny", "Capability patterns the role may never use (deny wins over allow)", V::List(role.deny.clone())),
                    row(p("settings"), "Settings", "Free key/value settings of the role", V::Map(role_settings_entries(role))),
                    row(p("delete"), "[ Delete role ]", "Refused while the fallback role or an actor refers to it", V::Action("Delete".to_owned())),
                ]
            }
            RbacPage::Actor(a) => {
                let held: Vec<String> = self
                    .gateway_cfg
                    .persons
                    .iter()
                    .find(|p| p.actor_id.as_str() == a)
                    .map(|p| p.roles.clone())
                    .unwrap_or_default();
                let mut rows: Vec<SettingRow> = rbac
                    .map(|r| r.roles.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|r| {
                        row(format!("rbac.actor.{a}.role.{r}"), &r, "Space toggles this role for the actor", V::Bool(held.contains(&r)))
                    })
                    .collect();
                if rows.is_empty() {
                    rows.push(row(format!("rbac.actor.{a}.none"), "(no roles exist yet)", "Create a role first", V::ReadOnly(String::new())));
                }
                rows.push(row(format!("rbac.actor.{a}.remove"), "[ Remove all roles ]", "Drops the actor's role assignment", V::Action("Remove".to_owned())));
                rows
            }
            RbacPage::Place(pl) => {
                let rule = rbac.and_then(|r| r.places.get(&monoagens_protocol::PlaceId::new(pl)));
                vec![
                    row(format!("rbac.place.{pl}.allow"), "Allow", "When not empty, only these capability patterns work in this place", V::List(rule.map(|r| r.allow.clone()).unwrap_or_default())),
                    row(format!("rbac.place.{pl}.deny"), "Deny", "Capability patterns refused in this place", V::List(rule.map(|r| r.deny.clone()).unwrap_or_default())),
                    row(format!("rbac.place.{pl}.delete"), "[ Delete place rule ]", "Removes the rule of this place", V::Action("Delete".to_owned())),
                ]
            }
            RbacPage::Base(cap) => {
                let cur = rbac
                    .and_then(|r| r.permission_bases.get(cap))
                    .map_or(OWNER_ONLY.to_owned(), |t| t.map_or_else(|| OWNER_ONLY.to_owned(), |t| t.as_str().to_owned()));
                vec![
                    row(format!("rbac.base.{cap}.tier"), "Minimum tier", "Tier an actor needs for this capability, or owner-only", V::Select(cur)),
                    row(format!("rbac.base.{cap}.delete"), "[ Remove permission base ]", "Back to the default (no minimum)", V::Action("Remove".to_owned())),
                ]
            }
        }
    }

    fn rbac_open_page(&mut self, page: RbacPage) {
        if self.settings.rbac_page.is_none() {
            self.settings.page_return = Some((self.settings.selected_index, self.settings.scroll_offset));
        }
        self.settings.rbac_page = Some(page);
        self.settings.selected_index = 0;
        self.settings.scroll_offset = 0;
    }

    /// Leaves the RBAC sub-page and restores the list selection.
    pub(crate) fn rbac_close_page(&mut self) {
        self.settings.rbac_page = None;
        if let Some((sel, off)) = self.settings.page_return.take() {
            self.settings.selected_index = sel;
            self.settings.scroll_offset = off;
        }
    }

    /// Enter on a row of the RBAC tab's main list.
    pub(crate) fn rbac_activate_main(&mut self, id: &str, row: &SettingRow) -> Vec<AppAction> {
        if id == "rbac.fallback" {
            let mut opts = vec![SelectOption::plain("(none)", "No fallback: unbound actors get no role")];
            if let Some(r) = &self.gateway_cfg.rbac {
                opts.extend(r.roles.keys().map(|n| SelectOption::plain(n, "")));
            }
            let cur = match &row.value {
                SettingValueKind::Select(s) => s.clone(),
                _ => "(none)".to_owned(),
            };
            self.settings.selector = Some(Selector::new(id, "Fallback role", &row.description, opts, &cur));
        } else if id == "rbac.new_role" {
            self.prompt("rbac.new_role", "New role name");
        } else if id == "rbac.new_place" {
            self.prompt("rbac.new_place", "Place id (for example telegram:12345)");
        } else if id == "rbac.new_base" {
            self.prompt("rbac.new_base", "Capability id (for example tool.shell)");
        } else if id == "rbac.new_actor" {
            let mut opts: Vec<SelectOption> = self
                .settings
                .hints
                .actors
                .iter()
                .map(|a| SelectOption::plain(a, "seen in the event log"))
                .collect();
            opts.push(SelectOption::plain("__type__", "Type an actor id"));
            self.settings.selector = Some(Selector::new(id, "Actor", &row.description, opts, "__type__"));
        } else if let Some(n) = id.strip_prefix("rbac.role.") {
            self.rbac_open_page(RbacPage::Role(n.to_owned()));
        } else if let Some(a) = id.strip_prefix("rbac.actor.") {
            self.rbac_open_page(RbacPage::Actor(a.to_owned()));
        } else if let Some(p) = id.strip_prefix("rbac.place.") {
            self.rbac_open_page(RbacPage::Place(p.to_owned()));
        } else if let Some(c) = id.strip_prefix("rbac.base.") {
            self.rbac_open_page(RbacPage::Base(c.to_owned()));
        }
        Vec::new()
    }

    fn prompt(&mut self, id: &str, text: &str) {
        self.settings.editing = Some(InlineEditState {
            target: InlineEditTarget::SettingField { id: id.to_owned() },
            prompt: text.to_owned(),
            buffer: String::new(),
            masked: false,
        });
    }

    /// Enter / Space on a row of the open RBAC sub-page.
    pub(crate) fn rbac_activate_page(&mut self, id: &str, row: &SettingRow) -> Vec<AppAction> {
        let Some(page) = self.settings.rbac_page.clone() else {
            return Vec::new();
        };
        match page {
            RbacPage::Role(n) => {
                let Some(field) = id.rsplit('.').next() else { return Vec::new() };
                match field {
                    "base" => self.settings.selector = Some(Selector::new(id, "Base tier", &row.description, tier_options(false), &sel_text(&row.value))),
                    "approval" => {
                        let opts = vec![
                            SelectOption::plain("inherit", "Use the approval mode of the actor's other roles"),
                            SelectOption::plain("yolo", "Run without asking"),
                            SelectOption::plain("default", "Ask for risky actions"),
                            SelectOption::plain("strict", "Ask for every action, even for the owner"),
                        ];
                        self.settings.selector = Some(Selector::new(id, "Approval mode", &row.description, opts, &sel_text(&row.value)));
                    }
                    "label" | "context" => {
                        let cur = match &row.value {
                            SettingValueKind::Text(t) if t != "(none)" => t.clone(),
                            _ => String::new(),
                        };
                        self.settings.editing = Some(InlineEditState {
                            target: InlineEditTarget::SettingField { id: id.to_owned() },
                            prompt: format!("Edit {}", row.label),
                            buffer: cur,
                            masked: false,
                        });
                    }
                    "allow" | "deny" => {
                        if let SettingValueKind::List(v) = &row.value {
                            let mut ed = ItemEditor::list(id, &format!("Role {n}: {}", row.label), v);
                            ed.can_pick = true;
                            self.settings.items = Some(ed);
                        }
                    }
                    "settings" => {
                        if let SettingValueKind::Map(m) = &row.value {
                            self.settings.items_base = m.clone();
                            self.settings.items = Some(ItemEditor::map(id, &format!("Role {n}: settings"), m));
                        }
                    }
                    "delete" => {
                        if let Some(why) = role_reference(&self.gateway_cfg, &n) {
                            self.set_toast(format!("Cannot delete role: {why}"), true);
                        } else {
                            self.settings.confirm = Some(ConfirmState {
                                prompt: format!("Delete role '{n}'? (y/n)"),
                                action: ConfirmAction::DeleteRole(n),
                            });
                        }
                    }
                    _ => {}
                }
            }
            RbacPage::Actor(a) => {
                if id.ends_with(".remove") {
                    self.settings.confirm = Some(ConfirmState {
                        prompt: format!("Remove the roles of '{a}'? (y/n)"),
                        action: ConfirmAction::RemoveActor(a),
                    });
                } else if let Some((_, role)) = id.rsplit_once(".role.") {
                    let mut held: Vec<String> = self
                        .gateway_cfg
                        .persons
                        .iter()
                        .find(|p| p.actor_id.as_str() == a)
                        .map(|p| p.roles.clone())
                        .unwrap_or_default();
                    if let Some(pos) = held.iter().position(|r| r == role) {
                        held.remove(pos);
                    } else {
                        held.push(role.to_owned());
                    }
                    let patch = persons_patch(&self.gateway_cfg, &a, &held);
                    return self.submit_patch(patch, format!("Saved roles of '{a}'"));
                }
            }
            RbacPage::Place(p) => {
                if id.ends_with(".delete") {
                    self.settings.confirm = Some(ConfirmState {
                        prompt: format!("Delete the rule of place '{p}'? (y/n)"),
                        action: ConfirmAction::DeletePlace(p),
                    });
                } else if let SettingValueKind::List(v) = &row.value {
                    let mut ed = ItemEditor::list(id, &format!("Place {p}: {}", row.label), v);
                    ed.can_pick = true;
                    self.settings.items = Some(ed);
                }
            }
            RbacPage::Base(c) => {
                if id.ends_with(".delete") {
                    self.settings.confirm = Some(ConfirmState {
                        prompt: format!("Remove the permission base of '{c}'? (y/n)"),
                        action: ConfirmAction::DeleteBase(c),
                    });
                } else {
                    self.settings.selector = Some(Selector::new(id, "Minimum tier", &row.description, tier_options(true), &sel_text(&row.value)));
                }
            }
        }
        Vec::new()
    }

    /// A selector of the RBAC tab was answered.
    pub(crate) fn rbac_select_picked(&mut self, target: &str, value: &str) -> Vec<AppAction> {
        if target == "rbac.fallback" {
            let v = if value == "(none)" { Value::Null } else { json!(value) };
            return self.submit_patch(json!({ "roles": { "fallback": v } }), "Saved fallback role".to_owned());
        }
        if target == "rbac.new_actor" {
            if value == "__type__" {
                self.prompt("rbac.new_actor", "Actor id");
            } else {
                self.rbac_open_page(RbacPage::Actor(value.to_owned()));
            }
            return Vec::new();
        }
        if let Some(rest) = target.strip_prefix("pick.") {
            // Capability picker of an item editor: add the id to that list.
            if let Some(ed) = &mut self.settings.items
                && ed.target == rest
            {
                ed.push_unique(value);
                let vals = ed.values();
                let t = ed.target.clone();
                return self.rbac_items_saved(&t, Some(vals), None);
            }
            return Vec::new();
        }
        if let Some(rest) = target.strip_prefix("rbac.role.")
            && let Some((name, field)) = rest.rsplit_once('.')
        {
            let patch = match field {
                "base" => json!({ "roles": { name: { "base": value } } }),
                "approval" => {
                    let v = if value == "inherit" { Value::Null } else { json!(value) };
                    json!({ "roles": { name: { "approval": v } } })
                }
                _ => return Vec::new(),
            };
            return self.submit_patch(patch, format!("Saved role '{name}'"));
        }
        if let Some(rest) = target.strip_prefix("rbac.base.")
            && let Some((cap, "tier")) = rest.rsplit_once('.')
        {
            return self.submit_patch(
                json!({ "permission_bases": { cap: value } }),
                format!("Saved permission base of '{cap}'"),
            );
        }
        Vec::new()
    }

    /// A list or map editor of the RBAC pages saved (`values` for lists, `entries` for maps).
    pub(crate) fn rbac_items_saved(
        &mut self,
        target: &str,
        values: Option<Vec<String>>,
        entries: Option<crate::settings_ui::MapChange>,
    ) -> Vec<AppAction> {
        let Some(rest) = target.strip_prefix("rbac.") else {
            return Vec::new();
        };
        if let Some(r) = rest.strip_prefix("role.")
            && let Some((name, field)) = r.rsplit_once('.')
        {
            if let (Some(v), "allow" | "deny") = (values.clone(), field) {
                return self.submit_patch(json!({ "roles": { name: { field: v } } }), format!("Saved {field} of '{name}'"));
            }
            if let (Some((old, new)), "settings") = (entries, field) {
                let mut m = Map::new();
                for (k, _) in &old {
                    if !new.iter().any(|(nk, _)| nk == k) {
                        m.insert(k.clone(), Value::Null);
                    }
                }
                for (k, v) in &new {
                    m.insert(k.clone(), settings_value(v));
                }
                return self.submit_patch(json!({ "roles": { name: { "settings": m } } }), format!("Saved settings of '{name}'"));
            }
        }
        if let Some(r) = rest.strip_prefix("place.")
            && let Some((place, field @ ("allow" | "deny"))) = r.rsplit_once('.')
            && let Some(v) = values
        {
            // Both lists are written so the rule exists as a whole.
            let other = if field == "allow" { "deny" } else { "allow" };
            let other_vals: Vec<String> = self
                .gateway_cfg
                .rbac
                .as_ref()
                .and_then(|rb| rb.places.get(&monoagens_protocol::PlaceId::new(place)))
                .map(|rule| if other == "allow" { rule.allow.clone() } else { rule.deny.clone() })
                .unwrap_or_default();
            return self.submit_patch(
                json!({ "places": { place: { field: v, other: other_vals } } }),
                format!("Saved place rule '{place}'"),
            );
        }
        Vec::new()
    }

    /// An inline text edit of the RBAC tab was committed.
    pub(crate) fn rbac_text_committed(&mut self, id: &str, text: &str) -> Vec<AppAction> {
        let text = text.trim();
        match id {
            "rbac.new_role" => {
                if let Err(e) = valid_name(text) {
                    self.set_toast(e, true);
                } else if text == "fallback" {
                    self.set_toast("'fallback' is reserved", true);
                } else if self.gateway_cfg.rbac.as_ref().is_some_and(|r| r.roles.contains_key(text)) {
                    self.set_toast(format!("role '{text}' already exists"), true);
                } else {
                    let mut acts = self.submit_patch(json!({ "roles": { text: { "base": "guest" } } }), format!("Created role '{text}'"));
                    self.rbac_open_page(RbacPage::Role(text.to_owned()));
                    // The page lists the role once the write has been applied locally.
                    self.ensure_role_listed(text);
                    acts.shrink_to_fit();
                    return acts;
                }
            }
            "rbac.new_place" => {
                if text.is_empty() {
                    self.set_toast("place id must not be empty", true);
                } else {
                    let acts = self.submit_patch(
                        json!({ "places": { text: { "allow": [], "deny": [] } } }),
                        format!("Created place rule '{text}'"),
                    );
                    self.rbac_open_page(RbacPage::Place(text.to_owned()));
                    return acts;
                }
            }
            "rbac.new_base" => {
                if text.is_empty() {
                    self.set_toast("capability id must not be empty", true);
                } else {
                    let acts = self.submit_patch(
                        json!({ "permission_bases": { text: OWNER_ONLY } }),
                        format!("Set permission base of '{text}' (owner-only)"),
                    );
                    self.rbac_open_page(RbacPage::Base(text.to_owned()));
                    return acts;
                }
            }
            "rbac.new_actor" => {
                if text.is_empty() {
                    self.set_toast("actor id must not be empty", true);
                } else {
                    self.rbac_open_page(RbacPage::Actor(text.to_owned()));
                }
            }
            _ => {
                if let Some(rest) = id.strip_prefix("rbac.role.")
                    && let Some((name, field @ ("label" | "context"))) = rest.rsplit_once('.')
                {
                    let v = if text.is_empty() { Value::Null } else { json!(text) };
                    return self.submit_patch(json!({ "roles": { name: { field: v } } }), format!("Saved {field} of '{name}'"));
                }
            }
        }
        Vec::new()
    }

    /// The local copy of the config does not have a just-created role until the write came back
    /// (attached gateway): add it so the page can show it right away.
    fn ensure_role_listed(&mut self, name: &str) {
        let rbac = self.gateway_cfg.rbac.get_or_insert_with(Default::default);
        rbac.roles.entry(name.to_owned()).or_insert_with(|| monoagens_valve::rbac::RoleDefinition {
            name: name.to_owned(),
            base: RoleTier::Guest,
            allow: Vec::new(),
            deny: Vec::new(),
            approval: None::<ApprovalMode>,
            label: None,
            context: None,
            settings: Default::default(),
        });
    }

    /// A confirmed question.
    pub(crate) fn confirmed(&mut self, action: ConfirmAction) -> Vec<AppAction> {
        match action {
            ConfirmAction::RemovePlugin(pid) => {
                self.settings.plugin_detail = None;
                self.submit_patch(json!({ "plugins": { pid.clone(): null } }), format!("Removed plugin '{pid}'"))
            }
            ConfirmAction::DeleteRole(n) => {
                if let Some(why) = role_reference(&self.gateway_cfg, &n) {
                    self.set_toast(format!("Cannot delete role: {why}"), true);
                    return Vec::new();
                }
                self.rbac_close_page();
                self.submit_patch(json!({ "roles": { n.clone(): null } }), format!("Deleted role '{n}'"))
            }
            ConfirmAction::DeletePlace(p) => {
                self.rbac_close_page();
                self.submit_patch(json!({ "places": { p.clone(): null } }), format!("Deleted place rule '{p}'"))
            }
            ConfirmAction::DeleteBase(c) => {
                self.rbac_close_page();
                self.submit_patch(json!({ "permission_bases": { c.clone(): null } }), format!("Removed permission base of '{c}'"))
            }
            ConfirmAction::RemoveActor(a) => {
                self.rbac_close_page();
                let patch = persons_patch(&self.gateway_cfg, &a, &[]);
                self.submit_patch(patch, format!("Removed roles of '{a}'"))
            }
        }
    }

    /// Opens the capability picker for the item editor `target` (known ids plus free text).
    pub(crate) fn open_capability_picker(&mut self, target: &str, current: &[String]) {
        let opts: Vec<SelectOption> = self
            .settings
            .hints
            .capabilities
            .iter()
            .filter(|c| !current.contains(c))
            .map(|c| SelectOption::plain(c, "capability of a connected body"))
            .collect();
        if opts.is_empty() {
            self.set_toast("No connected body reported capabilities; press a to type a pattern", false);
            return;
        }
        self.settings.selector = Some(Selector::new(
            &format!("pick.{target}"),
            "Capability",
            "Pick a capability id (a adds free text, patterns like chat.* work)",
            opts,
            "",
        ));
    }
}

fn sel_text(v: &SettingValueKind) -> String {
    match v {
        SettingValueKind::Select(s) => s.clone(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use monoagens_gateway::parse_config_str;

    fn cfg(extra: &str) -> GatewayConfig {
        parse_config_str(
            &format!("version = 1\n{extra}"),
            std::path::Path::new("t.toml"),
            std::path::Path::new("s.toml"),
        )
        .expect("parse")
    }

    #[test]
    fn delete_is_refused_while_referenced() {
        let c = cfg("[roles]\nfallback = \"guest-plus\"\n[roles.guest-plus]\nbase = \"guest\"\n[roles.family]\nbase = \"trusted\"\n[[person]]\nactor_id = \"per_a\"\nroles = [\"family\"]\n");
        assert_eq!(role_reference(&c, "guest-plus").as_deref(), Some("'guest-plus' is the fallback role"));
        assert!(role_reference(&c, "family").unwrap().contains("per_a"));
        let c2 = cfg("[roles.free]\nbase = \"guest\"\n");
        assert!(role_reference(&c2, "free").is_none());
    }

    #[test]
    fn persons_patch_keeps_accounts_and_drops_empty_entries() {
        let c = cfg("[[person]]\nactor_id = \"per_a\"\nowner = true\naccounts = [{ body_id = \"tg\", account_id = \"1\" }]\n");
        let p = persons_patch(&c, "per_b", &["family".to_owned()]);
        let arr = p["person"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["accounts"][0]["body_id"], json!("tg"));
        assert_eq!(arr[1]["roles"], json!(["family"]));
        let none = persons_patch(&cfg(""), "per_x", &[]);
        assert_eq!(none, json!({ "person": null }));
    }

    #[test]
    fn written_config_parses_back_into_rbac_config() {
        let rbac = cfg(
            "[roles]\nfallback = \"guest\"\n[roles.family]\nbase = \"trusted\"\nallow = [\"chat.speak\"]\napproval = \"strict\"\n[places.\"tg:1\"]\nallow = [\"chat.*\"]\ndeny = []\n[permission_bases]\n\"tool.shell\" = \"owner-only\"\n\"chat.speak\" = \"member\"\n",
        )
        .rbac
        .expect("rbac");
        assert_eq!(rbac.fallback_role.as_deref(), Some("guest"));
        assert_eq!(rbac.roles["family"].base, RoleTier::Trusted);
        assert_eq!(rbac.roles["family"].approval, Some(ApprovalMode::Strict));
        assert_eq!(rbac.permission_bases["tool.shell"], None);
        assert_eq!(rbac.permission_bases["chat.speak"], Some(RoleTier::Member));
        assert_eq!(rbac.places.len(), 1);
    }
}

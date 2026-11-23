//! Simple ON/OFF toggle engine, wildcard-to-explicit materialization,
//! Valve-backed live Inspector evaluation, and `toml_edit` patch application.

use std::collections::BTreeSet;

use monoagens_protocol::ActorId;
use monoagens_valve::rbac::{
    ApprovalMode, RbacConfig, RbacDecision, RoleDefinition, RoleTier, matches_pattern,
};
use serde_json::{Value, json};
use toml_edit::{
    Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value as TomlVal,
};

use crate::core::overlay::WidgetOutcome;
use crate::views::rbac::actors_places::{apply, toast};
use crate::views::rbac::catalog::{CapabilityCatalog, CapabilityEntry, CapabilityGroup};
use crate::views::rbac::{RbacAction, ToggleState};

/// Sentinel domain ID for the `Custom patterns` view in the left rail.
pub const CUSTOM_DOMAIN_ID: &str = "_custom";

/// Deterministic verdict computed via [`RbacConfig::evaluate`] for the bottom Inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectorVerdict {
    pub allowed: bool,
    pub hold: bool,
    pub status_label: &'static str,
    pub detail: String,
}

/// Computes the toggle state of `cap` for `role`.
#[must_use]
pub fn capability_state(role: &RoleDefinition, cap: &CapabilityEntry) -> ToggleState {
    if cap.owner_only {
        return ToggleState::LockedOwner;
    }
    if role.deny.iter().any(|p| matches_pattern(p, &cap.id)) {
        ToggleState::Off
    } else if role.allow.iter().any(|p| matches_pattern(p, &cap.id)) {
        ToggleState::On
    } else {
        ToggleState::Off
    }
}

/// Computes the aggregate master toggle state of `group` for `role`.
#[must_use]
pub fn group_state(role: &RoleDefinition, group: &CapabilityGroup) -> ToggleState {
    let tog: Vec<&CapabilityEntry> = group
        .capabilities
        .iter()
        .filter(|c| !c.owner_only)
        .collect();
    if tog.is_empty() {
        return if group.capabilities.iter().any(|c| c.owner_only) {
            ToggleState::LockedOwner
        } else {
            ToggleState::Off
        };
    }
    let on = tog
        .iter()
        .filter(|c| capability_state(role, c) == ToggleState::On)
        .count();
    if on == 0 {
        ToggleState::Off
    } else if on == tog.len() {
        ToggleState::On
    } else {
        ToggleState::Partial
    }
}

/// Toggles a single capability on `role` and returns `(new_allow, new_deny, config_patch)`.
#[must_use]
pub fn toggle_capability_patch(
    role_name: &str,
    role: &RoleDefinition,
    cap: &CapabilityEntry,
    group: &CapabilityGroup,
) -> (Vec<String>, Vec<String>, Value) {
    toggle_cap_in(role_name, role, cap, std::slice::from_ref(group))
}

/// Catalog-aware variant of [`toggle_capability_patch`] for cross-group wildcards like `"*"`.
#[must_use]
pub fn toggle_capability_patch_with_catalog(
    role_name: &str,
    role: &RoleDefinition,
    cap: &CapabilityEntry,
    group: &CapabilityGroup,
    catalog: &CapabilityCatalog,
) -> (Vec<String>, Vec<String>, Value) {
    let groups = if catalog.groups.is_empty() {
        std::slice::from_ref(group)
    } else {
        &catalog.groups
    };
    toggle_cap_in(role_name, role, cap, groups)
}

fn toggle_cap_in(
    role_name: &str,
    role: &RoleDefinition,
    cap: &CapabilityEntry,
    groups: &[CapabilityGroup],
) -> (Vec<String>, Vec<String>, Value) {
    if cap.owner_only {
        return (role.allow.clone(), role.deny.clone(), json!({}));
    }
    let (allow, deny) = match capability_state(role, cap) {
        ToggleState::Off => {
            let mut set: BTreeSet<String> = role.allow.iter().cloned().collect();
            set.insert(cap.id.clone());
            let d = role
                .deny
                .iter()
                .filter(|p| p.as_str() != cap.id)
                .cloned()
                .collect();
            (set.into_iter().collect(), d)
        }
        ToggleState::On => {
            let mut set = BTreeSet::new();
            for pat in &role.allow {
                if pat == &cap.id {
                    continue;
                }
                if is_wildcard(pat) && matches_pattern(pat, &cap.id) {
                    materialize_wildcard(&mut set, pat, groups, None, Some(&cap.id), &role.deny);
                } else {
                    set.insert(pat.clone());
                }
            }
            (set.into_iter().collect(), role.deny.clone())
        }
        ToggleState::Partial | ToggleState::LockedOwner => {
            return (role.allow.clone(), role.deny.clone(), json!({}));
        }
    };
    let patch = json!({ "roles": { role_name: { "allow": allow, "deny": deny } } });
    (allow, deny, patch)
}

/// Toggles every non-`owner_only` capability in `group` on `role`.
#[must_use]
pub fn toggle_group_patch(
    role_name: &str,
    role: &RoleDefinition,
    group: &CapabilityGroup,
) -> (Vec<String>, Vec<String>, Value) {
    toggle_group_in(role_name, role, group, std::slice::from_ref(group))
}

/// Catalog-aware variant of [`toggle_group_patch`] preserving unrelated groups if `"*"` is split.
#[must_use]
pub fn toggle_group_patch_with_catalog(
    role_name: &str,
    role: &RoleDefinition,
    group: &CapabilityGroup,
    catalog: &CapabilityCatalog,
) -> (Vec<String>, Vec<String>, Value) {
    let groups = if catalog.groups.is_empty() {
        std::slice::from_ref(group)
    } else {
        &catalog.groups
    };
    toggle_group_in(role_name, role, group, groups)
}

fn toggle_group_in(
    role_name: &str,
    role: &RoleDefinition,
    group: &CapabilityGroup,
    all: &[CapabilityGroup],
) -> (Vec<String>, Vec<String>, Value) {
    let state = group_state(role, group);
    let tog: Vec<&CapabilityEntry> = group
        .capabilities
        .iter()
        .filter(|c| !c.owner_only)
        .collect();
    if state == ToggleState::LockedOwner || tog.is_empty() {
        return (role.allow.clone(), role.deny.clone(), json!({}));
    }
    let (allow, deny) = if state == ToggleState::On {
        let gids: BTreeSet<&str> = group.capabilities.iter().map(|c| c.id.as_str()).collect();
        let mut set = BTreeSet::new();
        for pat in &role.allow {
            if gids.contains(pat.as_str()) {
                continue;
            }
            if is_wildcard(pat)
                && group
                    .capabilities
                    .iter()
                    .any(|c| matches_pattern(pat, &c.id))
            {
                materialize_wildcard(&mut set, pat, all, Some(&group.id), None, &role.deny);
            } else {
                set.insert(pat.clone());
            }
        }
        (set.into_iter().collect(), role.deny.clone())
    } else {
        let ids: BTreeSet<&str> = tog.iter().map(|c| c.id.as_str()).collect();
        let d = role
            .deny
            .iter()
            .filter(|p| !ids.contains(p.as_str()))
            .cloned()
            .collect();
        let has_owner = group.capabilities.iter().any(|c| c.owner_only);
        let mut set: BTreeSet<String> = role.allow.iter().cloned().collect();
        if let Some(pfx) = single_group_prefix(&group.capabilities)
            && !has_owner
            && tog.len() > 1
        {
            let wc = format!("{pfx}.*");
            set.retain(|p| !matches_pattern(&wc, p) || p == &wc);
            set.insert(wc);
        } else {
            set.extend(tog.into_iter().map(|c| c.id.clone()));
        }
        (set.into_iter().collect(), d)
    };
    let patch = json!({ "roles": { role_name: { "allow": allow, "deny": deny } } });
    (allow, deny, patch)
}

fn materialize_wildcard(
    set: &mut BTreeSet<String>,
    pat: &str,
    groups: &[CapabilityGroup],
    ex_grp: Option<&str>,
    ex_cap: Option<&str>,
    deny: &[String],
) {
    for grp in groups {
        if ex_grp == Some(grp.id.as_str()) {
            continue;
        }
        for c in &grp.capabilities {
            if !c.owner_only
                && ex_cap != Some(c.id.as_str())
                && matches_pattern(pat, &c.id)
                && !deny.iter().any(|d| matches_pattern(d, &c.id))
            {
                set.insert(c.id.clone());
            }
        }
    }
}

fn is_wildcard(pat: &str) -> bool {
    pat == "*" || pat.ends_with('*')
}

fn single_group_prefix(caps: &[CapabilityEntry]) -> Option<&str> {
    let mut prefix: Option<&str> = None;
    for cap in caps {
        let (head, _) = cap.id.split_once('.')?;
        if head.is_empty() || matches!(prefix, Some(p) if p != head) {
            return None;
        }
        prefix = Some(head);
    }
    prefix
}

/// Checks whether `role_name` is referenced as the fallback role or assigned to any actor.
#[must_use]
pub fn role_reference_reason(rbac: &RbacConfig, role_name: &str) -> Option<String> {
    if rbac.fallback_role.as_deref() == Some(role_name) {
        return Some(format!("'{role_name}' is the fallback role"));
    }
    let actors: Vec<&str> = rbac
        .actor_roles
        .iter()
        .filter(|(_, rs)| rs.iter().any(|r| r == role_name))
        .map(|(a, _)| a.as_str())
        .collect();
    (!actors.is_empty())
        .then(|| format!("'{role_name}' is used by actor(s): {}", actors.join(", ")))
}

/// Advances a role's [`RoleTier`] cyclically (`Guest -> Member -> Trusted -> Admin -> Owner -> Guest`).
#[must_use]
pub const fn next_base_tier(current: RoleTier) -> RoleTier {
    match current {
        RoleTier::Guest => RoleTier::Member,
        RoleTier::Member => RoleTier::Trusted,
        RoleTier::Trusted => RoleTier::Admin,
        RoleTier::Admin => RoleTier::Owner,
        RoleTier::Owner => RoleTier::Guest,
    }
}

/// Advances an optional [`ApprovalMode`] cyclically (`Default -> Off(Yolo) -> Strict -> Default`).
#[must_use]
pub const fn next_approval_mode(current: Option<ApprovalMode>) -> Option<ApprovalMode> {
    match current {
        None | Some(ApprovalMode::Default) => Some(ApprovalMode::Yolo),
        Some(ApprovalMode::Yolo) => Some(ApprovalMode::Strict),
        Some(ApprovalMode::Strict) => None,
    }
}

/// Human-readable label for an optional [`ApprovalMode`].
#[must_use]
pub const fn approval_mode_label(mode: Option<ApprovalMode>) -> &'static str {
    match mode {
        None | Some(ApprovalMode::Default) => "default",
        Some(ApprovalMode::Yolo) => "off",
        Some(ApprovalMode::Strict) => "strict",
    }
}

pub(super) fn cycle_role_base(
    rbac: &RbacConfig,
    role_name: &str,
) -> WidgetOutcome<Vec<RbacAction>> {
    let Some(role) = rbac.roles.get(role_name) else {
        return WidgetOutcome::Consumed;
    };
    let next = next_base_tier(role.base);
    apply(
        json!({ "roles": { role_name: { "base": next.as_str() } } }),
        format!("Set '{role_name}' base to '{}'", next.as_str()),
    )
}

pub(super) fn cycle_role_approval(
    rbac: &RbacConfig,
    role_name: &str,
) -> WidgetOutcome<Vec<RbacAction>> {
    let Some(role) = rbac.roles.get(role_name) else {
        return WidgetOutcome::Consumed;
    };
    let next = next_approval_mode(role.approval);
    let val = next.map_or(Value::Null, |m| Value::String(m.as_str().into()));
    apply(
        json!({ "roles": { role_name: { "approval": val } } }),
        format!(
            "Set '{role_name}' approval to '{}'",
            approval_mode_label(next)
        ),
    )
}

pub(super) fn delete_role_with_confirm(
    rbac: &RbacConfig,
    role_name: &str,
    confirm: &mut Option<String>,
) -> WidgetOutcome<Vec<RbacAction>> {
    if let Some(reason) = role_reference_reason(rbac, role_name) {
        *confirm = None;
        return toast(format!("Cannot delete: {reason}"), true);
    }
    if confirm.as_deref() == Some(role_name) {
        *confirm = None;
        return apply(
            json!({ "roles": { role_name: null } }),
            format!("Deleted role '{role_name}'"),
        );
    }
    *confirm = Some(role_name.into());
    toast(format!("Press d again to delete '{role_name}'"), false)
}

fn verdict(allowed: bool, hold: bool, status_label: &'static str, detail: String) -> InspectorVerdict {
    InspectorVerdict { allowed, hold, status_label, detail }
}

/// Evaluates `cap` for `role_name` by calling [`RbacConfig::evaluate`] directly.
#[must_use]
pub fn evaluate_inspector(
    rbac: &RbacConfig,
    role_name: &str,
    cap: Option<&CapabilityEntry>,
) -> InspectorVerdict {
    let Some(cap) = cap else {
        return verdict(false, false, "INFO", format!("Role '{role_name}' selected — choose a capability"));
    };
    let (actor, owner) = (ActorId::new("__rbac_insp_actor__"), ActorId::new("__rbac_insp_owner__"));
    let mut cfg = rbac.clone();
    cfg.fallback_role = None;
    cfg.actor_roles.clear();
    cfg.actor_roles.insert(actor.clone(), vec![role_name.into()]);
    if cap.owner_only {
        cfg.permission_bases.entry(cap.id.clone()).or_insert(None);
    }
    match cfg.evaluate(&actor, &owner, None, &cap.id) {
        RbacDecision::Allow => {
            let pat = rbac.roles.get(role_name).and_then(|r| r.allow.iter().find(|p| matches_pattern(p, &cap.id)));
            let d = pat.map_or_else(|| format!("permitted by '{role_name}' tier base"), |p| format!("matched allow '{p}' on '{role_name}'"));
            verdict(true, false, "ALLOWED", d)
        }
        RbacDecision::Hold { reason } => verdict(true, true, "ALLOWED (HOLD)", reason),
        RbacDecision::Deny { reason } => verdict(false, false, "DENIED", reason),
    }
}

/// Applies a JSON merge `patch` cleanly onto `toml_src` using `toml_edit`.
pub fn apply_rbac_patch_to_toml(toml_src: &str, patch: &Value) -> Result<String, String> {
    let obj = patch
        .as_object()
        .ok_or_else(|| "patch must be a JSON object".to_owned())?;
    let mut doc = toml_src
        .parse::<DocumentMut>()
        .map_err(|e| format!("invalid TOML: {e}"))?;
    merge_toml_table(doc.as_table_mut(), obj)?;
    Ok(doc.to_string())
}

fn merge_toml_table(
    table: &mut dyn TableLike,
    patch: &serde_json::Map<String, Value>,
) -> Result<(), String> {
    for (key, val) in patch {
        match val {
            Value::Null => {
                table.remove(key);
            }
            Value::Object(map) => {
                if !table.get(key).is_some_and(Item::is_table_like) {
                    table.insert(key, Item::Table(Table::new()));
                }
                merge_toml_table(
                    table
                        .get_mut(key)
                        .and_then(Item::as_table_like_mut)
                        .ok_or("table")?,
                    map,
                )?;
            }
            other => {
                let mut item = json_to_toml_item(other)?;
                if let (Some(Item::Value(old)), Item::Value(nv)) = (table.get(key), &mut item) {
                    *nv.decor_mut() = old.decor().clone();
                }
                table.insert(key, item);
            }
        }
    }
    Ok(())
}

fn json_to_toml_item(val: &Value) -> Result<Item, String> {
    if let Value::Array(items) = val
        && !items.is_empty()
        && items.iter().all(Value::is_object)
    {
        let mut aot = ArrayOfTables::new();
        for it in items {
            if let TomlVal::InlineTable(t) = json_to_toml_value(it)? {
                aot.push(t.into_table());
            }
        }
        return Ok(Item::ArrayOfTables(aot));
    }
    Ok(Item::Value(json_to_toml_value(val)?))
}

fn json_to_toml_value(val: &Value) -> Result<TomlVal, String> {
    match val {
        Value::Null => Err("null not allowed in array/inline table".into()),
        Value::Bool(b) => Ok(TomlVal::from(*b)),
        Value::String(s) => Ok(TomlVal::from(s.as_str())),
        Value::Number(n) => n
            .as_i64()
            .map(TomlVal::from)
            .or_else(|| n.as_f64().map(TomlVal::from))
            .ok_or_else(|| format!("bad number '{n}'")),
        Value::Array(items) => {
            let mut arr = Array::new();
            for it in items {
                arr.push(json_to_toml_value(it)?);
            }
            Ok(TomlVal::Array(arr))
        }
        Value::Object(map) => {
            let mut tbl = InlineTable::new();
            for (k, v) in map {
                tbl.insert(k, json_to_toml_value(v)?);
            }
            Ok(TomlVal::InlineTable(tbl))
        }
    }
}

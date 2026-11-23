//! Guest access: unregistered senders are admitted with the fallback role, `agent.converse`
//! decides whether they may wake the agent, and guests are recognised again after replay.

use std::collections::BTreeMap;

use monoagens_core::{DoorDecision, Doors, EventKind, EventRecord, LogSeq, NumericFeatures};
use monoagens_protocol::{
    ActorId, Audience, OpaquePayload, OriginActor, Percept, PlaceId, Taint, Time,
};
use monoagens_valve::{
    ApprovalMode, PERMISSION_AGENT_CONVERSE, RbacConfig, RbacDecision, RoleDefinition, RoleTier,
    Valve, ValveConfig, ValveSnapshot,
};

const WA: &str = "wa:user:6285111242960@s.whatsapp.net";

fn snapshot() -> ValveSnapshot {
    let mut snap = ValveSnapshot::new(LogSeq::new(1), Time::from_millis(1_000_000));
    snap.owner_actor_id = Some(ActorId::new("per_owner"));
    snap.known_actors.insert(ActorId::new("per_owner"));
    snap.known_actors.insert(ActorId::new("per_family"));
    snap
}

fn percept(actor: &str) -> Percept {
    Percept::minimal(
        ActorId::new(actor),
        PlaceId::new("wa:dm:6285111242960@s.whatsapp.net"),
        Audience::new([ActorId::new(actor), ActorId::new("soul")]),
        Time::from_millis(1_000_000),
        OpaquePayload::from_bytes(b"halo"),
    )
}

fn role(name: &str, base: RoleTier, allow: &[&str]) -> RoleDefinition {
    RoleDefinition {
        name: name.to_owned(),
        base,
        allow: allow.iter().map(|s| (*s).to_owned()).collect(),
        deny: Vec::new(),
        approval: None,
        label: None,
        context: None,
        settings: BTreeMap::new(),
    }
}

fn rbac(guest_allow: &[&str]) -> RbacConfig {
    let mut cfg = RbacConfig {
        fallback_role: Some("guest".to_owned()),
        ..RbacConfig::default()
    };
    cfg.roles
        .insert("guest".to_owned(), role("guest", RoleTier::Guest, guest_allow));
    cfg.roles
        .insert("family".to_owned(), role("family", RoleTier::Member, &[]));
    cfg
}

fn valve(guest_access: bool, rbac: Option<RbacConfig>) -> Valve {
    let cfg = ValveConfig {
        guest_access,
        rbac,
        ..ValveConfig::default()
    };
    Valve::with_default_policies(cfg, snapshot()).expect("verified")
}

fn refusal(decision: &DoorDecision) -> Option<&str> {
    match decision {
        DoorDecision::Refuse { reason, .. } => Some(reason.as_str()),
        _ => None,
    }
}

#[test]
fn guest_access_off_keeps_unknown_audience_refusal() {
    let mut v = valve(false, None);
    let mut p = percept(WA);
    assert_eq!(refusal(&v.inbound(&mut p)), Some("unknown_audience"));
}

#[test]
fn guest_is_admitted_when_rbac_grants_converse_and_refused_otherwise() {
    // The guest role may converse once the owner adds `agent.converse` to its allow list.
    let mut allowed = valve(true, Some(rbac(&[PERMISSION_AGENT_CONVERSE])));
    let mut p = percept(WA);
    assert_eq!(allowed.inbound(&mut p), DoorDecision::Allow);

    // Without the permission the message is refused with `agent_not_permitted`, never
    // `unknown_audience`.
    let mut denied = valve(true, Some(rbac(&[])));
    let mut p = percept(WA);
    assert_eq!(refusal(&denied.inbound(&mut p)), Some("agent_not_permitted"));

    // No RBAC table: guests may not wake the agent either.
    let mut no_rbac = valve(true, None);
    let mut p = percept(WA);
    assert_eq!(refusal(&no_rbac.inbound(&mut p)), Some("agent_not_permitted"));
}

#[test]
fn registered_people_and_the_owner_still_converse() {
    let mut v = valve(true, Some(rbac(&[])));
    let mut owner = percept("per_owner");
    assert_eq!(v.inbound(&mut owner), DoorDecision::Allow);
    let mut family = percept("per_family");
    assert_eq!(v.inbound(&mut family), DoorDecision::Allow);
}

#[test]
fn explicit_permission_base_lets_guests_converse() {
    let mut cfg = rbac(&[]);
    cfg.permission_bases
        .insert(PERMISSION_AGENT_CONVERSE.to_owned(), Some(RoleTier::Guest));
    let mut v = valve(true, Some(cfg));
    let mut p = percept(WA);
    assert_eq!(v.inbound(&mut p), DoorDecision::Allow);
}

#[test]
fn guests_are_rebuilt_from_percept_events_after_restart() {
    let ev = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(900_000),
        ActorId::new(WA),
        PlaceId::new("wa:dm:6285111242960@s.whatsapp.net"),
        Audience::new([ActorId::new(WA), ActorId::new("soul")]),
        OriginActor::new(ActorId::new(WA)),
        Taint::Untrusted,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(b"halo"),
        Vec::new(),
        None,
    );
    // A fresh valve (a restart) has no guest facts until the log is replayed.
    let mut restarted = valve(true, Some(rbac(&[PERMISSION_AGENT_CONVERSE])));
    assert!(restarted.snapshot().guest_actors.is_empty());
    restarted.replay_from_events(&[ev]);
    assert!(restarted.snapshot().guest_actors.contains(&ActorId::new(WA)));
    let mut p = percept(WA);
    assert_eq!(restarted.inbound(&mut p), DoorDecision::Allow);
}

#[test]
fn admitted_guest_is_default_denied_unlisted_capabilities_while_member_base_and_owner_work() {
    let place = PlaceId::new("wa:dm:6285111242960@s.whatsapp.net");
    let guest_actor = ActorId::new(WA);
    let member_actor = ActorId::new("per_family");
    let owner_actor = ActorId::new("per_owner");

    // 1. Guest with allow = ["chat.*", "web.search", "web.fetch", "agent.converse"] and empty permission_bases.
    let mut cfg = rbac(&["chat.*", "web.search", "web.fetch", PERMISSION_AGENT_CONVERSE]);
    cfg.actor_roles
        .insert(member_actor.clone(), vec!["family".to_owned()]);

    let mut v = valve(true, Some(cfg));
    let mut p = percept(WA);
    assert_eq!(v.inbound(&mut p), DoorDecision::Allow);

    for cap in ["chat.speak", "web.search", "web.fetch", PERMISSION_AGENT_CONVERSE] {
        assert_eq!(
            v.check_rbac(&guest_actor, Some(&place), cap),
            RbacDecision::Allow,
            "guest should be allowed '{cap}'"
        );
    }
    for cap in [
        "workspace.run",
        "workspace.write",
        "pdf.delete",
        "soul_tuning.set_mode",
    ] {
        assert!(
            matches!(
                v.check_rbac(&guest_actor, Some(&place), cap),
                RbacDecision::Deny { .. }
            ),
            "guest should be denied '{cap}'"
        );
    }
    for cap in ["workspace.run", "pdf.delete", "soul_tuning.set_mode"] {
        assert_eq!(
            v.check_rbac(&owner_actor, Some(&place), cap),
            RbacDecision::Allow
        );
    }

    // 2. Explicit `[permission_bases]` ("web.search" = "member") allows member-tier role and denies guest-tier role.
    let mut base_cfg = rbac(&["chat.*", PERMISSION_AGENT_CONVERSE]);
    base_cfg
        .actor_roles
        .insert(member_actor.clone(), vec!["family".to_owned()]);
    base_cfg
        .permission_bases
        .insert("web.search".to_owned(), Some(RoleTier::Member));
    base_cfg.roles.insert(
        "owner_strict".to_owned(),
        RoleDefinition {
            name: "owner_strict".to_owned(),
            base: RoleTier::Owner,
            allow: vec!["*".to_owned()],
            deny: Vec::new(),
            approval: Some(ApprovalMode::Strict),
            label: None,
            context: None,
            settings: BTreeMap::new(),
        },
    );

    let v2 = valve(true, Some(base_cfg.clone()));
    assert_eq!(
        v2.check_rbac(&member_actor, Some(&place), "web.search"),
        RbacDecision::Allow
    );
    assert!(matches!(
        v2.check_rbac(&guest_actor, Some(&place), "web.search"),
        RbacDecision::Deny { .. }
    ));

    // 3. Owner with explicit ApprovalMode::Strict holds.
    let mut strict_owner_cfg = base_cfg;
    strict_owner_cfg
        .actor_roles
        .insert(owner_actor.clone(), vec!["owner_strict".to_owned()]);
    let v3 = valve(true, Some(strict_owner_cfg));
    assert!(matches!(
        v3.check_rbac(&owner_actor, Some(&place), "workspace.run"),
        RbacDecision::Hold { .. }
    ));
}

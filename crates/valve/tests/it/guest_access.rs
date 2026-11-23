//! Guest access: unregistered senders are admitted with the fallback role, `agent.converse`
//! decides whether they may wake the agent, and guests are recognised again after replay.

use std::collections::BTreeMap;

use monoagens_core::{DoorDecision, Doors, EventKind, EventRecord, LogSeq, NumericFeatures};
use monoagens_protocol::{
    ActorId, Audience, OpaquePayload, OriginActor, Percept, PlaceId, Taint, Time,
};
use monoagens_valve::{
    PERMISSION_AGENT_CONVERSE, RbacConfig, RoleDefinition, RoleTier, Valve, ValveConfig,
    ValveSnapshot,
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

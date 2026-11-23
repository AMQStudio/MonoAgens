use std::collections::BTreeMap;

use monoagens_core::{ContextPlan, DoorDecision, Doors, LogSeq};
use monoagens_protocol::{
    Action, ActionId, ActorId, Audience, CapabilityDescriptor, CapabilityFlags, CapabilityId,
    CapabilitySchemaHash, EventId, OpaquePayload, OriginActor, PlaceId, Taint, Time, VisibleScope,
};
use monoagens_valve::{
    ApprovalMode, AttestedPlacePresence, PlaceAccessRule, RbacConfig, RbacDecision, RoleDefinition,
    RoleTier, Valve, ValveConfig, ValveMode, ValveReasonTag, ValveSnapshot, matches_pattern,
};
use serde_json::json;

fn t_ms(ms: u64) -> Time {
    Time::from_millis(ms)
}

fn base_rbac_snapshot() -> ValveSnapshot {
    let owner = ActorId::new("owner:alice");
    let member = ActorId::new("person:bob");
    let guest = ActorId::new("person:carol");
    let strict = ActorId::new("person:strict");
    let admin = ActorId::new("person:admin");
    let place_main = PlaceId::new("room:main");
    let place_pub = PlaceId::new("room:public");

    let mut snap = ValveSnapshot::new(LogSeq::new(1), t_ms(1_000_000));
    snap.owner_actor_id = Some(owner);
    for actor in [&member, &guest, &strict, &admin] {
        snap.known_actors.insert(actor.clone());
        for place in [&place_main, &place_pub] {
            snap.place_presences.push(AttestedPlacePresence {
                actor: actor.clone(),
                place: place.clone(),
                attested_at: t_ms(990_000),
            });
        }
    }

    for (cap_id, flags) in [
        (
            "chat.speak",
            CapabilityFlags {
                communicative: true,
                identity_changing: false,
                irreversible: false,
            },
        ),
        (
            "web-search.query",
            CapabilityFlags {
                communicative: false,
                identity_changing: false,
                irreversible: false,
            },
        ),
        (
            "web-search.admin",
            CapabilityFlags {
                communicative: false,
                identity_changing: false,
                irreversible: false,
            },
        ),
        (
            "admin.config",
            CapabilityFlags {
                communicative: false,
                identity_changing: false,
                irreversible: false,
            },
        ),
        (
            "identity.self_modify",
            CapabilityFlags {
                communicative: false,
                identity_changing: true,
                irreversible: true,
            },
        ),
    ] {
        snap.register_capability(CapabilityDescriptor {
            id: CapabilityId::new(cap_id),
            description: format!("Capability {cap_id}"),
            places: vec![place_main.clone(), place_pub.clone()],
            flags,
            typical_latency_ms: 10,
            capability_schema_hash: CapabilitySchemaHash::new("h"),
            sensed_effect: None,
        });
    }

    snap
}

fn make_action(id: &str, actor: &str, cap: &str, place: &str, audience: &[&str]) -> Action {
    Action {
        protocol_version: Default::default(),
        action_id: ActionId::new(id),
        capability_id: CapabilityId::new(cap),
        places: vec![PlaceId::new(place)],
        audience: Audience::new(audience.iter().map(|s| ActorId::new(*s))),
        origin_actor: OriginActor::new(ActorId::new(actor)),
        taint: Taint::Trusted,
        payload: OpaquePayload::from_bytes(b"ok"),
        correlation_id: None,
        expected_effect: None,
        h_act: None,
        timestamp_ms: t_ms(1_000_000),
    }
}

fn sample_rbac_config() -> RbacConfig {
    let mut roles = BTreeMap::new();
    roles.insert(
        "guest".to_owned(),
        RoleDefinition {
            name: "guest".to_owned(),
            base: RoleTier::Guest,
            allow: vec!["chat.speak".to_owned()],
            deny: vec!["web-search.admin".to_owned()],
            approval: None,
            label: Some("Guest".to_owned()),
            context: Some("External visitor".to_owned()),
            settings: BTreeMap::from([("workspace.quota".to_owned(), json!(10))]),
        },
    );
    roles.insert(
        "member".to_owned(),
        RoleDefinition {
            name: "member".to_owned(),
            base: RoleTier::Member,
            allow: vec!["web-search.*".to_owned()],
            deny: vec!["web-search.admin".to_owned()],
            approval: Some(ApprovalMode::Yolo),
            label: Some("Member".to_owned()),
            context: None,
            settings: BTreeMap::from([
                ("workspace.quota".to_owned(), json!(100)),
                ("member.flag".to_owned(), json!(true)),
            ]),
        },
    );
    roles.insert(
        "strict_trusted".to_owned(),
        RoleDefinition {
            name: "strict_trusted".to_owned(),
            base: RoleTier::Trusted,
            allow: vec!["*".to_owned()],
            deny: vec![],
            approval: Some(ApprovalMode::Strict),
            label: None,
            context: None,
            settings: BTreeMap::from([("workspace.quota".to_owned(), json!(500))]),
        },
    );
    roles.insert(
        "admin".to_owned(),
        RoleDefinition {
            name: "admin".to_owned(),
            base: RoleTier::Admin,
            allow: vec!["*".to_owned()],
            deny: vec![],
            approval: Some(ApprovalMode::Default),
            label: None,
            context: None,
            settings: BTreeMap::new(),
        },
    );

    let mut actor_roles = BTreeMap::new();
    actor_roles.insert(ActorId::new("person:bob"), vec!["member".to_owned()]);
    actor_roles.insert(
        ActorId::new("person:strict"),
        vec!["member".to_owned(), "strict_trusted".to_owned()],
    );
    actor_roles.insert(ActorId::new("person:admin"), vec!["admin".to_owned()]);

    let mut places = BTreeMap::new();
    places.insert(
        PlaceId::new("room:public"),
        PlaceAccessRule {
            allow: vec!["chat.*".to_owned()],
            deny: vec!["admin.*".to_owned()],
        },
    );

    let mut permission_bases = BTreeMap::new();
    permission_bases.insert("chat.speak".to_owned(), Some(RoleTier::Guest));
    permission_bases.insert("web-search.query".to_owned(), Some(RoleTier::Member));
    permission_bases.insert("admin.config".to_owned(), Some(RoleTier::Admin));
    permission_bases.insert("system.owner_only".to_owned(), None);

    RbacConfig {
        fallback_role: Some("guest".to_owned()),
        roles,
        actor_roles,
        places,
        permission_bases,
    }
}

#[test]
fn rbac_tier_ordering_parsing_and_patterns() {
    assert!(RoleTier::Guest < RoleTier::Member);
    assert!(RoleTier::Member < RoleTier::Trusted);
    assert!(RoleTier::Trusted < RoleTier::Admin);
    assert!(RoleTier::Admin < RoleTier::Owner);

    for (tier, s) in [
        (RoleTier::Guest, "guest"),
        (RoleTier::Member, "member"),
        (RoleTier::Trusted, "trusted"),
        (RoleTier::Admin, "admin"),
        (RoleTier::Owner, "owner"),
    ] {
        assert_eq!(tier.as_str(), s);
        assert_eq!(RoleTier::parse_str(s), Some(tier));
        assert_eq!(RoleTier::parse_str(&s.to_ascii_uppercase()), Some(tier));
    }
    assert_eq!(RoleTier::parse_str("unknown"), None);

    for (mode, s) in [
        (ApprovalMode::Yolo, "yolo"),
        (ApprovalMode::Default, "default"),
        (ApprovalMode::Strict, "strict"),
    ] {
        assert_eq!(mode.as_str(), s);
        assert_eq!(ApprovalMode::parse_str(s), Some(mode));
    }
    assert_eq!(ApprovalMode::parse_str("bogus"), None);

    assert!(matches_pattern("*", "anything.at.all"));
    assert!(matches_pattern("web-search.*", "web-search.query"));
    assert!(!matches_pattern("web-search.*", "web-search2.query"));
    assert!(!matches_pattern("web-search.*", "web-search"));
    assert!(matches_pattern("web*", "web-search.query"));
    assert!(matches_pattern("chat.speak", "chat.speak"));
    assert!(!matches_pattern("chat.speak", "chat.reply"));
}

#[test]
fn rbac_tiers_owner_only_globs_places_and_role_settings() {
    let rbac = sample_rbac_config();
    let owner = ActorId::new("owner:alice");
    let bob = ActorId::new("person:bob");
    let carol = ActorId::new("person:carol"); // falls back to "guest"
    let strict = ActorId::new("person:strict");
    let admin = ActorId::new("person:admin");
    let main_room = PlaceId::new("room:main");
    let pub_room = PlaceId::new("room:public");

    assert_eq!(rbac.effective_tier(&owner, &owner), RoleTier::Owner);
    assert_eq!(rbac.effective_tier(&bob, &owner), RoleTier::Member);
    assert_eq!(rbac.effective_tier(&carol, &owner), RoleTier::Guest);
    assert_eq!(rbac.effective_tier(&strict, &owner), RoleTier::Trusted);
    assert_eq!(rbac.effective_tier(&admin, &owner), RoleTier::Admin);

    // Owner-only (`None`) base refuses non-owners (even Admin), allows Owner.
    assert_eq!(
        rbac.evaluate(&owner, &owner, Some(&main_room), "system.owner_only"),
        RbacDecision::Allow
    );
    assert!(matches!(
        rbac.evaluate(&admin, &owner, Some(&main_room), "system.owner_only"),
        RbacDecision::Deny { .. }
    ));
    assert!(!rbac.is_permitted(&admin, &owner, Some(&main_room), "system.owner_only"));

    // Explicit deny beats allow (`web-search.*` allows `web-search.query`, `deny` blocks `web-search.admin`).
    assert_eq!(
        rbac.evaluate(&bob, &owner, Some(&main_room), "web-search.query"),
        RbacDecision::Allow
    );
    assert!(matches!(
        rbac.evaluate(&bob, &owner, Some(&main_room), "web-search.admin"),
        RbacDecision::Deny { .. }
    ));

    // Guest (`carol` via fallback) is below `member` base for `web-search.query` and `admin` for `admin.config`.
    assert!(matches!(
        rbac.evaluate(&carol, &owner, Some(&main_room), "web-search.query"),
        RbacDecision::Deny { .. }
    ));
    assert!(matches!(
        rbac.evaluate(&bob, &owner, Some(&main_room), "admin.config"),
        RbacDecision::Deny { .. }
    ));
    assert_eq!(
        rbac.evaluate(&admin, &owner, Some(&main_room), "admin.config"),
        RbacDecision::Allow
    );

    // Place narrowing: `room:public` only allows `chat.*` and denies `admin.*` for non-owners,
    // while `owner` remains above place restrictions.
    assert_eq!(
        rbac.evaluate(&bob, &owner, Some(&pub_room), "chat.speak"),
        RbacDecision::Allow
    );
    assert!(matches!(
        rbac.evaluate(&bob, &owner, Some(&pub_room), "web-search.query"),
        RbacDecision::Deny { .. }
    ));
    assert!(matches!(
        rbac.evaluate(&admin, &owner, Some(&pub_room), "admin.config"),
        RbacDecision::Deny { .. }
    ));
    assert_eq!(
        rbac.evaluate(&owner, &owner, Some(&pub_room), "admin.config"),
        RbacDecision::Allow
    );

    // Role settings merge deterministically in ascending tier order (`strict_trusted` overrides `member`).
    let bob_settings = rbac.effective_settings(&bob);
    assert_eq!(bob_settings.get("workspace.quota"), Some(&json!(100)));
    let strict_settings = rbac.effective_settings(&strict);
    assert_eq!(strict_settings.get("workspace.quota"), Some(&json!(500)));
    assert_eq!(strict_settings.get("member.flag"), Some(&json!(true)));
    assert_eq!(
        rbac.effective_approval_mode(&strict, &owner),
        ApprovalMode::Strict
    );
}

#[test]
fn valve_action_rbac_narrows_and_strict_mode_holds_with_pending_approval() {
    let mut rbac = sample_rbac_config();
    rbac.roles.insert(
        "owner_strict".to_owned(),
        RoleDefinition {
            name: "owner_strict".to_owned(),
            base: RoleTier::Owner,
            allow: vec!["*".to_owned()],
            deny: vec![],
            approval: Some(ApprovalMode::Strict),
            label: None,
            context: None,
            settings: BTreeMap::new(),
        },
    );

    let snap = base_rbac_snapshot();
    let cfg = ValveConfig {
        rbac: Some(rbac),
        ..Default::default()
    };
    let mut valve = Valve::with_default_policies(cfg, snap).expect("verified");

    assert!(valve.rbac_config().is_some());
    assert_eq!(
        valve.check_rbac(
            &ActorId::new("person:bob"),
            Some(&PlaceId::new("room:main")),
            "web-search.query"
        ),
        RbacDecision::Allow
    );

    // Guest (`person:carol`) denied by RBAC for `web-search.query`, audit log carries CedarDenied.
    let mut guest_search = make_action(
        "act-guest-search",
        "person:carol",
        "web-search.query",
        "room:main",
        &["person:carol"],
    );
    let d1 = valve.action(&mut guest_search);
    assert!(matches!(
        d1,
        DoorDecision::Refuse { ref rule, .. } if rule == "valve.action.rbac"
    ));
    let last_entry = valve.audit_log().entries().last().expect("audit entry");
    assert_eq!(last_entry.reason_tag, Some(ValveReasonTag::CedarDenied));

    // `person:strict` (`ApprovalMode::Strict`) holds R0 `chat.speak` via PendingApproval (`H_act`, TTL, grant).
    let mut strict_chat = make_action(
        "act-strict-1",
        "person:strict",
        "chat.speak",
        "room:main",
        &["person:strict"],
    );
    let d2 = valve.action(&mut strict_chat);
    assert!(matches!(
        d2,
        DoorDecision::Hold { ref rule, ref reason }
            if rule == "valve.approval.pending" && reason == "PendingApproval"
    ));
    assert!(strict_chat.h_act.is_some());
    assert_eq!(valve.approval_manager().pending().len(), 1);

    // Approving out-of-band with owner signature lets the exact held action pass on retry.
    let grant = valve.approve_action_signed(&strict_chat, false, ActorId::new("owner:alice"));
    assert_eq!(grant.action_id, strict_chat.action_id);
    assert_eq!(valve.action(&mut strict_chat), DoorDecision::Allow);
}

#[test]
fn rbac_never_widens_line1_line2_or_taint_quarantine() {
    let mut snap = base_rbac_snapshot();
    let admin = ActorId::new("person:admin");
    snap.known_actors.insert(admin.clone());
    snap.place_presences.push(AttestedPlacePresence {
        actor: admin.clone(),
        place: PlaceId::new("room:main"),
        attested_at: t_ms(990_000),
    });

    // Give non-owner `admin` wildcard `allow: ["*"]` and `RoleTier::Owner` base.
    let mut rbac = sample_rbac_config();
    rbac.roles.insert(
        "super_non_owner".to_owned(),
        RoleDefinition {
            name: "super_non_owner".to_owned(),
            base: RoleTier::Owner,
            allow: vec!["*".to_owned()],
            deny: vec![],
            approval: Some(ApprovalMode::Yolo),
            label: None,
            context: None,
            settings: BTreeMap::new(),
        },
    );
    rbac.actor_roles
        .insert(admin.clone(), vec!["super_non_owner".to_owned()]);

    let cfg = ValveConfig {
        rbac: Some(rbac),
        ..Default::default()
    };
    let mut valve = Valve::with_default_policies(cfg, snap).expect("verified");

    // 1. Absolute Line 1: non-owner `identity_changing` MUST still be refused with NonOwnerIdentityChange!
    let mut id_change = make_action(
        "act-line1",
        "person:admin",
        "identity.self_modify",
        "room:main",
        &["person:admin"],
    );
    let d_line1 = valve.action(&mut id_change);
    assert!(matches!(
        d_line1,
        DoorDecision::Refuse { ref reason, .. } if reason == ValveReasonTag::NonOwnerIdentityChange.as_str()
    ));

    // 2. Absolute Line 2: out-of-scope context item MUST still be stripped!
    let secret_ev = EventId::new(42);
    valve.snapshot_mut().register_item_audience(
        secret_ev,
        Audience::new([ActorId::new("owner:alice")]),
    );
    let mut plan = ContextPlan {
        place: Some(PlaceId::new("room:main")),
        items: vec![secret_ev],
        visible_scope: Some(VisibleScope::new(
            Audience::new([admin.clone()]),
            Some(PlaceId::new("room:main")),
        )),
        ..ContextPlan::default()
    };
    let d_line2 = valve.context(&mut plan);
    assert!(matches!(
        d_line2,
        DoorDecision::Strip { ref removed_ids, .. } if removed_ids == &vec![secret_ev]
    ));

    // 3. Cross-audience taint quarantine MUST still refuse even when RBAC allows `*` + `Yolo`!
    valve.snapshot_mut().active_context_audience = Some(Audience::new([admin]));
    let mut cross_act = make_action(
        "act-cross",
        "person:admin",
        "chat.speak",
        "room:main",
        &["person:bob"],
    );
    cross_act.taint = Taint::Untrusted;
    let d_taint = valve.action(&mut cross_act);
    assert!(matches!(
        d_taint,
        DoorDecision::Refuse { ref reason, .. } if reason == ValveReasonTag::TaintBlocked.as_str()
    ));

    // 4. `ValveMode::Off` and `rbac_enabled = false` disable RBAC checks cleanly.
    let mut cfg_off = valve.config().clone();
    cfg_off.rbac_enabled = false;
    valve.set_config(cfg_off);
    assert_eq!(
        valve.check_rbac(
            &ActorId::new("person:carol"),
            Some(&PlaceId::new("room:main")),
            "system.owner_only"
        ),
        RbacDecision::Allow
    );

    let mut cfg_mode_off = valve.config().clone();
    cfg_mode_off.rbac_enabled = true;
    cfg_mode_off.mode = ValveMode::Off;
    valve.set_config(cfg_mode_off);
    assert_eq!(
        valve.check_rbac(
            &ActorId::new("person:carol"),
            Some(&PlaceId::new("room:main")),
            "system.owner_only"
        ),
        RbacDecision::Allow
    );
}

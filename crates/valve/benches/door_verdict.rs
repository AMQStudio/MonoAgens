use std::hint::black_box;

use criterion::{Criterion, criterion_group, criterion_main};
use monoagens_core::{Doors, LogSeq};
use monoagens_protocol::{
    Action, ActionId, ActorId, Audience, CapabilityDescriptor, CapabilityFlags, CapabilityId,
    CapabilitySchemaHash, OpaquePayload, OriginActor, PlaceId, Taint, Time,
};
use monoagens_valve::{AttestedPlacePresence, Valve, ValveConfig, ValveSnapshot};

fn reference_valve_and_action() -> (Valve, Action) {
    let owner = ActorId::new("owner:primary");
    let alice = ActorId::new("person:alice");
    let place = PlaceId::new("room:main");
    let cap_id = CapabilityId::new("chat.speak");

    let mut snapshot = ValveSnapshot::new(LogSeq::new(42), Time::from_millis(1_000_000));
    snapshot.owner_actor_id = Some(owner);
    snapshot.known_actors.insert(alice.clone());
    snapshot.place_presences.push(AttestedPlacePresence {
        actor: alice.clone(),
        place: place.clone(),
        attested_at: Time::from_millis(950_000),
    });
    snapshot.register_capability(CapabilityDescriptor {
        id: cap_id.clone(),
        description: "Send a chat message".to_owned(),
        places: vec![place.clone()],
        flags: CapabilityFlags {
            communicative: true,
            identity_changing: false,
            irreversible: false,
        },
        typical_latency_ms: 50,
        capability_schema_hash: CapabilitySchemaHash::new("sha256:chat_speak"),
        sensed_effect: None,
    });

    let valve =
        Valve::with_default_policies(ValveConfig::default(), snapshot).expect("verified default");

    let action = Action {
        protocol_version: Default::default(),
        action_id: ActionId::new("act-bench-1"),
        capability_id: cap_id,
        places: vec![place],
        audience: Audience::new([alice.clone()]),
        origin_actor: OriginActor::new(alice),
        taint: Taint::Trusted,
        payload: OpaquePayload::from_bytes(b"hello"),
        correlation_id: None,
        expected_effect: None,
        h_act: None,
        timestamp_ms: Time::from_millis(1_000_000),
    };

    (valve, action)
}

fn bench_door_verdict(c: &mut Criterion) {
    let (mut valve, mut action) = reference_valve_and_action();

    c.bench_function("door_verdict", |b| {
        b.iter(|| {
            let decision = valve.action(black_box(&mut action));
            black_box(decision)
        });
    });
}

criterion_group!(benches, bench_door_verdict);
criterion_main!(benches);

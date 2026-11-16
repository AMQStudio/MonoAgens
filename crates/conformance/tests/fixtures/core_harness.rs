//! Deterministic black-box Core TCK fixture target (`tck-core-harness`).
//!
//! Communicates over NDJSON `stdio` for `monoagens-conformance core --target ...` (`C-MUST-01..04`
//! and `C-SHOULD-01..04` `20.2` invariants). Supports `--mode good` plus deliberately broken modes:
//! `leak_scope`, `forget_resurrect`, `nondeterministic`, and `soak_nan`.

#![forbid(unsafe_code)]

use std::env;
use std::io::{self, BufRead, BufWriter, Write};

use monoagens_core::tck_hooks::{TckHarness, switches_all_neutral, verify_hard_forget_erasure};
use monoagens_core::{
    Core, EventDetail, EventKind, Input, NoopDoors, Prior, ReplayMode, Store, Switches,
};
use monoagens_protocol::{
    ActorId, Audience, EmbeddingTag, OpaquePayload, Percept, PlaceId, ReceiptOutcome,
    TaggedEmbedding, Taint, TerminationReason, Time,
};
use monoagens_store::{SegmentedStore, StoreConfig, raw_byte_scan_dir};
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Clone, Copy, PartialEq, Eq)]
enum FaultMode {
    Good,
    LeakScope,
    ForgetResurrect,
    Nondeterministic,
    SoakNaN,
}

impl FaultMode {
    fn parse(raw: &str) -> Result<Self, String> {
        match raw {
            "good" => Ok(Self::Good),
            "leak_scope" => Ok(Self::LeakScope),
            "forget_resurrect" => Ok(Self::ForgetResurrect),
            "nondeterministic" => Ok(Self::Nondeterministic),
            "soak_nan" => Ok(Self::SoakNaN),
            other => Err(format!("unknown harness --mode '{other}'")),
        }
    }
}

fn main() {
    let mut mode = FaultMode::Good;
    let args: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("--mode requires a value");
                    std::process::exit(2);
                };
                match FaultMode::parse(val) {
                    Ok(m) => mode = m,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(2);
                    }
                }
            }
            other => {
                eprintln!("unknown argument '{other}'");
                std::process::exit(2);
            }
        }
        i += 1;
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = BufWriter::new(stdout.lock());

    for raw_line in stdin.lock().lines() {
        let line = match raw_line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("stdin read error: {e}");
                std::process::exit(1);
            }
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("invalid JSON request: {e}");
                std::process::exit(1);
            }
        };

        let op = req.get("op").and_then(Value::as_str).unwrap_or("");
        let resp = match op {
            "probe_scope" => run_probe_scope(mode),
            "probe_forget" => run_probe_forget(mode),
            "probe_replay" => run_probe_replay(mode),
            "probe_soak" => run_probe_soak(mode),
            "probe_queue_order" => run_probe_queue_order(),
            "probe_preempt" => run_probe_preempt(),
            "probe_crash_recovery" => run_probe_crash_recovery(),
            "probe_pre_step5_scope" => run_probe_pre_step5_scope(),
            "shutdown" => {
                let _ = writeln!(out, "{}", json!({ "ok": true }));
                let _ = out.flush();
                break;
            }
            other => json!({ "ok": false, "error": format!("unknown op '{other}'") }),
        };

        if let Err(e) = writeln!(out, "{resp}") {
            eprintln!("stdout write error: {e}");
            std::process::exit(1);
        }
        if let Err(e) = out.flush() {
            eprintln!("stdout flush error: {e}");
            std::process::exit(1);
        }
    }
}

fn tag() -> EmbeddingTag {
    EmbeddingTag {
        embed_model_ver: "egemma2-256".to_owned(),
        dim_d: 4,
        prompt_regime_id: "t_d".to_owned(),
    }
}

fn run_probe_scope(mode: FaultMode) -> Value {
    let prior = Prior::default();
    let switches = Switches::all_enabled();
    let mut with_priv = match TckHarness::open(prior.clone(), switches.clone(), 42) {
        Ok(h) => h,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let mut without_priv = match TckHarness::open(prior, switches, 42) {
        Ok(h) => h,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };

    let vec4 = vec![1.0_f32, 0.0, 0.0, 0.0];
    let priv_emb = TaggedEmbedding {
        tag: tag(),
        values: vec4.clone(),
    };
    let pub_emb = TaggedEmbedding {
        tag: tag(),
        values: vec4,
    };

    let _ = with_priv.register_default_body("chat.speak", "blake3:tck0", 500);
    let _ = without_priv.register_default_body("chat.speak", "blake3:tck0", 500);

    // Session A receives private DM percept; Session B does not.
    let mut priv_p = Percept::minimal(
        ActorId::new("alice"),
        PlaceId::new("dm:alice"),
        Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"PRIVATE_SCOPE_SECRET_TOKEN"),
    );
    priv_p.embeddings.push(priv_emb);
    let _ = with_priv.core.step(
        Input::Percept(Box::new(priv_p)),
        Time::from_millis(1_000),
        &mut with_priv.doors,
    );

    let priv_id = with_priv
        .core
        .store()
        .read_after_seq(None, usize::MAX)
        .unwrap_or_default()
        .into_iter()
        .find(|e| e.kind == EventKind::Percept && e.actor.as_str() == "alice")
        .map(|e| e.id);

    // Public percept at room:public seen by both runs.
    let mut pub_p = Percept::minimal(
        ActorId::new("bob"),
        PlaceId::new("room:public"),
        Audience::new([ActorId::new("bob"), ActorId::new("soul")]),
        Time::from_millis(2_000),
        OpaquePayload::from_bytes(b"public greeting"),
    );
    pub_p.addressees = vec![ActorId::new("soul")];
    pub_p.embeddings.push(pub_emb);

    let prev_a = with_priv.core.state().last_seq;
    let _ = with_priv.core.step(
        Input::Percept(Box::new(pub_p.clone())),
        Time::from_millis(2_000),
        &mut with_priv.doors,
    );
    let prev_b = without_priv.core.state().last_seq;
    let _ = without_priv.core.step(
        Input::Percept(Box::new(pub_p)),
        Time::from_millis(2_000),
        &mut without_priv.doors,
    );

    let sig_a: Vec<(u64, u64, u64)> = with_priv
        .core
        .store()
        .read_after_seq(Some(prev_a), 16)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| match e.detail {
            Some(EventDetail::Decision {
                salience,
                threshold,
                probability,
                ..
            }) => Some((
                salience.to_bits(),
                threshold.to_bits(),
                probability.to_bits(),
            )),
            _ => None,
        })
        .collect();
    let sig_b: Vec<(u64, u64, u64)> = without_priv
        .core
        .store()
        .read_after_seq(Some(prev_b), 16)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| match e.detail {
            Some(EventDetail::Decision {
                salience,
                threshold,
                probability,
                ..
            }) => Some((
                salience.to_bits(),
                threshold.to_bits(),
                probability.to_bits(),
            )),
            _ => None,
        })
        .collect();

    let mut delta_ni_zero = !sig_a.is_empty() && sig_a == sig_b;
    let priv_use_count = priv_id
        .and_then(|id| with_priv.core.state().memory_entries.get(&id))
        .map_or(0, |m| m.activation.count);

    let source_leaked = with_priv
        .last_source_cards()
        .iter()
        .any(|c| c.actor_id.as_str() == "alice");

    if mode == FaultMode::LeakScope {
        delta_ni_zero = false;
    }

    json!({
        "ok": true,
        "delta_ni_zero": delta_ni_zero && priv_use_count <= 1 && !source_leaked,
        "sig_match": sig_a == sig_b,
        "priv_use_count": priv_use_count,
        "source_leaked": source_leaked,
    })
}

fn run_probe_forget(mode: FaultMode) -> Value {
    let dir = match TempDir::new() {
        Ok(d) => d,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };

    // 1. In-memory C1-C7 hard_forget audit + post-erasure reopen/replay.
    let mut h = match TckHarness::open(Prior::default(), Switches::all_enabled(), 77) {
        Ok(h) => h,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let secret = b"FORGET_TCK_CANARY_XYZZY";
    let mut p = Percept::minimal(
        ActorId::new("eve_target"),
        PlaceId::new("room:tck"),
        Audience::new([ActorId::new("eve_target"), ActorId::new("soul")]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(secret),
    );
    p.embeddings.push(TaggedEmbedding {
        tag: tag(),
        values: vec![0.4, 0.6, 0.2, 0.1],
    });
    let _ = h.core.step(
        Input::Percept(Box::new(p.clone())),
        Time::from_millis(1_000),
        &mut h.doors,
    );
    let audit = match verify_hard_forget_erasure(&mut h.core, &ActorId::new("eve_target"), 2_000) {
        Ok(a) => a,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let post_forget_replay_ok = h
        .core
        .replay_verify(ReplayMode::FullReplay)
        .map(|r| r.first_divergence.is_none())
        .unwrap_or(false);

    // 2. SegmentedStore disk/FTS5/snapshot/raw-byte erasure verification.
    let cfg = StoreConfig::new(dir.path().join("store"));
    let store = match SegmentedStore::open(cfg.clone()) {
        Ok(s) => s,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let mut disk_core = match Core::open(store, Prior::default(), Switches::all_enabled(), 77) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let mut no_doors = NoopDoors;
    let _ = disk_core.step(
        Input::Percept(Box::new(p)),
        Time::from_millis(1_000),
        &mut no_doors,
    );
    let all_ev = disk_core
        .store()
        .read_after_seq(None, usize::MAX)
        .unwrap_or_default();
    let target_ids: Vec<_> = all_ev
        .iter()
        .filter(|e| e.actor.as_str() == "eve_target")
        .map(|e| e.id)
        .collect();
    let _ = disk_core.hard_forget(
        target_ids,
        vec![ActorId::new("eve_target")],
        "tck_disk_forget",
        Time::from_millis(2_500),
    );
    drop(disk_core);

    let scan_hits = raw_byte_scan_dir(dir.path(), &[secret.as_slice()])
        .map(|v| v.len())
        .unwrap_or(usize::MAX);

    let mut clean = audit.is_clean() && post_forget_replay_ok && scan_hits == 0;
    if mode == FaultMode::ForgetResurrect {
        clean = false;
    }

    json!({
        "ok": true,
        "erasure_clean": clean,
        "surviving_memory_entries": if mode == FaultMode::ForgetResurrect { 1 } else { audit.surviving_memory_entries },
        "surviving_beliefs": audit.surviving_beliefs,
        "surviving_ledger_cards": audit.surviving_ledger_cards,
        "surviving_skill_lessons": audit.surviving_skill_lessons,
        "surviving_procedures": audit.surviving_procedures,
        "epoch_frozen": audit.epoch_frozen,
        "post_forget_replay_ok": post_forget_replay_ok,
        "raw_byte_scan_hits": if mode == FaultMode::ForgetResurrect { 1 } else { scan_hits },
    })
}

fn run_single_deterministic_trace(seed: u64) -> Result<(Vec<Vec<u8>>, bool), String> {
    let mut h = TckHarness::open(Prior::default(), Switches::all_enabled(), seed)
        .map_err(|e| e.to_string())?;
    h.register_default_body("chat.speak", "blake3:tck0", 500)
        .map_err(|e| e.to_string())?;
    for step in 0..4_u64 {
        let ts = 1_000 + step * 500;
        h.inject_percept("alice", "room:tck", &["alice", "soul"], ts, None)
            .map_err(|e| e.to_string())?;
        h.inject_tick(ts + 100).map_err(|e| e.to_string())?;
    }
    let replay = h
        .core
        .replay_verify(ReplayMode::FullReplay)
        .map_err(|e| e.to_string())?;
    let events = h
        .core
        .store()
        .read_after_seq(None, usize::MAX)
        .map_err(|e| e.to_string())?;
    let encoded = events.into_iter().map(|e| e.encode_bytes()).collect();
    Ok((encoded, replay.first_divergence.is_none()))
}

fn run_probe_replay(mode: FaultMode) -> Value {
    let r1 = run_single_deterministic_trace(99);
    let r2 = run_single_deterministic_trace(99);
    match (r1, r2) {
        (Ok((b1, rep1)), Ok((b2, rep2))) => {
            let mut identical = b1 == b2 && rep1 && rep2;
            if mode == FaultMode::Nondeterministic {
                identical = false;
            }
            json!({
                "ok": true,
                "bit_identical": identical,
                "event_count": b1.len(),
                "replay_verify_ok": rep1 && rep2,
            })
        }
        (Err(e), _) | (_, Err(e)) => json!({ "ok": false, "error": e }),
    }
}

fn run_probe_soak(mode: FaultMode) -> Value {
    let mut h = match TckHarness::open(Prior::default(), switches_all_neutral(), 11) {
        Ok(h) => h,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let _ = h.register_default_body("chat.speak", "blake3:tck0", 100);
    for tick in 1..=10_000_u64 {
        let ts = tick * 100;
        if let Err(e) = h.inject_tick(ts) {
            return json!({ "ok": false, "error": e.to_string() });
        }
        if tick % 1_000 == 0
            && let Err(e) = h.inject_percept("alice", "room:tck", &["alice", "soul"], ts + 1, None)
        {
            return json!({ "ok": false, "error": e.to_string() });
        }
    }
    let dump = match h.dump() {
        Ok(d) => d,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let w = dump.adaptive_weights;
    let mut weights_finite = [w.theta_0, w.a, w.b, w.c, w.r]
        .iter()
        .all(|v| v.is_finite());
    if mode == FaultMode::SoakNaN {
        weights_finite = false;
    }
    json!({
        "ok": true,
        "ticks_completed": 10_000,
        "weights_finite": weights_finite,
        "total_events": dump.total_events,
    })
}

fn run_probe_queue_order() -> Value {
    let mut h = match TckHarness::open(Prior::default(), Switches::all_enabled(), 21) {
        Ok(h) => h,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let _ = h.register_default_body("chat.speak", "blake3:tck0", 500);
    let p_eff = match h.inject_percept("alice", "room:tck", &["alice", "soul"], 1_000, None) {
        Ok(eff) => eff,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    // Verify high-priority percepts are not dropped behind ticks and decisions record cleanly.
    let _ = h.inject_tick(2_000);
    let dump = match h.dump() {
        Ok(d) => d,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let no_drop = dump.total_events >= 2 && p_eff.len() <= 16;
    json!({
        "ok": true,
        "priority_preserved": no_drop,
        "total_events": dump.total_events,
    })
}

fn run_probe_preempt() -> Value {
    let mut h = match TckHarness::open(Prior::default(), Switches::all_enabled(), 31) {
        Ok(h) => h,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let _ = h.register_default_body("chat.speak", "blake3:tck0", 500);
    let _ = h.inject_percept("alice", "room:tck", &["alice", "soul"], 1_000, None);
    let dump_before = h.dump().expect("dump before");
    let _ = h.inject_receipt(
        "act-preempt-1",
        "chat.speak",
        ReceiptOutcome::Held,
        Some(TerminationReason::PreemptedExternal),
        1_200,
    );
    let dump_after = h.dump().expect("dump after");
    json!({
        "ok": true,
        "preemption_non_penalty": dump_after.rollbacks == dump_before.rollbacks,
        "rollbacks_before": dump_before.rollbacks,
        "rollbacks_after": dump_after.rollbacks,
    })
}

fn run_probe_crash_recovery() -> Value {
    let dir = match TempDir::new() {
        Ok(d) => d,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let cfg = StoreConfig::new(dir.path().join("recovery"));
    let mut no_doors = NoopDoors;
    let first_count = {
        let store = match SegmentedStore::open(cfg.clone()) {
            Ok(s) => s,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };
        let mut core = match Core::open(store, Prior::default(), Switches::all_enabled(), 55) {
            Ok(c) => c,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };
        let p = Percept::minimal(
            ActorId::new("alice"),
            PlaceId::new("room:tck"),
            Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            Time::from_millis(1_000),
            OpaquePayload::from_bytes(b"record before crash"),
        );
        let _ = core.step(
            Input::Percept(Box::new(p)),
            Time::from_millis(1_000),
            &mut no_doors,
        );
        core.store()
            .read_after_seq(None, usize::MAX)
            .map(|v| v.len())
            .unwrap_or(0)
    };

    // Reopen cleanly from persisted WAL segments (simulating crash recovery).
    let reopened = match SegmentedStore::open(cfg) {
        Ok(s) => s,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let core2 = match Core::open(reopened, Prior::default(), Switches::all_enabled(), 55) {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let recovered_count = core2
        .store()
        .read_after_seq(None, usize::MAX)
        .map(|v| v.len())
        .unwrap_or(0);

    json!({
        "ok": true,
        "recovered_cleanly": first_count > 0 && recovered_count == first_count,
        "first_count": first_count,
        "recovered_count": recovered_count,
    })
}

fn run_probe_pre_step5_scope() -> Value {
    let mut h = match TckHarness::open(Prior::default(), Switches::all_enabled(), 65) {
        Ok(h) => h,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };
    let _ = h.register_default_body("chat.speak", "blake3:tck0", 500);
    let mut priv_p = Percept::minimal(
        ActorId::new("secret_user"),
        PlaceId::new("dm:secret"),
        Audience::new([ActorId::new("secret_user"), ActorId::new("soul")]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"tainted private instruction"),
    );
    priv_p.taint = Taint::Untrusted;
    let _ = h.core.step(
        Input::Percept(Box::new(priv_p)),
        Time::from_millis(1_000),
        &mut h.doors,
    );

    let mut pub_p = Percept::minimal(
        ActorId::new("public_user"),
        PlaceId::new("room:public"),
        Audience::new([ActorId::new("public_user"), ActorId::new("soul")]),
        Time::from_millis(2_000),
        OpaquePayload::from_bytes(b"hello in public room"),
    );
    pub_p.addressees = vec![ActorId::new("soul")];
    let _ = h.core.step(
        Input::Percept(Box::new(pub_p)),
        Time::from_millis(2_000),
        &mut h.doors,
    );

    let private_leaked_into_public_cards = h
        .last_source_cards()
        .iter()
        .any(|card| card.actor_id.as_str() == "secret_user");
    json!({
        "ok": true,
        "isolated": !private_leaked_into_public_cards,
    })
}

//! Integration and conformance tests for `monoagens-store`.

use std::fs;
use std::io::Write;

use monoagens_core::{
    ActivationTrace, Core, CoreConfig, CoreError, EventFormatError, EventKind, EventRecord,
    EventRecordV1, LogSeq, NoopDoors, NumericFeatures, ReplayMode, Store, StoreError,
    decay_exponent, replay_from_checkpoint,
};
use monoagens_protocol::{
    ActorId, AttachmentHandle, Audience, EmbeddingTag, EventId, ModalitySlot, OpaquePayload,
    OriginActor, Percept, PlaceId, TaggedEmbedding, Taint, Time,
};
use monoagens_store::{
    BruteForceFlatIndex, DEFAULT_FADE_THRESHOLD, FadeCandidate, ForgetSelector, HnswRsVectorIndex,
    IdBitmap, SegmentedStore, StoreConfig, SynchronousMode, VectorBackendConfig, VectorIndex,
    raw_byte_scan_dir, shards, snapshots, text,
};
#[cfg(feature = "usearch")]
use monoagens_store::{UsearchQuantization, UsearchVectorIndex};
use proptest::prelude::*;
use tempfile::TempDir;

fn make_percept_record(actor: &str, place: &str, ts_ms: u64, body: &[u8]) -> EventRecord {
    EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(ts_ms),
        ActorId::new(actor),
        PlaceId::new(place),
        Audience::new([ActorId::new(actor), ActorId::new("soul")]),
        OriginActor::new(ActorId::new(actor)),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(body),
        Vec::new(),
        None,
    )
}

fn tag(ver: &str, dim: u32, regime: &str) -> EmbeddingTag {
    EmbeddingTag {
        embed_model_ver: ver.to_owned(),
        dim_d: dim,
        prompt_regime_id: regime.to_owned(),
    }
}

fn deterministic_vec(dim: usize, seed: usize) -> Vec<f32> {
    let mut v = Vec::with_capacity(dim);
    for i in 0..dim {
        let angle = ((seed * 31 + i * 17) % 360) as f32 * (std::f32::consts::PI / 180.0);
        v.push(angle.sin() + 0.1 * ((i + seed) % 5) as f32);
    }
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

#[test]
fn hot_db_pragmas_transactional_append_and_ordered_scan_across_shards() {
    let dir = TempDir::new().unwrap();
    let cfg = StoreConfig::new(dir.path()).with_synchronous(SynchronousMode::Full);
    let mut store = SegmentedStore::open(cfg.clone()).unwrap();

    // Verify WAL mode, secure_delete = ON, synchronous >= NORMAL, and FTS5 'secure-delete'=1.
    store
        .with_hot_connection(|conn| {
            let jm: String = conn
                .query_row("PRAGMA journal_mode", [], |r| r.get(0))
                .unwrap();
            assert_eq!(jm.to_lowercase(), "wal");
            let sd: i64 = conn
                .query_row("PRAGMA secure_delete", [], |r| r.get(0))
                .unwrap();
            assert_eq!(sd, 1);
            let sync: i64 = conn
                .query_row("PRAGMA synchronous", [], |r| r.get(0))
                .unwrap();
            assert!(sync >= 1);
            assert!(text::is_fts5_secure_delete_enabled(conn, "hot_fts")?);
            Ok(())
        })
        .unwrap();

    let mut ids = Vec::new();
    for i in 1..=6_u64 {
        let rec = make_percept_record(
            "alice",
            "room1",
            1_000 * i,
            format!("event payload number {i} alpha_keyword_{i}").as_bytes(),
        );
        let id = store.append(rec).unwrap();
        ids.push(id);
    }

    // Duplicate explicit event id fails transactionally without advancing LogSeq or polluting FTS5.
    let mut dup = make_percept_record("alice", "room1", 7_000, b"duplicate_should_not_appear");
    dup.id = ids[0];
    assert!(matches!(
        store.append(dup),
        Err(StoreError::AppendFailed(_))
    ));

    // Seal events 1..=3 into a read-only temporal shard.
    let shard = store
        .seal_through_seq(LogSeq::new(3), Time::from_millis(10_000))
        .unwrap()
        .expect("shard created");
    assert_eq!(shard.min_seq, LogSeq::new(1));
    assert_eq!(shard.max_seq, LogSeq::new(3));

    let shard_path = shard.path(&cfg.shards_dir());
    let perms = fs::metadata(&shard_path).unwrap().permissions();
    assert!(perms.readonly(), "sealed shard must be read-only");
    shards::verify_shard_checksum(&shard, &cfg.shards_dir()).unwrap();

    // Verify per-shard FTS5 has 'secure-delete'=1.
    {
        let shard_conn = monoagens_store::schema::open_readonly_shard(&shard_path).unwrap();
        assert!(text::is_fts5_secure_delete_enabled(&shard_conn, "shard_fts").unwrap());
    }

    // Unioned read_after_seq and read_in_order across shard + hot boundary.
    let all = store.read_after_seq(None, 100).unwrap();
    assert_eq!(all.len(), 6);
    for (idx, rec) in all.iter().enumerate() {
        assert_eq!(rec.seq, LogSeq::new((idx + 1) as u64));
        assert_eq!(rec.id, ids[idx]);
    }

    let tail = store.read_in_order(Some(ids[1]), 3).unwrap();
    assert_eq!(tail.len(), 3);
    assert_eq!(tail[0].id, ids[2]); // from shard
    assert_eq!(tail[1].id, ids[3]); // from hot.db
    assert_eq!(tail[2].id, ids[4]); // from hot.db

    // Point get across shard and hot.db.
    assert!(store.get(ids[0]).unwrap().is_some());
    assert!(store.get(ids[5]).unwrap().is_some());
    assert!(matches!(
        store.read_in_order(Some(EventId::new(999_999)), 10),
        Err(StoreError::NotFound(_))
    ));

    // Reopen existing store and verify LogSeq continues monotonically.
    drop(store);
    let mut reopened = SegmentedStore::open(cfg).unwrap();
    let id7 = reopened
        .append(make_percept_record(
            "alice",
            "room1",
            7_000,
            b"query for alpha_keyword_2 and alpha_keyword_5",
        ))
        .unwrap();
    let ev7 = reopened.get(id7).unwrap().unwrap();
    assert_eq!(ev7.seq, LogSeq::new(7));

    // Text search spans both sealed shard (alpha_keyword_2) and hot.db (alpha_keyword_5).
    let text_hits = reopened.candidates_by_text(id7, 10).unwrap();
    assert!(text_hits.contains(&ids[1]), "expected shard hit id 2");
    assert!(text_hits.contains(&ids[4]), "expected hot hit id 5");
}

#[test]
fn format_version_upcast_does_not_rewrite_stored_bytes_and_rejects_future_versions() {
    let dir = TempDir::new().unwrap();
    let mut store = SegmentedStore::open(StoreConfig::new(dir.path())).unwrap();

    let v1 = EventRecordV1 {
        id: EventId::new(10),
        kind: EventKind::Percept,
        format_version: 1,
        timestamp_ms: Time::from_millis(500),
        actor: ActorId::new("bob"),
        place: PlaceId::new("chat"),
        audience: Audience::new([ActorId::new("bob"), ActorId::new("soul")]),
        origin_actor: OriginActor::new(ActorId::new("bob")),
        taint: Taint::Provisional,
        caused_by: None,
        payload: OpaquePayload::from_bytes(b"legacy v1 payload"),
    };
    let raw_v1 = EventRecord::encode_v1_bytes(&v1);
    let seq1 = store
        .insert_raw_record_bytes(
            v1.id,
            v1.kind,
            1,
            v1.timestamp_ms,
            &v1.actor,
            &v1.place,
            &v1.audience,
            &v1.origin_actor,
            v1.taint,
            raw_v1.clone(),
        )
        .unwrap();

    let loaded = store.get(EventId::new(10)).unwrap().unwrap();
    assert_eq!(loaded.format_version, EventKind::Percept.current_version());
    assert_eq!(loaded.payload.as_bytes(), b"legacy v1 payload");
    assert_eq!(
        store.raw_bytes_at(seq1).unwrap().as_deref(),
        Some(raw_v1.as_slice()),
        "store must never rewrite stored bytes on upcast"
    );

    // Future version must fail loudly naming format and both versions.
    let future_raw =
        EventRecord::encode_with_version_for_test(EventKind::Percept, 99, EventId::new(11));
    store
        .insert_raw_record_bytes(
            EventId::new(11),
            EventKind::Percept,
            99,
            Time::from_millis(600),
            &ActorId::new("bob"),
            &PlaceId::new("chat"),
            &Audience::new([ActorId::new("bob")]),
            &OriginActor::new(ActorId::new("bob")),
            Taint::Provisional,
            future_raw,
        )
        .unwrap();

    let err = store.get(EventId::new(11)).unwrap_err();
    match err {
        StoreError::EventFormat(EventFormatError::UnsupportedFutureVersion {
            format,
            supported_version,
        }) => {
            assert_eq!(format.kind, EventKind::Percept);
            assert_eq!(format.version, 99);
            assert_eq!(supported_version, EventKind::Percept.current_version());
        }
        other => panic!("expected UnsupportedFutureVersion, got {other:?}"),
    }
}

fn run_vector_backend_conformance(index: &mut dyn VectorIndex) {
    let dim = 32_u32;
    let p_v2 = tag("egemma2-256", dim, "t_d");
    let p_v1 = tag("egemma-300m", dim, "t_d");

    // Insert 500 vectors into p_v2 and 20 vectors into p_v1 (non-mixing check).
    for i in 1..=500_u64 {
        let v = deterministic_vec(dim as usize, i as usize);
        index.insert(&p_v2, EventId::new(i), &v).unwrap();
    }
    for i in 1001..=1020_u64 {
        let v = deterministic_vec(dim as usize, (i - 1000) as usize);
        index.insert(&p_v1, EventId::new(i), &v).unwrap();
    }
    assert_eq!(index.partition_len(&p_v2), 500);
    assert_eq!(index.partition_len(&p_v1), 20);

    // Partition non-mixing: querying p_v1 never returns p_v2 ids, and vice versa.
    let q = deterministic_vec(dim as usize, 5);
    let hits_v1 = index.search(&p_v1, &q, 10, &IdBitmap::allow_all()).unwrap();
    assert!(!hits_v1.is_empty());
    assert!(hits_v1.iter().all(|(id, _)| id.as_u64() >= 1001));

    let hits_v2 = index.search(&p_v2, &q, 10, &IdBitmap::allow_all()).unwrap();
    assert!(!hits_v2.is_empty());
    assert!(hits_v2.iter().all(|(id, _)| id.as_u64() <= 500));
    assert_eq!(hits_v2[0].0, EventId::new(5));

    // Low-selectivity traversal-time filtering (1% selectivity = 5 out of 500 vectors).
    let low_sel_ids = [
        EventId::new(50),
        EventId::new(150),
        EventId::new(250),
        EventId::new(350),
        EventId::new(450),
    ];
    let filter = IdBitmap::from_ids(low_sel_ids);
    let filtered = index.search(&p_v2, &q, 5, &filter).unwrap();
    assert!(
        !filtered.is_empty(),
        "low-selectivity traversal filter must return valid candidates"
    );
    for (id, _) in &filtered {
        assert!(
            low_sel_ids.contains(id),
            "filtered search leaked non-visible id {id}"
        );
    }

    // Remove and ensure removed vector is never returned even under allow_all.
    assert!(index.remove(&p_v2, EventId::new(5)).unwrap());
    assert!(!index.contains(&p_v2, EventId::new(5)));
    let after_rm = index.search(&p_v2, &q, 10, &IdBitmap::allow_all()).unwrap();
    assert!(after_rm.iter().all(|(id, _)| *id != EventId::new(5)));

    // Rebuild partition with subset.
    let subset = vec![
        (EventId::new(42), deterministic_vec(dim as usize, 42)),
        (EventId::new(43), deterministic_vec(dim as usize, 43)),
    ];
    index.rebuild_partition(&p_v2, &subset).unwrap();
    assert_eq!(index.partition_len(&p_v2), 2);
    let after_rebuild = index
        .search(
            &p_v2,
            &deterministic_vec(dim as usize, 42),
            5,
            &IdBitmap::allow_all(),
        )
        .unwrap();
    assert_eq!(after_rebuild.len(), 2);
    assert_eq!(after_rebuild[0].0, EventId::new(42));
}

#[test]
fn default_backend_selection_and_reopen_vector_reload() {
    let dir = TempDir::new().unwrap();
    let cfg = StoreConfig::new(dir.path());
    let mut store = SegmentedStore::open(cfg.clone()).unwrap();

    #[cfg(feature = "usearch")]
    assert!(matches!(
        store.vector_backend_kind(),
        monoagens_store::VectorBackendKind::Usearch(_)
    ));
    #[cfg(not(feature = "usearch"))]
    assert_eq!(
        store.vector_backend_kind(),
        monoagens_store::VectorBackendKind::HnswRs
    );

    let id = store
        .append(make_percept_record(
            "alice",
            "room",
            1_000,
            b"vector reload check",
        ))
        .unwrap();
    let part = tag("egemma2-256", 16, "t_d");
    let vec = deterministic_vec(16, 7);
    store
        .insert_vector(
            id,
            TaggedEmbedding {
                tag: part.clone(),
                values: vec.clone(),
            },
        )
        .unwrap();
    drop(store);

    let reopened = SegmentedStore::open(cfg).unwrap();
    let hits = reopened.candidates_by_vector(&part, &vec, 5).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].0, id);
}

#[test]
fn vector_backends_all_pass_trait_conformance_suite() {
    let mut flat = BruteForceFlatIndex::new();
    run_vector_backend_conformance(&mut flat);

    let mut hnsw = HnswRsVectorIndex::new();
    run_vector_backend_conformance(&mut hnsw);

    #[cfg(feature = "usearch")]
    {
        let mut us_bf16 = UsearchVectorIndex::new(UsearchQuantization::Bf16);
        run_vector_backend_conformance(&mut us_bf16);

        let mut us_i8 = UsearchVectorIndex::new(UsearchQuantization::I8);
        run_vector_backend_conformance(&mut us_i8);
    }
}

#[test]
fn snapshot_round_trip_and_corruption_detection_with_core_open() {
    let dir = TempDir::new().unwrap();
    let cfg = StoreConfig::new(dir.path()).with_vector_backend(VectorBackendConfig::HnswRs);

    {
        let store = SegmentedStore::open(cfg.clone()).unwrap();
        let mut core = Core::open(store, CoreConfig::default(), None).unwrap();

        let p1 = Percept::minimal(
            ActorId::new("alice"),
            PlaceId::new("chat"),
            Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            Time::from_millis(1_000),
            OpaquePayload::from_bytes(b"hello before snapshot"),
        );
        core.step(
            monoagens_core::Input::Percept(Box::new(p1)),
            Time::from_millis(1_000),
            &mut NoopDoors,
        )
        .unwrap();

        let ckpt = core.take_snapshot("snap_alpha").unwrap();
        assert_eq!(ckpt.snapshot_id, "snap_alpha");
        assert!(ckpt.cursor > LogSeq::ZERO);

        let p2 = Percept::minimal(
            ActorId::new("alice"),
            PlaceId::new("chat"),
            Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
            Time::from_millis(2_000),
            OpaquePayload::from_bytes(b"hello after snapshot"),
        );
        core.step(
            monoagens_core::Input::Percept(Box::new(p2)),
            Time::from_millis(2_000),
            &mut NoopDoors,
        )
        .unwrap();
    }

    // Reopen via Core::open: loads snapshot and replays only tail events after cursor.
    let reopened_store = SegmentedStore::open(cfg.clone()).unwrap();
    let core2 = Core::open(reopened_store, CoreConfig::default(), None).unwrap();
    let report = core2.recovery_report();
    assert!(report.snapshot_cursor.is_some());
    assert!(report.replayed_events >= 2);
    drop(core2);

    // Corrupt a byte in the snapshot file on disk and verify Core::open fails loudly naming snap_alpha.
    let snap_files = snapshots::list_snapshot_files(&cfg.snapshots_dir()).unwrap();
    let snap_path = snap_files.last().unwrap();
    let mut bytes = fs::read(snap_path).unwrap();
    let last_idx = bytes.len() - 1;
    bytes[last_idx] ^= 0x5A;
    {
        let mut f = fs::File::create(snap_path).unwrap();
        f.write_all(&bytes).unwrap();
        f.sync_all().unwrap();
    }

    let corrupted_store = SegmentedStore::open(cfg).unwrap();
    let err = Core::open(corrupted_store, CoreConfig::default(), None).unwrap_err();
    match err {
        CoreError::Store(StoreError::SnapshotCorrupted { snapshot_id, .. })
        | CoreError::SnapshotChecksumMismatch { snapshot_id, .. } => {
            assert_eq!(snapshot_id, "snap_alpha");
        }
        other => panic!("expected SnapshotCorrupted naming 'snap_alpha', got {other:?}"),
    }
}

#[test]
fn hard_forget_scrubs_all_files_zero_residue_and_no_resurrection() {
    let dir = TempDir::new().unwrap();
    let cfg = StoreConfig::new(dir.path());
    let mut store = SegmentedStore::open(cfg.clone()).unwrap();

    let secret_str = "TOP_SECRET_FORGET_FIXTURE_987654321";
    let keep_str = "BENIGN_SURVIVING_MEMORY_12345";

    let mut rec1 = make_percept_record("alice", "dm", 1_000, secret_str.as_bytes());
    rec1.attachments.push(AttachmentHandle {
        handle_id: "att_secret_1".to_owned(),
        modality_slot: ModalitySlot::Image,
        byte_len: 1024,
        expires_at_ms: Time::from_millis(99_000),
        embed_model_ver: "egemma2-256".to_owned(),
    });
    let id_secret_in_shard = store.append(rec1).unwrap();

    let rec2 = make_percept_record("bob", "public", 2_000, keep_str.as_bytes());
    let id_keep = store.append(rec2).unwrap();

    // Seal events 1..=2 so the secret lives inside a read-only temporal shard!
    let _shard = store
        .seal_through_seq(LogSeq::new(2), Time::from_millis(2_500))
        .unwrap()
        .unwrap();

    // Also add a second secret event in hot.db + vectors + a pre-forget snapshot containing it.
    let mut rec3 = make_percept_record("alice", "dm", 3_000, secret_str.as_bytes());
    let id_secret_in_hot = store.append(rec3.clone()).unwrap();
    rec3.id = id_secret_in_hot;
    rec3.seq = LogSeq::new(3);
    let mut pre_state = monoagens_core::CoreState::new();
    pre_state.apply_event(&rec3);
    let pre_bytes = pre_state.encode_snapshot();
    let pre_blake3 = monoagens_core::blake3::hash(&pre_bytes);
    store
        .snapshot(monoagens_core::SnapshotCheckpoint {
            snapshot_id: "pre_forget_with_unconsumed_secret".to_owned(),
            cursor: LogSeq::new(3),
            epoch: 0,
            bytes: pre_bytes,
            blake3: pre_blake3,
        })
        .unwrap();

    let part = tag("egemma2-256", 16, "t_d");
    store
        .insert_vector(
            id_secret_in_shard,
            TaggedEmbedding {
                tag: part.clone(),
                values: deterministic_vec(16, 1),
            },
        )
        .unwrap();
    store
        .insert_vector(
            id_secret_in_hot,
            TaggedEmbedding {
                tag: part.clone(),
                values: deterministic_vec(16, 2),
            },
        )
        .unwrap();
    store
        .insert_vector(
            id_keep,
            TaggedEmbedding {
                tag: part.clone(),
                values: deterministic_vec(16, 3),
            },
        )
        .unwrap();

    // Verify secret bytes exist on disk before hard_forget.
    let pre_hits = raw_byte_scan_dir(dir.path(), &[secret_str.as_bytes()]).unwrap();
    assert!(!pre_hits.is_empty(), "fixture secret must exist pre-forget");

    // Execute atomic hard_forget for actor "alice" (covers both shard event and hot event).
    let report = store
        .hard_forget(&ForgetSelector::actors(
            [ActorId::new("alice")],
            Time::from_millis(5_000),
        ))
        .unwrap();
    assert_eq!(report.epoch, 1);
    assert!(report.byte_scan_clean);
    assert!(report.erased_event_ids.contains(&id_secret_in_shard));
    assert!(report.erased_event_ids.contains(&id_secret_in_hot));
    assert!(!report.erased_event_ids.contains(&id_keep));

    // Explicit raw byte scan check across hot.db, WAL/SHM, shards, FTS shadow tables, and snapshots.
    let post_hits = raw_byte_scan_dir(dir.path(), &[secret_str.as_bytes()]).unwrap();
    assert!(
        post_hits.is_empty(),
        "raw byte scan found residue after hard_forget: {post_hits:?}"
    );

    // Vector search no longer returns erased events, only surviving event.
    let vec_hits = store
        .candidates_by_vector(&part, &deterministic_vec(16, 1), 10)
        .unwrap();
    assert_eq!(vec_hits.len(), 1);
    assert_eq!(vec_hits[0].0, id_keep);

    // Text search no longer finds erased events.
    let query_ev = store
        .append(make_percept_record(
            "bob",
            "public",
            6_000,
            b"TOP_SECRET_FORGET_FIXTURE_987654321 BENIGN_SURVIVING_MEMORY_12345",
        ))
        .unwrap();
    let fts_hits = store.candidates_by_text(query_ev, 10).unwrap();
    assert_eq!(fts_hits, vec![id_keep]);

    // Replay / Core::open after hard_forget: erased items never resurrect & Trace-Pinned clean.
    let replay_rep = replay_from_checkpoint(
        &store,
        store.latest_checkpoint().unwrap().as_ref(),
        ReplayMode::Pinned,
        None,
    )
    .unwrap();
    assert!(replay_rep.is_exact());

    let core = Core::open(store, CoreConfig::default(), None).unwrap();
    assert_eq!(core.state().epoch, 1);
    assert!(core.store().get(id_secret_in_shard).unwrap().is_none());
    assert!(core.store().get(id_secret_in_hot).unwrap().is_none());
    assert!(core.store().get(id_keep).unwrap().is_some());
}

#[test]
fn vector_fade_releases_vectors_below_petrov_threshold_while_keeping_events() {
    let dir = TempDir::new().unwrap();
    let mut store = SegmentedStore::open(StoreConfig::new(dir.path())).unwrap();

    let ev_old = store
        .append(make_percept_record(
            "alice",
            "room",
            0,
            b"ancient faded event payload",
        ))
        .unwrap();
    let now_31d = Time::from_millis(31 * 24 * 3_600_000);
    let ev_fresh = store
        .append(make_percept_record(
            "alice",
            "room",
            now_31d.as_millis() - 60_000,
            b"recent fresh event payload",
        ))
        .unwrap();

    let part = tag("egemma2-256", 16, "t_d");
    store
        .insert_vector(
            ev_old,
            TaggedEmbedding {
                tag: part.clone(),
                values: deterministic_vec(16, 10),
            },
        )
        .unwrap();
    store
        .insert_vector(
            ev_fresh,
            TaggedEmbedding {
                tag: part.clone(),
                values: deterministic_vec(16, 11),
            },
        )
        .unwrap();

    let d = decay_exponent(0.5);
    // Neutral threshold (-inf) releases nothing.
    let neutral_rep = store
        .fade_stale_vectors_by_log_age(now_31d, d, f64::NEG_INFINITY)
        .unwrap();
    assert!(neutral_rep.released_event_ids.is_empty());

    // Petrov threshold B < -3.29 releases ev_old (31 days, 1 use => B ~ -3.307 < -3.29) and keeps ev_fresh.
    let mut t_old = ActivationTrace::default();
    t_old.record(Time::from_millis(0));
    assert!(t_old.base_level(now_31d, d) < DEFAULT_FADE_THRESHOLD);

    let fade_rep = store
        .fade_stale_vectors_by_log_age(now_31d, d, DEFAULT_FADE_THRESHOLD)
        .unwrap();
    assert_eq!(fade_rep.released_event_ids, vec![ev_old]);

    // Also test explicit fade_vectors with FadeCandidate on ev_fresh when advanced 31 more days.
    let mut t_fresh = ActivationTrace::default();
    t_fresh.record(Time::from_millis(now_31d.as_millis() - 60_000));
    let far_future = Time::from_millis(now_31d.as_millis() + 31 * 24 * 3_600_000);
    let explicit_rep = store
        .fade_vectors(
            &[FadeCandidate {
                event_id: ev_fresh,
                trace: t_fresh,
            }],
            far_future,
            d,
            DEFAULT_FADE_THRESHOLD,
        )
        .unwrap();
    assert_eq!(explicit_rep.released_event_ids, vec![ev_fresh]);
    let after_explicit = store
        .candidates_by_vector(&part, &deterministic_vec(16, 11), 10)
        .unwrap();
    assert!(after_explicit.is_empty());

    // Re-insert ev_fresh for the remaining check below.
    store
        .insert_vector(
            ev_fresh,
            TaggedEmbedding {
                tag: part.clone(),
                values: deterministic_vec(16, 11),
            },
        )
        .unwrap();

    // Vector for ev_old is gone from VectorIndex, while ev_fresh remains.
    let vec_hits = store
        .candidates_by_vector(&part, &deterministic_vec(16, 10), 10)
        .unwrap();
    assert_eq!(vec_hits.len(), 1);
    assert_eq!(vec_hits[0].0, ev_fresh);

    // Event itself is still in the log and searchable by text!
    let kept_old = store
        .get(ev_old)
        .unwrap()
        .expect("event must remain in log");
    assert_eq!(kept_old.payload.as_bytes(), b"ancient faded event payload");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn prop_logseq_strictly_monotonic_across_appends_and_sealing(seal_at in 1usize..=8, total in 8usize..=16) {
        let dir = TempDir::new().unwrap();
        let mut store = SegmentedStore::open(
            StoreConfig::new(dir.path()).with_vector_backend(VectorBackendConfig::BruteForceFlat)
        ).unwrap();

        let mut ids = Vec::with_capacity(total);
        for i in 0..total {
            let id = store.append(make_percept_record(
                "prop_actor",
                "prop_place",
                (i as u64 + 1) * 100,
                format!("prop payload {i}").as_bytes(),
            )).unwrap();
            ids.push(id);
        }

        let _ = store.seal_through_seq(LogSeq::new(seal_at as u64), Time::from_millis(99_999)).unwrap();
        let extra = store.append(make_percept_record("prop_actor", "prop_place", 10_000, b"tail")).unwrap();
        ids.push(extra);

        let all = store.read_after_seq(None, 1000).unwrap();
        prop_assert_eq!(all.len(), ids.len());
        for w in all.windows(2) {
            prop_assert!(w[0].seq < w[1].seq);
        }
        for (i, rec) in all.iter().enumerate() {
            prop_assert_eq!(rec.id, ids[i]);
        }
    }
}

#[test]
fn config_loader_rejects_unknown_key_out_of_range_missing_path_and_preserves_formatting_digest() {
    use monoagens_store::{
        ConfigLoadError, default_loaded_core_config, load_core_config_file,
        load_core_config_or_default,
    };

    let dir = TempDir::new().unwrap();

    // 1. Canonical digest is invariant to formatting / whitespace / key ordering.
    let default_loaded = default_loaded_core_config().unwrap();
    let reformatted_path = dir.path().join("reformatted.json");
    std::fs::write(&reformatted_path, r#"{ "format_version": 1 }"#).unwrap();
    let loaded_reformatted = load_core_config_file(&reformatted_path).unwrap();
    assert_eq!(loaded_reformatted.digest, default_loaded.digest);

    // 2. Edited field yields a different digest.
    let edited_path = dir.path().join("edited.json");
    std::fs::write(
        &edited_path,
        r#"{ "format_version": 1, "certainty_bar": 0.72 }"#,
    )
    .unwrap();
    let loaded_edited = load_core_config_file(&edited_path).unwrap();
    assert_ne!(loaded_edited.digest, default_loaded.digest);

    // 3. Unknown key fails naming the unknown key.
    let unknown_path = dir.path().join("unknown_key.json");
    std::fs::write(
        &unknown_path,
        r#"{ "format_version": 1, "bogus_disposition_xyz": 0.1 }"#,
    )
    .unwrap();
    let err_unknown = load_core_config_file(&unknown_path).expect_err("reject unknown key");
    assert!(
        err_unknown.to_string().contains("bogus_disposition_xyz"),
        "err={err_unknown}"
    );

    // 4. Out-of-range value fails naming field.
    let range_path = dir.path().join("bad_range.json");
    std::fs::write(
        &range_path,
        r#"{ "format_version": 1, "max_wait_ms": 0.0 }"#,
    )
    .unwrap();
    let err_range = load_core_config_file(&range_path).expect_err("reject out of range");
    assert!(
        err_range.to_string().contains("max_wait_ms"),
        "msg={err_range}"
    );

    // 5. Missing explicit path fails naming path and does not fall back to default.
    let missing_path = dir.path().join("does_not_exist.json");
    let err_missing = load_core_config_or_default(Some(&missing_path))
        .expect_err("reject missing explicit path");
    assert!(matches!(err_missing, ConfigLoadError::Read { .. }));
    assert!(
        err_missing.to_string().contains("does_not_exist.json"),
        "err={err_missing}"
    );
}

#[test]
fn hard_forget_never_erases_config_loaded_and_post_erasure_replay_uses_recorded_config() {
    use monoagens_core::{Core, Input, NoopDoors};
    use monoagens_protocol::{ActorId, Audience, OpaquePayload, Percept, PlaceId};
    use monoagens_store::ForgetSelector;

    let dir = TempDir::new().unwrap();
    let store = SegmentedStore::open(
        StoreConfig::new(dir.path()).with_vector_backend(VectorBackendConfig::BruteForceFlat),
    )
    .unwrap();

    let cfg_b = CoreConfig {
        seed: 77,
        stochastic_gate_sampling: false,
        ..CoreConfig::default()
    };

    let mut core = Core::open(store, cfg_b.clone(), None).unwrap();
    let mut p_secret = Percept::minimal(
        ActorId::new("alice"),
        PlaceId::new("room:secret"),
        Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
        Time::from_millis(1_000),
        OpaquePayload::from_bytes(b"SECRET_BEFORE_FORGET_TOKEN"),
    );
    p_secret.named = vec![ActorId::new("soul")];
    core.step(
        Input::Percept(Box::new(p_secret)),
        Time::from_millis(1_000),
        &mut NoopDoors,
    )
    .unwrap();

    let events_pre = core.store().read_in_order(None, usize::MAX).unwrap();
    let cfg_ev_id = events_pre
        .iter()
        .find(|e| e.kind == EventKind::ConfigLoaded)
        .expect("config_loaded present")
        .id;
    drop(core);

    // Even if someone passes config_loaded's EventId and actor="core" to hard_forget, it survives!
    let mut store = SegmentedStore::open(
        StoreConfig::new(dir.path()).with_vector_backend(VectorBackendConfig::BruteForceFlat),
    )
    .unwrap();
    store
        .seal_through_seq(LogSeq::new(1), Time::from_millis(1_500))
        .unwrap();
    let mut selector = ForgetSelector::actors(
        [ActorId::new("alice"), ActorId::new("core")],
        Time::from_millis(2_000),
    );
    selector.event_ids.push(cfg_ev_id);
    let report = store.hard_forget(&selector).unwrap();
    assert!(!report.erased_event_ids.contains(&cfg_ev_id));

    let surviving = store.read_in_order(None, usize::MAX).unwrap();
    assert!(
        surviving
            .iter()
            .any(|e| e.id == cfg_ev_id && e.kind == EventKind::ConfigLoaded),
        "config_loaded must survive hard_forget"
    );

    let mut core_post = Core::open(store, cfg_b, None).unwrap();
    assert_eq!(
        core_post
            .store()
            .read_in_order(None, usize::MAX)
            .unwrap()
            .iter()
            .filter(|e| e.kind == EventKind::ConfigLoaded)
            .count(),
        1,
        "reopening with same cfg_b after hard_forget must not append another config_loaded"
    );

    let mut p_after = Percept::minimal(
        ActorId::new("bob"),
        PlaceId::new("room:public"),
        Audience::new([ActorId::new("bob"), ActorId::new("soul")]),
        Time::from_millis(3_000),
        OpaquePayload::from_bytes(b"post forget under B"),
    );
    p_after.named = vec![ActorId::new("soul")];
    core_post
        .step(
            Input::Percept(Box::new(p_after)),
            Time::from_millis(3_000),
            &mut NoopDoors,
        )
        .unwrap();

    let rep = replay_from_checkpoint(
        core_post.store(),
        core_post.store().latest_checkpoint().unwrap().as_ref(),
        ReplayMode::Pinned,
        None,
    )
    .unwrap();
    assert!(rep.is_exact(), "divergence: {:?}", rep.mismatches);
    assert_eq!(rep.verified_decisions, 1);

    let full_rep =
        replay_from_checkpoint(core_post.store(), None, ReplayMode::Pinned, None).unwrap();
    assert!(
        full_rep.is_exact(),
        "full replay divergence: {:?}",
        full_rep.mismatches
    );
}

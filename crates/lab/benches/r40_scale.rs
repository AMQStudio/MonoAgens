//! Criterion microbenchmarks (`R-40`) for `Core::open` across `10^5`/`10^6`/`10^7` events
//! and filtered `VectorIndex` search at `1%` selectivity across `usearch`, `hnsw_rs`, and `Flat`.

use std::hint::black_box;

use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};
use monoagens_core::{
    Core, CoreConfig, EventKind, EventRecord, LogSeq, MemStore, NumericFeatures,
    SnapshotCheckpoint, Store, StoreError,
};
use monoagens_protocol::{
    ActorId, Audience, EmbeddingTag, EventId, OpaquePayload, OriginActor, PlaceId, Taint, Time,
};
use monoagens_store::{
    BruteForceFlatIndex, HnswRsVectorIndex, IdBitmap, UsearchQuantization, UsearchVectorIndex,
    VectorIndex,
};

#[derive(Clone)]
struct BenchSnapshotStore {
    ckpt: SnapshotCheckpoint,
    tail: Vec<EventRecord>,
}

impl Store for BenchSnapshotStore {
    fn append(&mut self, mut rec: EventRecord) -> Result<EventId, StoreError> {
        let next = self
            .tail
            .last()
            .map_or(self.ckpt.cursor.as_u64() + 1, |r| r.seq.as_u64() + 1);
        rec.id = EventId::new(next);
        rec.seq = LogSeq::new(next);
        self.tail.push(rec);
        Ok(EventId::new(next))
    }

    fn read_in_order(
        &self,
        _after: Option<EventId>,
        limit: usize,
    ) -> Result<Vec<EventRecord>, StoreError> {
        Ok(self.tail.iter().take(limit).cloned().collect())
    }

    fn read_after_seq(
        &self,
        after: Option<LogSeq>,
        limit: usize,
    ) -> Result<Vec<EventRecord>, StoreError> {
        let start = after.map_or(0, |s| s.as_u64());
        Ok(self
            .tail
            .iter()
            .filter(|r| r.seq.as_u64() > start)
            .take(limit)
            .cloned()
            .collect())
    }

    fn get(&self, id: EventId) -> Result<Option<EventRecord>, StoreError> {
        Ok(self.tail.iter().find(|r| r.id == id).cloned())
    }

    fn snapshot(&mut self, checkpoint: SnapshotCheckpoint) -> Result<(), StoreError> {
        self.ckpt = checkpoint;
        Ok(())
    }

    fn latest_checkpoint(&self) -> Result<Option<SnapshotCheckpoint>, StoreError> {
        Ok(Some(self.ckpt.clone()))
    }

    fn candidates_by_vector(
        &self,
        _tag: &EmbeddingTag,
        _query_vector: &[f32],
        _limit: usize,
    ) -> Result<Vec<(EventId, f32)>, StoreError> {
        Ok(Vec::new())
    }

    fn candidates_by_text(
        &self,
        _query_event_id: EventId,
        _limit: usize,
    ) -> Result<Vec<EventId>, StoreError> {
        Ok(Vec::new())
    }

    fn release_vector(&mut self, _event_id: EventId) -> Result<(), StoreError> {
        Ok(())
    }
}

fn bench_core_open(c: &mut Criterion) {
    let mut seed_core = Core::open(
        MemStore::new(),
        CoreConfig {
            seed: 42,
            ..CoreConfig::default()
        },
        None,
    )
    .unwrap();
    let base_ckpt = seed_core.take_snapshot("bench-base").unwrap();
    let base_rec = EventRecord::new_root(
        EventKind::Percept,
        Time::from_millis(1_000),
        ActorId::new("alice"),
        PlaceId::new("room:bench"),
        Audience::new([ActorId::new("alice"), ActorId::new("soul")]),
        OriginActor::new(ActorId::new("alice")),
        Taint::Trusted,
        NumericFeatures::default(),
        OpaquePayload::from_bytes(b"bench".to_vec()),
        Vec::new(),
        None,
    );

    let mut group = c.benchmark_group("r40_core_open");
    for depth in [100_000_u64, 1_000_000, 10_000_000] {
        let tail_start = depth - 10;
        let tail: Vec<EventRecord> = (1..=10)
            .map(|offset| {
                let mut r = base_rec.clone();
                r.id = EventId::new(tail_start + offset);
                r.seq = LogSeq::new(tail_start + offset);
                r.timestamp_ms = Time::from_millis(500_000 + offset * 100);
                r
            })
            .collect();
        let store_template = BenchSnapshotStore {
            ckpt: SnapshotCheckpoint {
                snapshot_id: format!("ckpt-{depth}"),
                cursor: LogSeq::new(tail_start),
                epoch: 0,
                bytes: base_ckpt.bytes.clone(),
                blake3: base_ckpt.blake3,
            },
            tail,
        };
        group.bench_with_input(
            BenchmarkId::from_parameter(depth),
            &store_template,
            |b, tmpl| {
                b.iter_batched(
                    || tmpl.clone(),
                    |st| {
                        let core = Core::open(
                            st,
                            CoreConfig {
                                seed: 42,
                                ..CoreConfig::default()
                            },
                            None,
                        )
                        .unwrap();
                        black_box(core.recovery_report().replayed_events)
                    },
                    BatchSize::SmallInput,
                );
            },
        );
    }
    group.finish();
}

fn bench_filtered_vector_1pct(c: &mut Criterion) {
    let dim = 256_usize;
    let total = 5_000_usize;
    let tag = EmbeddingTag {
        embed_model_ver: "egemma2-256".to_owned(),
        dim_d: dim as u32,
        prompt_regime_id: "t_d".to_owned(),
    };
    let make_vec = |seed: usize| -> Vec<f32> {
        let mut v = vec![0.0_f32; dim];
        let mut x = (seed as u64)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        for slot in &mut v {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let u = ((x >> 32) as f32) / (u32::MAX as f32);
            *slot = u * 2.0 - 1.0;
        }
        let norm = v.iter().map(|s| s * s).sum::<f32>().sqrt();
        if norm > 0.0 {
            for s in &mut v {
                *s /= norm;
            }
        }
        v
    };

    let mut flat = BruteForceFlatIndex::new();
    let mut usearch_i8 = UsearchVectorIndex::new(UsearchQuantization::I8);
    let mut usearch_bf16 = UsearchVectorIndex::new(UsearchQuantization::Bf16);
    let mut hnsw_rs = HnswRsVectorIndex::new();

    for i in 1..=total {
        let eid = EventId::new(i as u64);
        let vec = make_vec(i);
        flat.insert(&tag, eid, &vec).unwrap();
        usearch_i8.insert(&tag, eid, &vec).unwrap();
        usearch_bf16.insert(&tag, eid, &vec).unwrap();
        hnsw_rs.insert(&tag, eid, &vec).unwrap();
    }

    let visible_ids: Vec<EventId> = (1..=total)
        .step_by(100)
        .map(|i| EventId::new(i as u64))
        .collect();
    let bitmap = IdBitmap::from_ids(visible_ids);
    let query = make_vec(1);

    let mut group = c.benchmark_group("r40_vector_1pct");
    group.bench_function("BruteForceFlat", |b| {
        b.iter(|| black_box(flat.search(&tag, &query, 10, &bitmap).unwrap()))
    });
    group.bench_function("Usearch_I8", |b| {
        b.iter(|| black_box(usearch_i8.search(&tag, &query, 10, &bitmap).unwrap()))
    });
    group.bench_function("Usearch_Bf16", |b| {
        b.iter(|| black_box(usearch_bf16.search(&tag, &query, 10, &bitmap).unwrap()))
    });
    group.bench_function("HnswRs", |b| {
        b.iter(|| black_box(hnsw_rs.search(&tag, &query, 10, &bitmap).unwrap()))
    });
    group.finish();
}

criterion_group!(benches, bench_core_open, bench_filtered_vector_1pct);
criterion_main!(benches);

# 009 — Core: trust, fading and forgetting (Track J)
Status: todo | Wave: 2c | Depends on: 007
Read first: [core.md](../core.md) §7–8
May touch: `monoagens/memory.py`, `monoagens/store.py` (forget paths only), `tests/`

## Goal
Attribution, reliability and the three ways of letting go.

## Tasks
- [ ] T1 Attribution and reliability (α/β) per domain; a non-owner never supersedes an owner fact.
- [ ] T2 Fade with a fixed threshold during sleep.
- [ ] T3 Soft forgetting.
- [ ] T4 Hard forgetting along `derived_from`, leaving only `{"forgotten": true}` and metadata.
- [ ] T5 (deferred until real logs show slow rebuilds) Snapshots of `state` every `snapshot_every_events`; rebuild = latest snapshot + later events; hard forgetting invalidates snapshots that contain forgotten data.

## Done when
A stranger cannot override an owner fact; hard forget leaves only `{"forgotten": true}` and metadata, also after a rebuild from snapshots.

## Notes

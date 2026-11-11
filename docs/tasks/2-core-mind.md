# Stage 2 — Core: depth of mind

After launch. Order and status: [README](README.md). Each section is one task; work only inside
yours.

---

## 006 — Core: inner thought and self-model (Track F)
Status: todo | Depends on: 010
Read first: [core.md](../core.md) §6, §8
May touch: `monoagens/pfc.py`, `tests/test_pfc.py`

**Goal.** The PFC thinks before self-selected speech and knows itself honestly.

- [ ] T1 `think(thought, motivation 1..5, confidence 0..1)` before any communicative `act` when self-selecting; gate on `speaking_bar` / `certainty_bar`.
- [ ] T2 Stable prompt: principles, honesty principles (core.md §8), knowledge cutoff from per-model config.
- [ ] T3 `<self>` from the registries, closed-world, including the capabilities of every running organ; person cards of people present.

**Done when.** Proactive replies always have a recorded thought; "what can you do?" matches
`<self>`; a false claim about the past is not confirmed, and a missing record is not called
proof.

**Notes.**

---

## 007 — Core: revise, beliefs, lessons and procedures (Track H)
Status: todo | Depends on: 002
Read first: [core.md](../core.md) §6–7, [core-math.md](../core-math.md) §7, §14
May touch: `monoagens/memory.py` (sleep part), `tests/test_memory.py`

**Goal.** Self-correction and knowing vs misremembering.

- [ ] T1 `revise` sets `valid_to` / `superseded_by`; nothing deleted.
- [ ] T2 `basis` (observed | told | inferred) on memories; sleep gist is `inferred`.
- [ ] T3 Source over gist: events win; the summary is revised.
- [ ] T4 Merge episodes only when people, place and time agree.
- [ ] T5 `guard` beliefs checked in sleep; contradictions as precision-weighted prediction errors.
- [ ] T6 Lessons and procedures with α/β credence; generic ones go to layer 2 and are used in every body.

**Done when.** A correction supersedes the old memory; a contradicted belief is revised at the
next sleep; two similar episodes with different people stay apart; a lesson learned in one body
is used in another.

**Notes.**

---

## 008 — Core: wake gate and free-text conditions (Track I)
Status: todo | Depends on: 005
Read first: [core.md](../core.md) §5, [core-math.md](../core-math.md) §6, §15
May touch: `monoagens/expect.py`, `tests/test_expect.py`

**Goal.** Cheap waking for Expectations; cosine filters, a classifier judges.

- [ ] T1 Wake gate: still relevant? stale? (quiet hours stay in the owner policy).
- [ ] T2 Free text: cosine below `lo` ignored; every candidate → one recorded classifier call.
- [ ] T3 `hi` band opens only by the rule of three; `lo` learning within bounds and a minimum gray width.

**Done when.** A fulfilled promise wakes no one; a question on the same topic never resolves a
promise.

**Notes.**

---

## 009 — Core: trust, fading and forgetting (Track J)
Status: todo | Depends on: 007
Read first: [core.md](../core.md) §7–8
May touch: `monoagens/memory.py`, `monoagens/forget.py`, `tests/`

**Goal.** Attribution, reliability and the three ways of letting go. The fade and hard-forget
paths exist since 001 T4; this task adds the policy around them.

- [ ] T1 Attribution and reliability (α/β) per domain; a non-owner never supersedes an owner fact.
- [ ] T2 Fade with a fixed threshold during sleep.
- [ ] T3 Soft forgetting.
- [ ] T4 Hard forgetting also removes `share.granted` grants for the forgotten content.
- [ ] T5 (deferred until real logs show slow rebuilds) Snapshots of `state` every `snapshot_every_events`; rebuild = latest snapshot + later events; hard forgetting invalidates snapshots that contain forgotten data.

**Done when.** A stranger cannot override an owner fact; hard forget leaves only
`{"forgotten": true}` and metadata, also after a rebuild.

**Notes.**

---

## 017 — Core: born from a prior
Status: deferred (until weeks of real logs) | Depends on: 001
Read first: [core.md](../core.md) §9.3–9.4
May touch: `monoagens/config.py`, `monoagens/prior.py`, `prior/`, `tests/`

**Goal.** Load a prior at birth with the loading rules.

- [ ] T1 Prior loader with the loading rules (embedder, LLM family, format version).
- [ ] T2 Default prior in the repository.

**Done when.** A fresh soul born from the default prior passes the replay suite with empty
memory.

**Notes.**

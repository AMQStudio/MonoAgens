# ADR-0016: Self-Model, Activity Stream, and Human-Like Memory Layers — Remember What I Just Did, Everywhere, Without Re-Reading Threads

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** The Soul forgets in one room what it just did in another and must call `thread_history` to find out. The only cross-room self-knowledge is a "Recent actions" block that lists tool calls (not conversations), is global, and is protected only by a prompt sentence. This ADR gives the Soul a layered memory modelled on human memory, with an autobiographical activity stream at its center, rendered per audience under the privacy rules of ADR-0012.
- **Supersedes:** — (additive; replaces the `actions` ledger block in code, which no ADR decided)
- **Related:** ADR-0011 (Soul area), ADR-0012 (envelope and audience filtering), ADR-0013 (receipts, open flows), ADR-0002 (memory), ADR-0005 (jobs)
- **Implementation:** draft

---

## 1. Context

- `monoagens/engine/memory/ledger.py:72-101` keeps the last 200 rows of `actions` and renders
  ten as "Recent actions you performed", tool kind plus target plus summary, with the rule
  "internal: never reveal who requested them" enforced only by prompt text.
- Conversational activity (what the Soul said, to whom, in which room, what it promised) is not
  part of any self-knowledge block. In the observed DM, the Soul could describe its group
  activity only after reading the group archive with `thread_history`, a tool round per
  question.
- Episodes (`memory/episodes.py`) and memories exist, but they are consolidated later and are
  not a short-term record of the Soul's own recent life.
- Commitments ("edit access postponed to Monday") live only in transcripts.

## 2. Decision drivers

- One continuous self: what the Soul did a minute ago anywhere is part of who it is now.
- No extra tool round or LLM call to remember its own recent activity.
- Privacy by construction: another room's activity is rendered only to an audience allowed to
  know it, at the allowed level of detail.
- Bounded cost: short-term memory decays and consolidates, like human memory.

## 3. Options considered

1. **Keep `thread_history` lookups.** Rejected: a tool round per recollection, and the Soul does
   not know it should look.
2. **Inject recent transcripts of all rooms.** Rejected: token cost and cross-room leakage.
3. **Autobiographical activity stream plus layered memory, rendered per audience (chosen).**

## 4. Decision

### 4.1 Memory layers

| Layer | Human analogue | Store | Lifetime | Written by |
|---|---|---|---|---|
| Working memory | what is in mind now | assembled per turn | one turn | context assembly |
| Activity stream | "what I just did" (autobiographical short-term) | `activity` | hours to days, decays | harness at turn end |
| Open loops | prospective memory ("I promised to...") | `activity` rows with status `open`, linked to flows and tasks | until closed or expired | harness and flows (ADR-0013) |
| Episodes | episodic memory ("that afternoon in the group") | `episodes` | long | consolidation job |
| Semantic memory | facts about people and the world | `memories` | long, retention policy | `MemoryStore` |
| Procedural memory | skills | `skills` | long | ladder job |

### 4.2 Activity stream

One row per Soul turn and per mechanism event, across all rooms:

- when, room (`Address`), counterpart person ids, what it did (`replied`, `tool`, `raised
  requirement`, `decided`, `declined`), outcome, a one-line gist, open-loop status and due time,
  and a privacy level (the highest level of what the gist covers, per ADR-0012).
- Version 1: the gist is deterministic (tool names, receipt data, first sentence of the reply).
  Later, if scenarios show it is too thin: a small structured field of the main model answer,
  with the deterministic gist as fallback. No extra LLM call either way.
- Receipts from ADR-0013 (for example "Muqsith ran /approve #5") enter with their `actor`, so
  the Soul never claims another person's action.
- The `actions` table is folded into the stream.

### 4.3 Rendering per audience

Every turn renders a "What I have been doing" block, filtered by the same audience policy as
ADR-0012:

| Activity location versus current audience | Rendered detail |
|---|---|
| Same room | full gist |
| Another room the current sender is proven to be a member of | gist, labelled with the room name |
| Another room the sender is not in | meta only ("I was active in another conversation"), no names, no content |
| Privacy above the audience cap | omitted |

Open loops relevant to the current counterpart are always rendered (subject to the same rules).
`thread_history` remains for details beyond the gist.

### 4.4 Consolidation and forgetting

- A periodic core job consolidates aged activity into episodes and, through `MemoryStore`, into
  semantic memories; consolidated rows decay out of the stream.
- Salience (open loops, failures not yet superseded, recent counterpart) decides what survives
  in the rendered block under its token budget.
- Expression plugins may observe the stream (post-turn hook, ADR-0014) but never write to it.

### 4.5 Forgetting: cascade, tombstone, and no re-learning

Today `memory_tombstones` (content-free: kind, level, scope, content hash, reason, time) is
written on deletion, but nothing reads it: capture never checks it, so a forgotten fact can be
learned again from the next message, and episodes or activity rows that mention it survive.

- **Cascade.** A subject veto or a forget request removes the content from every layer of §4:
  `memories` and their embeddings, `memory_index`, episodes and their distilled text, activity
  rows and their gists, open loops, and consolidation inputs. Message rows in `messages` keep
  existing as the transcript of record but are excluded from capture and consolidation.
- **Tombstone, not silence.** One marker per forgotten item remains: subject person id, scope,
  layers purged, requested by, reason (`veto`, `forget_request`, `retention`), time, and a keyed
  hash of the normalized content (HMAC with a deployment secret, so the hash cannot be
  dictionary-checked outside the deployment). No content, no summary.
- **No re-learning.** Capture and consolidation check tombstones before writing; a match blocks
  the write. Only the subject, by explicitly sharing the fact again after the tombstone, can
  create a new memory (new consent).
- **The Soul knows it forgot.** For the current counterpart, the self-model renders veto and
  forget-request markers as counts and dates ("you were asked to forget 2 items about this
  person on 2026-11-09"), never content, so the Soul can answer honestly ("I have forgotten
  that at your request"). Retention-expiry tombstones are not rendered.

## 5. Configuration

- `engine.activity.render_items` (default 12), `engine.activity.render_chars`,
  `engine.activity.keep_hours` (default 72), `engine.activity.consolidate_every`.

## 6. Metrics and gates

- Recollection without tools: in golden scenarios, asking in a DM "what did you just do in group
  X" (sender is a member) is answered with zero `thread_history` calls.
- Leak gate: the same question from a non-member yields meta only (structural assertion).
- Block size stays within `render_chars`.

## 7. Acceptance criteria

- After replying in group A, a turn in group B (shared member) or DM renders that activity.
- A command run by a user appears with that user as actor.
- An open loop ("check again on Monday") appears when its counterpart talks to the Soul.
- Consolidated activity is findable as an episode or memory after it leaves the stream.
- After a forget request, no layer returns the content, the same fact restated by someone else
  is not captured again, and the Soul can say that it forgot at the subject's request.

## 8. Risks and mitigations

- **Gist reveals too much.** Mitigation: gist privacy level computed by the deterministic
  pipeline; rendering follows §4 rules; tests per row of the table.
- **Model skips the structured gist.** Mitigation: deterministic fallback.
- **Stream grows.** Mitigation: `keep_hours`, consolidation, and per-turn budget.
- **Paraphrase escapes the exact-hash tombstone.** Mitigation: the keyed hash catches restated
  text; a semantic fingerprint (embedding kept in the tombstone) would catch paraphrases but
  keeps a content-derived vector. Open decision for the owner; default is hash only.
- **Forgotten content resurfaces through consolidation.** Mitigation: consolidation reads
  tombstones first; a test covers forget, then consolidate, then recall.

## 9. Interaction with other ADRs

- ADR-0012: audience filtering and privacy labels are reused unchanged.
- ADR-0013: receipts and open flows feed the stream and the open-loop layer.
- ADR-0002: semantic memory and the single write port stay; consolidation writes through it.
- ADR-0011: implements the self-model of the Soul area.

## 10. Code references

- `monoagens/engine/memory/ledger.py` (`ActionLedger.recent_actions`, `actions_context`).
- `monoagens/engine/schema/base.py` (`actions`), `monoagens/engine/memory/episodes.py`.
- `monoagens/engine/schema/memory.py` (`memory_tombstones`), `monoagens/engine/memory/maintenance.py`,
  `monoagens/engine/memory/candidates.py`, `monoagens/engine/memory/gate.py` (capture path).
- `monoagens/engine/turn/prompts.py` (Recent actions instruction),
  `monoagens/engine/tools/thread_history.py`.

*This ADR lives at `docs/v0.1.0/adr/engine/0016-self-model-activity-stream-and-memory-layers.md`.*

# Proposal

## Why

MonoAgens exists today only as two documents: `ARCHITECTURE.md` (parts and invariants) and
`RESEARCH.md` (formulas and sources). There is no code, so no claim about the soul can be
measured. Earlier attempts (Python, PostgreSQL, feature-first) grew wide before the mind was
proven, and their tests were slow.

This change builds the thinnest slice that runs the whole hourglass end to end, in Rust, so
that every later mechanism is added against a working loop and a Lab that can measure it.

## What Changes

- Add a Rust workspace with a pure, IO-free core, so the core's tests run in milliseconds and
  the core can later move to small devices.
- Add the protocol: the six facts (actor, place, audience, event, capability, receipt) as
  versioned, serializable types.
- Add an append-only event log with "record first, then act", projections that rebuild from
  the log, and exact replay.
- Add the cognitive loop: one queue, one consumer, seeded randomness, injected clock, and a
  registry of mechanisms where each has a neutral value.
- Add the appraisal gate: signals, salience, habituation, threshold, a seeded go / no-go draw
  and the circuit breaker on commitments. Silence is a recorded decision.
- Add activation-based recall (ACT-R base level plus spreading activation) over the log.
- Add expectations: promises and predictions as typed records whose lifecycle lives in code,
  so a promise fires without the model having to remember it.
- Add reasoning: the context is rebuilt from the log on every wake (no session transcript),
  and the model acts only through the five thought acts.
- Add the gateway with its three doors, built-in plugins, capability routing, person mapping,
  one versioned configuration file, and model and embedding providers.
- Add the valve as a frame with two cost guards only: a daily budget and a runaway brake, each
  switchable, with a recorded on / off state. The core is otherwise unrestricted: no privacy
  filtering, no roles, no owner-only rule. The doors exist as hook points and every event
  records who started its chain and who could perceive it, so that policy is added later on
  top without changing the core.
- Add the soul's identity, with a default on first start and recorded changes.
- Add two bodies as built-in plugins: a terminal body and a scripted test body.
- Add the Lab: scenarios against the real program in virtual time, ablation by neutral value,
  run records, and development / validation / sealed sets. End-to-end scenarios run with a
  scripted model (deterministic) and with a real model (live).
- Add a research backlog that ties each open question to a Lab measurement.

Deliberately left for later changes: the two absolute lines (owner-only identity change and
privacy by audience), roles and permissions, chat-app adapters, installed (out-of-process)
plugins, quiet hours, approvals, sleep and
consolidation, learning, trust and credence, patience and inner-thought bars, free-text
expectation conditions, the perception ledger, hard forgetting, speech, and configuration
migration.

## Capabilities

### New Capabilities

- `protocol`: the six facts every body is described with, their versioning and their
  serialized form.
- `event-log`: the append-only record, record-first ordering, projections, versions in stored
  data and replay.
- `cognitive-loop`: the single queue and consumer, the order of one turn, reproducibility and
  the mechanism registry with neutral values.
- `appraisal-gate`: the model-free decision of whether a moment deserves reasoning.
- `memory-recall`: ranking memories by activation rather than by similarity alone.
- `expectations`: promises and predictions, their firing, answering and expiry.
- `reasoning`: context assembly per wake, the five thought acts and recorded model calls.
- `soul-identity`: the soul's name, persona and principles and how they change.
- `gateway`: doors, plugins, routing, person mapping, configuration and providers.
- `valve`: deterministic policy in the doors; in this change the frame, two cost guards and
  the recording of the facts later policy needs.
- `lab`: scenarios, ablation, run records, scenario sets and live end-to-end runs.
- `cli`: the `monoagens` program: chatting through the terminal body, running the Lab,
  replaying and inspecting the log.

### Modified Capabilities

None. `openspec/specs/` is empty.

## Impact

- **Code**: new Rust workspace (`crates/`), `prior/` (default dispositions), `lab/`
  (scenarios and run records), `research/` (backlog). No existing code is touched; there is
  none.
- **Toolchain**: Rust stable is required and is not installed on the development machine yet.
- **Dependencies**: SQLite (compiled in), an async runtime and an HTTP client in the gateway
  only. The core depends on the protocol crate and a serialization library, nothing else.
- **External systems**: live runs need an OpenAI-compatible chat endpoint and, optionally, an
  embeddings endpoint, set through environment variables. Default tests need neither.
- **Documents**: `RESEARCH.md` refers to sections, files and terms that `ARCHITECTURE.md` no
  longer has ("host" for gateway, "hooks" for doors, `prior/dispositions.json`, *Mental
  model*, *Evaluation*). This change reconciles those references; it does not change any
  invariant.
- **Conflict with earlier work**: the previous README described a PAD emotional engine.
  `ARCHITECTURE.md` forbids emotions and simulated feelings, so this change follows the
  architecture and builds drives only.
- **Safety note**: by the owner's decision this slice is unrestricted. Any actor can cause any
  capability to be used, including an identity change, and anything the soul was told can
  surface in any place. Only cost is bounded. This slice is for the terminal and the Lab, not
  for a shared deployment.
- **Departure from `ARCHITECTURE.md`**: the architecture describes the two absolute lines as
  part of the valve and on by default. This change does not build them yet. The architecture
  document is left unchanged; the lines arrive with the policy change that follows.

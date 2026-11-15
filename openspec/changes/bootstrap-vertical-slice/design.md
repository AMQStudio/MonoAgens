# Design

## Context

See `proposal.md` for motivation. The repository holds `ARCHITECTURE.md` and `RESEARCH.md` and
no code. The owner chose Rust for the core and the gateway, and a thin vertical slice as the
first change.

Constraints that shape the approach:

- The architecture invariants in `ARCHITECTURE.md` are fixed. This design implements them; it
  does not reinterpret them.
- The core is the research subject. It must be testable in milliseconds, replayable exactly,
  and free of anything a small device could not host.
- Earlier attempts suffered from slow tests. Test speed is a design requirement here, with
  budgets (Decision 12).
- Rust is not installed on the development machine; Python 3.15 and `uv` are.
- `RESEARCH.md` uses older terms. In this design: *host* means gateway, *hooks* means doors.

## Goals / Non-Goals

**Goals:**

- One loop that runs end to end: body → gateway → core → model → body, with every step on
  the record.
- A core with no IO, no async runtime, no clock and no threads of its own.
- Exact replay of decisions from the log.
- A Lab that can prove or refute a mechanism's claim by ablation.
- A layout in which later mechanisms, bodies and policies are additions, not rewrites.

**Non-Goals:**

- `no_std` builds of the core. The core avoids what would block it, but this change does not
  build or test for a microcontroller target.
- Installed (out-of-process) plugins. Boundary messages are serializable and tested for
  round-trip, so a process transport is an addition later.
- The two absolute lines, privacy filtering, roles, permissions, approvals, quiet hours and
  declassification. The valve in this change guards cost only.
- Sleep, consolidation, gist memories, fading, learning of weights or knobs, trust and
  credence, patience, inner-thought bars, free-text expectation conditions, the perception
  ledger, addressee degrees, initiative.
- Snapshots of projections, approximate vector indexes, configuration migration (only
  version 1 exists), hard forgetting.
- An LLM judge in the Lab. Checks are structural or simple text containment.

## Decisions

### 1. Workspace layout: dependencies point one way

```
crates/
  protocol/   monoagens-protocol   six facts, boundary messages, versions
  core/       monoagens-core       loop, gate, recall, expectations, context plan, registry
  store/      monoagens-store      SQLite log and memory index (implements core's port)
  gateway/    monoagens-gateway    runtime, doors, valve, plugins, persons, config, providers
  lab/        monoagens-lab        scenarios, test body, runner, ablation, run records
  cli/        monoagens            the binary; terminal body
prior/        dispositions.json, references.json
lab/          scenarios/{dev,validation,sealed}/, runs/
research/     backlog.md
```

`core` depends on `protocol` and `serde` only. A test fails the build if `core` gains any other
dependency. `gateway` depends on `core`; `lab` and `cli` depend on `gateway`. Nothing depends
upward.

*Alternative considered:* one crate with modules. Rejected: the dependency rule would be a
convention instead of something the compiler enforces, and core tests would compile the
gateway's dependencies.

### 2. The core is a sans-IO state machine with synchronous ports

```rust
impl<S: Store> Core<S> {
    fn open(store: S, prior: Prior, switches: Switches, seed: u64) -> Result<Self>;
    fn step(&mut self, input: Input, now: Time, doors: &mut dyn Doors) -> Result<Vec<Effect>>;
    fn next_due(&self) -> Option<Time>;
}
```

- `Input`: percept, receipt, reasoning result, reasoning failure, tick, identity set, valve
  state.
- `Effect`: `Wake(ContextPlan)` (ask the gateway to run the model) and `Dispatch(Action)`
  (hand an action to a body).
- `Store` (port): append, read in order, get by id, candidates by vector, candidates by text.
  The core ships an in-memory `Store` for its own tests.
- `Doors` (port): `inbound`, `context`, `action`, each returning a verdict. The gateway
  implements it with the valve first and last. A no-op `Doors` gives the valve-free core.
- Time is the `now` argument; randomness is a small generator in the core seeded with
  `hash(seed, event id)`.

`step` appends the input to the store before anything else, then follows the turn in
`ARCHITECTURE.md` *One turn of the loop*. The driver must not execute an effect whose event
failed to append; `step` returns an error in that case.

*Alternatives considered:* (a) async traits in the core. Rejected: pulls a runtime into the
research subject and makes replay harder to reason about. (b) a pure function returning
"append" effects. Rejected: recall needs to query the store inside the step, so a port is
needed anyway, and record-first is easier to guarantee in one place.

### 3. Events, ids and versions

- `EventId` is the log's sequence number (u64). Sources, places and capabilities are opaque
  strings given by the gateway.
- Event kinds in this slice: `percept`, `decision`, `wake_started`, `model_call`, `thought`,
  `action`, `receipt`, `expectation_opened`, `expectation_changed`, `memory_used`,
  `door_verdict`, `identity_changed`, `valve_state`, `tick`, `failure`.
- Every event stores: kind, format version, time, actor, place, audience, `origin_actor`,
  `chain` (the id of the event that started the causal chain), `cause` (the event it follows
  from), numeric features, and an opaque payload.
- `origin_actor` and `chain` are inherited from `cause` by the core. Nothing a model outputs
  can set them. No rule uses them in this slice; they are recorded now because the absolute
  lines will rest on them later (`RESEARCH.md` §10), and a log without them could not be
  protected after the fact.
- Upcasting is a function per kind from version n to n+1, applied on read.
- A tick is appended only when it fires something (spec `cognitive-loop`). Decay terms are
  computed from timestamps on demand, so unrecorded ticks leave no state behind and replay
  stays exact.

### 4. The core never reads payloads; the gateway attaches features

The gateway computes features at the inbound door's entry: the embedding vector (if a provider
is configured), `addressed` and `named` as reported by the body. The core uses only those.
Text search is the store's job: the core passes the *id* of the query event and the store
reads that event's payload for the full-text query.

A test processes one event stream twice with different payload bytes and identical features
and requires identical decisions.

### 5. Gate: formulas from RESEARCH.md §2–3, unchanged

Signals, salience, habituation, threshold and the go / no-go draw are implemented as written
in `RESEARCH.md` §2–3, with knob mappings from the prior (for example `talkativeness` 0 → θ₀
0.85, 1 → 0.35). `info_gap` uses reference vectors the gateway computes at start-up from
`prior/references.json`. The circuit breaker covers: `addressed`, an event that fires or
answers an open expectation, and an event whose `chain` is an active chain of the soul.

Patience and inner-thought bars are left out; a `go` on an unallocated turn wakes reasoning
directly, and reasoning may end the wake with only a thought.

### 6. Recall: ACT-R activation over events

Memories in this slice are events of kind `percept`, `thought`, `action` and `receipt`.
Candidates are the top 40 by vector and the top 20 by text (`RESEARCH.md` §5). Ranking is
`A = B + context_weight · cos + importance_weight · importance` with the Petrov approximation
for `B`. Use counts come from `memory_used` events, so they are a projection. Importance is 0
for percepts and model-given (0..1) for thoughts.

Without vectors, the cosine term is 0 and candidates come from text only.

### 7. Expectations: two kinds, two conditions

- Kinds: `promise` (opened by reasoning through `expect`) and `prediction` (opened by the loop
  after every action).
- Conditions: `due_at` (a time) and `reply_from` (a source in a place before an expiry).
- A fired promise stays open. A failed wake schedules a refire with backoff (1, 5, 30
  minutes, then hourly). Only `resolve` or `revise` closes or moves it.
- A wake caused by a fired or answered expectation runs in that expectation's `chain`, not in
  a new one. This is what carries work across places: a request made in private, an act in a
  group and the group's answer are one chain, and the answer's wake sees the whole of it.
- An incoming percept is *self-caused* when its body reports it as the delivery of an action
  with an open prediction (the body echoes the action id). Matching by content is left for
  later.
- `next_due()` returns the earliest due or expiry time. The runtime sleeps until then or until
  the next event; the Lab jumps its virtual clock to it.

### 8. Context: the core plans, the gateway renders, one model call per wake

The core builds a `ContextPlan`: an ordered list of sections, each a list of event ids with
provenance, plus the context audience. Order (stable → volatile): identity, capabilities
(sorted by id), recalled memories, open expectations for this place and source, chain working
set, recent events of the place, `<now>`, triggering event. The plan passes the context door;
in this slice the door removes nothing, but the plan and the recorded `wake_started` event
state the audience of the trigger and of every item, which is what a privacy rule will filter
on later. The gateway then reads payloads and renders the prompt; payloads from others are
wrapped in quoted-data delimiters.

Bounds: recalled memories top-k (default 12), recent events of the place (default 20), chain
working set (default 30 events), all dispositions in the prior.

Each wake is one fresh model request: a system part (identity, capabilities, rules of the
thought acts) and a user part (the rest). The five thought acts are offered as tools. The
response's tool calls are parsed into protocol `ThoughtAct` values and returned to the queue
as one reasoning-result event. Tool results are never sent back in the same request: a
receipt comes back as an event and, through the circuit breaker, causes the next wake with a
rebuilt context. This is how the soul works through several steps without a transcript. The
working set of a chain is the part of the log that plays the role of a conventional context
window during execution, and it stays in the log afterwards.

*Alternative considered:* a tool loop inside one model conversation. Rejected: it creates a
second place where state lives, outside the log and outside the doors.

### 9. Valve: pure functions over facts

`fn judge_inbound(facts, config) -> Verdict`, `judge_context(plan, config) -> Plan'`,
`judge_action(facts, config) -> Verdict`. No IO, no model, no randomness. Rules in this slice:

| Door | Rule | Default |
|---|---|---|
| context | refuse wake when day's tokens ≥ budget, unless caused by a fired promise | on |
| context | refuse wake when chain wakes ≥ `max_chain_wakes` | on |

The inbound and action doors carry no rule. The owner decided that this slice restricts
nothing but cost: the core is fully autonomous, and privacy, roles and the absolute lines are
built later as further rules in the same doors. Two things make that possible without touching
the core again:

- **The doors are complete hook points now.** Every percept, context plan and action already
  passes its door with all its facts, and the "valve first and last, others only narrow"
  ordering is implemented and tested with the two guards.
- **The facts are recorded now.** Every event has `origin_actor`, `chain` and an audience. A
  thought's audience is the audience of the wake's trigger; an action's and its receipt's
  audience is that of the target place. The identity-changing flag stays on the capability
  descriptor.

Every refusal becomes a `door_verdict` event with the rule's name. The valve's state is
recorded at start and on change. Each rule can be switched off in `[valve]`.

*Alternative considered:* build line 1 and line 2 now, as `ARCHITECTURE.md` describes. Set
aside by the owner: the first privacy rule drafted for this slice already blocked a plain use
(ask a group, report back in private), and choosing the right rule needs the running system
and the Lab. Building it later is an addition; guessing it now would mean changing it.

### 10. Storage: SQLite, compiled in

`rusqlite` with the `bundled` feature (its build enables FTS5). One database file in the data
directory: an `events` table (sequence, kind, version, time, facts, payload), an FTS5 table
over text payloads, and vectors as blobs. Vectors are loaded into memory at open and searched
by brute-force cosine.

*Alternatives considered:* (a) PostgreSQL + pgvector, as in earlier work. Rejected: a server
dependency contradicts "lives anywhere", and slows every test. (b) a custom append-only file.
Rejected for now: SQLite gives durability and full-text search for free; the `Store` port
keeps the choice replaceable.

### 11. Gateway runtime, configuration and providers

- **Runtime:** `tokio`. Sources (bodies, model results, ticks) send into one `mpsc` channel;
  one task owns the `Core` and consumes it. Model calls and body actions run as spawned tasks
  under `tokio::time::timeout`; their results return through the channel. The core itself is
  called synchronously.
- **Configuration:** `monoagens.toml`, `version = 1`, sections `soul`, `owner`, `person`,
  `model`, `embedding`, `valve`, `loop`, `data`. Edited in place with `toml_edit` when the
  identity capability is used, after a backup copy. Default paths follow XDG and can be
  overridden with `--config` or `MONOAGENS_HOME`.
- **Secrets and endpoints:** environment variables `MONOAGENS_MODEL_BASE_URL`,
  `MONOAGENS_MODEL_API_KEY`, `MONOAGENS_EMBEDDING_BASE_URL`, `MONOAGENS_EMBEDDING_API_KEY`, or
  a `secrets.toml` beside the configuration with owner-only permissions. A secret-looking key
  in `monoagens.toml` is a start-up error.
- **Model providers:** `openai-compatible` (chat completions with tools, via `reqwest`) and
  `scripted`. One adapter covers most hosted and local servers; native provider adapters are
  later additions behind the same interface.
- **Embedding providers:** `none`, `openai-compatible`, `hash` (deterministic feature hashing,
  for offline use) and `scripted` (the Lab maps a scenario's concept labels to vectors, so
  meaning-based recall is testable without a model).
- **Persons:** `[[person]]` entries map `body:account` strings to a person id; one is named in
  `[owner]`. An unmapped account gets `per_` + a hash of `body:account`, which is stable across
  restarts without stored state.
- **Identity capability:** the gateway registers `identity.revise`, flagged identity-changing.
  Anyone can cause it in this slice; the change event records the chain's `origin_actor`.

*Alternative considered for the runtime:* threads and blocking HTTP, with no async runtime.
Smaller, but timeouts on every hook and action are uniform under `tokio`, and future
transports for installed plugins and chat bodies are async in practice.

### 12. Tests in four layers, with time budgets

| Layer | What | Budget |
|---|---|---|
| core unit | in-memory store, no IO | whole suite under 3 s |
| gateway integration | scripted provider, in-memory SQLite | under 15 s |
| Lab scripted | development set, virtual time | under 30 s |
| Lab live | real model, on request only | not in `cargo test` |

`cargo test` runs the first three. A budget overrun is treated as a defect.

### 13. Lab: TOML scenarios, virtual time, outcome checks

A scenario file declares persons and places, timed steps (`at`, body, place, actor, features,
text, optional concept labels), scripted model outputs keyed by trigger, and checks. The
runner builds the real gateway and core over an in-memory database with a virtual clock and
test bodies, feeds steps in time order, advances the clock to `next_due()` between steps, and
evaluates the checks against the log.

- Ablation: `--ablate <mechanism>` sets that row of the registry to neutral. `lab compare`
  runs both and prints per-metric differences.
- Run record: one JSON file per run in `lab/runs/`, with the fields listed in spec `lab`.
- Sealed set: the runner loads it but prints aggregates only. This change ships the directory
  empty with a note; the owner writes sealed scenarios and does not show them to whoever
  designs mechanisms.
- Live mode: the same scenarios without the scripted outputs. Checks are tagged `structural`
  (must pass) or `wording` (reported as a rate).

### 14. Mechanism registry for this slice

One table in the core; each row has an id, the `RESEARCH.md` section, a claim and a neutral
value. Registration without a claim does not compile.

| Mechanism | § | Neutral value | Worse without it |
|---|---|---|---|
| `own_name` | 2 | weight 0 | misses unallocated mentions of its name |
| `info_gap` | 2 | weight 0 | misses questions it could answer |
| `feeling_of_knowing` | 2 | weight 0 | wakes for what it has no memory of |
| `relevance` | 2 | weight 0 | misses events that touch open promises |
| `social_cost` | 2 | weight 0 | speaks when the floor is someone else's |
| `habituation` | 3 | h = 0 | wakes again and again for a repeating source |
| `talk_rate` | 3 | a = 0 | takes more than its share of a group |
| `budget_drive` | 3 | b = 0 | spends the day's budget early |
| `promise_debt` | 3 | c = 0 | attends less when promises are due |
| `familiarity` | 3 | r = 0 | speaks up in places it does not know |
| `circuit_breaker` | 3, 15 | off: commitments go through the draw | ignores allocated turns and fired promises |
| `base_level` | 5 | B = 0 | recall degrades with age and distractors |
| `spreading` | 5 | `context_weight` = 0 | recall ignores the present context |
| `importance` | 5 | `importance_weight` = 0 | its own notes rank like chatter |
| `prediction` | 17 | no prediction opened | replies to its acts need an explicit address |
| `agency` | 19 | off: echoes are appraised | reacts to the echo of its own acts |

### 15. Research is tracked beside the code

`research/backlog.md` lists open questions. Each entry states the question, why it matters,
which Lab measurement would answer it, and its status. A new mechanism enters only as: a
`RESEARCH.md` section with sources → a registry row with a claim and neutral value →
development scenarios → an ablation result. Entries seeded by this change:

1. Rebuilt context against a transcript baseline on multi-step tasks.
2. Which privacy rule to build: how line 2 can hold while chains that cross audiences (ask a
   group, report back in private) keep working. Measured with the cross-audience scenario and
   a count of private items that reach a wider audience with no rule in place.
3. Whether gate signals computed over private memories leak through behaviour once a privacy
   rule exists.
4. Embedding model for multilingual and on-device use; quality of the hash fallback.
5. Verification of the 2026 preprint citations flagged in `RESEARCH.md`.
6. Human baseline data for when-to-speak calibration.
7. Prediction expiry per medium.
8. Start-up time and memory on long logs (snapshots, vector index).
9. A session-and-notes baseline in the style of existing autonomous harnesses, run on the same
   scenarios.
10. What a microcontroller hosts: a body only, or the core.

## Risks / Trade-offs

- [The slice is still wide for one change] → Task groups are ordered so that each ends in
  passing tests of its own; the core groups need nothing from the gateway.
- [Nothing but cost is restricted: any actor can cause any capability, including an identity
  change, and private material can surface in any place] → The owner's decision for this
  slice. Stated in the proposal and in the README; only terminal and test bodies ship; the
  Lab reports how often private items reach a wider audience, as the baseline the later
  privacy rule must beat.
- [This slice does not yet hold the two absolute lines that `ARCHITECTURE.md` puts in the
  valve] → The facts they need are recorded from the first event and the doors are in place,
  so they are added as rules; research backlog entry 2 decides their form.
- [A runaway chain or a loop bug could spend the model budget] → The budget and the brake are
  on by default and are the only rules kept.
- [Projections and vectors live in memory and are rebuilt at open] → Fine to roughly 10⁵
  events; backlog entry 8. The `Store` port hides the change.
- [OpenAI-compatible servers differ in tool-calling behaviour] → The adapter normalizes tool
  calls and treats plain text as a thought; scripted tests do not depend on it; live results
  separate structural from wording checks.
- [Live runs are not reproducible] → Decisions and valve behaviour still are, given the
  recorded model outputs; replay never calls the model.
- [The hash embedder is lexical, not semantic] → Meaning-based recall is asserted with the
  scripted embedder in default tests and with a real embedder live.
- [Rust compile times] → Heavy dependencies sit only in `store` and `gateway`; core tests
  build `protocol`, `core` and `serde`.
- [`RESEARCH.md` terms differ from `ARCHITECTURE.md`] → A task reconciles the references
  without changing formulas or invariants.

## Migration Plan

Greenfield; there is nothing to migrate and nothing to roll back.

1. Install the Rust stable toolchain with `rustup` and pin it in `rust-toolchain.toml`.
2. Initialize a git repository so that the core version in run records is a commit.
3. Data created by this slice carries format versions from the first event, so later changes
   upcast instead of migrating.

## Open Questions

- **License.** Dual MIT / Apache-2.0 is the usual choice for Rust projects aiming at wide
  adoption. The owner decides before the first public release; it does not affect this change.
- **Crate and binary name on crates.io.** `monoagens` must be checked for availability before
  publishing; not needed to build.
- **Reference model for published run records.** Any OpenAI-compatible model works for the
  live runs; which one is quoted in results can be chosen when results are first published.

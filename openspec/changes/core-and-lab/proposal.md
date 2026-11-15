# Proposal

## Why

MonoAgens asks one question: how far can a harness carry a mind? Today there is no code, so
nothing in `RESEARCH.md` can be measured. The owner decided to answer that question first and
alone: build the core as far as the research already reaches, and the Lab that measures it,
and leave bodies, tools, plugins, the gateway and policy for later.

## What Changes

- Add a Rust workspace of five crates: `protocol`, `core`, `store`, `reason` and `lab`. The
  core has no IO, no async runtime, no clock and no threads of its own.
- Add the protocol's six facts and the five thought acts as versioned, serializable types.
- Add the append-only event log, record-first ordering, projections and exact replay.
- Add the cognitive loop with one queue and one consumer, three doors as empty hook points,
  seeded randomness, an injected clock and a registry of mechanisms with neutral values.
- Implement every mechanism of `RESEARCH.md` sections 1–21 that has a stated model:
  - the gate: signals, salience, habituation, drives, the seeded go / no-go draw, the circuit
    breaker, patience and backlog chunking;
  - memory: ACT-R activation, hybrid candidates, explicit recall, familiarity against
    recollection, closeness;
  - expectations: promises, predictions and waits; time, reply and free-text conditions with
    the wake gate; grounding with content misses; sense of agency; judged silence;
  - reasoning: a context rebuilt per wake, the five thought acts, inner-thought bars, control
    of report, grounding in time;
  - beliefs: source monitoring, verbatim against gist, the reality filter, credence and source
    reliability;
  - sleep: consolidation into gist at event boundaries, pattern separation and fading;
  - the perception ledger for other minds and for the soul itself;
  - learning with bounds: the delta rule, bounded knob updates and wake-band learning.
- Add `reason`: prompt rendering and the model and embedding providers (scripted and
  OpenAI-compatible), which the core needs in order to think at all.
- Add the Lab: scenarios against the real core in virtual time, scripted test bodies with
  scripted capabilities, ablation, run records, three scenario sets, human-likeness metrics,
  a cost guard, deterministic scripted runs and live runs with a real model, and a probe for
  talking to a persistent soul from the terminal.
- Add a research backlog that ties each open question to a Lab measurement.

Left for later changes: wave 2 and wave 3 of `RESEARCH.md` (sections 22–25: addressee degrees,
idle mode and initiative, drives as opportunity costs, maps, habits and conventions); the
gateway, the valve and all policy (`RESEARCH.md` section 10: the two absolute lines, privacy,
roles); real bodies, tools and plugins; the `monoagens` product CLI; hard forgetting.

The earlier change `bootstrap-vertical-slice` is parked. Its gateway, valve, CLI and terminal
body specs are kept as material for the change that follows this one.

## Capabilities

### New Capabilities

- `protocol`: the six facts, the thought acts, versions and the serialized form.
- `event-log`: the append-only record, record-first ordering, projections and replay.
- `cognitive-loop`: the single queue and consumer, the doors as hook points, reproducibility
  and the mechanism registry.
- `appraisal-gate`: the model-free decision of whether a moment deserves reasoning, including
  patience and backlog chunking.
- `memory-recall`: ranking by activation, explicit recall, and telling recollection from
  familiarity.
- `expectations`: promises, predictions and waits; their conditions, the wake gate, grounding
  and judged silence.
- `reasoning`: context assembly, the thought acts, inner-thought bars and control of report.
- `beliefs`: what the soul may assert: basis, trace, validity, credence and source
  reliability.
- `sleep-consolidation`: consolidation while idle, event boundaries, gist, pattern separation
  and fading.
- `perception-ledger`: per-source facts about who witnessed what, and the soul's card of
  itself.
- `bounded-learning`: learning from outcomes within bounds, with every change on the record.
- `soul-identity`: the soul's name, persona and principles and how they change.
- `model-providers`: interchangeable language-model and embedding providers.
- `lab`: scenarios, test bodies, ablation, run records, scenario sets, metrics, live runs and
  the Lab's commands.

### Modified Capabilities

None. `openspec/specs/` is empty.

## Impact

- **Code**: new Rust workspace (`crates/`), `prior/`, `lab/` (scenarios, run records),
  `research/`, `docs/`. There is no existing code.
- **Toolchain**: Rust stable is required and is not installed on the development machine yet.
- **Dependencies**: SQLite compiled in (`store`), a blocking HTTP client (`reason`). The core
  depends on the protocol crate and a serialization library only. No async runtime.
- **External systems**: live runs need an OpenAI-compatible chat endpoint and, for
  meaning-based recall, an embeddings endpoint, set through environment variables. Default
  tests need neither.
- **Cost**: sleep, classification and learning add model calls. Each is recorded with its
  purpose, and the Lab's cost guard caps tokens per run and wakes per chain.
- **Documents**: `RESEARCH.md` refers to sections, files and terms that `ARCHITECTURE.md` no
  longer has. This change reconciles those references without changing formulas.
- **Not a product yet**: there is no body except the Lab's test bodies, no tool, and no
  restriction on who may cause what. Nothing in this change is meant for a shared deployment.
- **Research**: the open points of `RESEARCH.md` and `ARCHITECTURE.md` went to a researcher
  (`research/researcher-prompt.md`). Run 1 (`research/findings/2026-11-15-run-1.md`) is
  accepted for R-01 to R-05 and R-07 to R-28, with every number as a start value to tune in
  the Lab, and the owner ruled on four contradictions between the two documents (design,
  Decision 1). No task waits for research: what run 1 left open is built as stated or left
  neutral, and goes to research brief 2, whose findings enter the next change.
  Most formulas are the researcher's inference from sourced shapes, so they are expected to
  move with Lab results.
- **Evidence rule**: by `ARCHITECTURE.md`, a mechanism whose removal changes nothing measurable
  is removed. Some mechanisms built here may therefore be taken out again by their own
  ablation result.

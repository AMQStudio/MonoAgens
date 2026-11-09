# ADR-0011: Layer Boundaries and Core Charter — Core as Engine, Soul, Moral, Privacy, and Memory; Gateways as Body and UX; Plugins as Capability

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** A read-only audit of the whole repository (nine areas, 184 findings, 42 tagged high) showed that the core owns presentation, persona, and conversation-shaping heuristics, and guards its invariants with LLM reviewers after drafting. This ADR fixes what each layer owns and how layers talk, so the overhaul ADRs that follow (ADR-0012, ADR-0013, ADR-0014, ADR-0015) have one boundary to build on.
- **Supersedes:** in part ADR-0001 §4 (adds the charter to the appendage seam), ADR-0009 §4 (affect and vibe routing leave the core), ADR-0010 §4 (the gateway-side badge parsing and the reserved layer contract)
- **Related:** ADR-0003 (turn lifecycle), ADR-0006 (RBAC), ADR-0012 (envelope), ADR-0013 (flows), ADR-0014 (packaging and hooks), ADR-0015 (i18n)
- **Implementation:** draft

---

## 1. Context

MonoAgens promises one continuous Soul that lives in many bodies, with deterministic privacy
"before any LLM sees the prompt" and a persona-agnostic engine (README). The audit of
2026-11-09 found the code drifting from that promise in ten recurring ways:

1. No message envelope: speaker, addressee, privacy, and kind are re-derived per use, often by
   an LLM, and command receipts are stored as the bot's own words. The model then claims the
   user's actions ("I approved #5") and misreads replies to its own messages.
2. Detection after drafting: privacy, cross-room isolation, and refusal honesty are enforced by
   reviewers over the draft (up to seven top-level LLM calls per reply); some fail open.
3. Mechanisms as chat text: approvals are parsed from replies by a regex, confirmations from
   English keyword tables, `/link` guesses pairing codes from free text.
4. Persona inside the core: about 4600 characters of persona prose in the core prompt, persona
   logic in the privacy service, stores, the LLM client, and jobs.
5. Invariant violations: missing lease fences, blocking calls in `async def`, a subject-veto
   bypass, plugin tables outside Alembic.
6. Capability dishonesty: unconfigured tools are offered to the model.
7. An i18n regime that forbids partial language packs.
8. Four near-identical gateways and a broken Discord interaction path.
9. A 1981-line CLI and a 2206-line installer that duplicates the Python admin.
10. Dead code and never-read configuration.

The product is not released, so a structural correction is cheaper now than later.

## 2. Decision drivers

- The README's promises must become true in code, not in prompts.
- Every new body (gateway) or capability (plugin) must be addable without editing the core.
- Safety must be deterministic and fail closed; an LLM may phrase, never guard.
- The model must always know who it is, who is speaking, and to whom.
- Few concepts, each invariant guarded twice ("simple but redundant").
- Keep the sovereign topology: one deployment, one Soul, one PostgreSQL database, one primary.

## 3. Options considered

1. **Patch in place.** Fix each finding where it sits. Rejected: the root causes (no envelope,
   detection after drafting, UX in core) would keep producing new findings.
2. **Split into services around a message broker.** Rejected: violates the one-database
   topology and adds operations cost with no benefit for a single Soul.
3. **Adopt an agent framework for orchestration.** Rejected: replaces exactly the parts that are
   the product (soul continuity, memory, RBAC, privacy).
4. **Core charter with gateways for body and UX and plugins for capability (chosen).**

## 4. Decision

### 4.1 Core charter

The core (`monoagens/engine/`) owns five things and nothing else:

| Area | Owns |
|---|---|
| Engine | runtime, lease, durability, flow runner (ADR-0013), tool door, `Agent.dispatch()`, mechanism events |
| Soul | identity and the self-model: who it is, what it just did, who it is talking to (§4.5) |
| Moral | behavioral rules from `soul.yaml`, `SafetyPolicy`, RBAC from `roles.yaml` |
| Privacy | envelope labels (ADR-0012), audience filtering at context assembly, consent, subject veto |
| Memory | `MemoryStore`, `memories_readable`, automatic capture and retrieval, `recall`, `forget` |

The core decides **what** happens: who may decide, what an audience may see, which tool runs.
It never decides **how** something looks on a platform, never holds platform vocabulary
(emoji, buttons, reaction hints), and never parses mechanism decisions out of chat text.

A tool or command stays in the core only when it operates a core mechanism; anything that
acts on the world is a tool package, a plugin, or gateway UX (ADR-0014):

- Memory is automatic and is not a tool: capture at ingress and turn end, retrieval in every
  turn, consolidation in the background (ADR-0016). The model has two doors: `recall` (a
  deliberate, deeper search over facts, conversations, and activity, which absorbs today's
  `thread_history` and memory search) and `forget` (a request that the core validates against
  the subject and executes as the forgetting cascade). `info` exposes self-knowledge.
- Core commands: `/approve`, `/deny`, `/pending` (flows); `/link`, `/unlink`, `/whoami`
  (identity); `/memory` (a person views and deletes their own memories, no model call),
  `/search` (archive search, no model call), `/forget`; `/status`, `/alerts`, `/reload`,
  `/freeze`, `/stop`, `/ping` (runtime); `/help`, `/tools`, `/language`; `/skill` (procedural
  memory).
- Everything else moves: file, research, messaging, and automation tools to the `assistant`
  tool package; `sandbox`, `shell`, `settings`, MCP bridge to `power`; `/mood`, `/calmdown` to
  the expression plugin; `/steps`, `/thinking` to the gateway base.

### 4.2 Gateway responsibilities

A gateway is the body and the user experience of one transport. It owns transport I/O, pairing,
address normalization, the rendering of every core event and requirement kind (ADR-0013), the
translation of gestures (button, reaction, typed reply) into structured decisions, its own
strings (ADR-0015), and the transport hints that feed the envelope (mention, reply target,
direct or group). A gateway never decides authorization or audience.

### 4.3 Plugin responsibilities

A plugin adds capability or behavior: tools, commands, hooks, and flows with their own
preconditions (ADR-0013). A plugin teaches the core what it needs through typed requirements;
the core never learns plugin-specific rules. A plugin never provides a body. The packaging and
hook surfaces are fixed in ADR-0014.

### 4.4 Communication between layers

- Layers communicate only through typed contracts exported by one public package,
  `monoagens.sdk`: `Percept`, `Address`, `Action`, envelope labels, outcome and requirement
  types, event and decision types, hook protocols, and service handles.
- Gateways and plugins import only `monoagens.sdk`; the core imports neither gateways nor
  plugins.
- These boundaries are enforced by import-linter contracts in the verification gate, not by
  review discipline.

### 4.5 Soul split: identity stays, expression moves

Identity is self-awareness and stays in the core: name, pronouns, language, timezone,
description, background, persona and character text, behavioral rules, dialogue examples, and
the self-model. Only the expression layer moves to a plugin that is **disabled by default**:
affect (PAD), mood, vibe, per-person style learning, humanize (pacing, bubble splitting),
polish, ambient behavior, voice sampling knobs, and `mode`/`modes`. Disabling the expression
plugin makes the Soul plainer, never less aware of itself.

### 4.6 Simple but redundant

Each invariant is guarded by two independent deterministic mechanisms that call the same policy
function at two points, and both fail closed:

| Invariant | First guard | Second guard |
|---|---|---|
| Privacy and cross-room isolation | audience filtering at context assembly | audience check in `Agent.dispatch()` |
| Single primary | lease fence in every mutating store | lease check in `Agent.dispatch()` |
| Authorization of decisions | gateway offers the gesture only to deciders | core re-validates decider, RBAC, subject veto |

LLM judges are not safety controls. They are retired as their causes are removed (ADR-0012 §4).

### 4.7 Trust tiers

First-party gateways and plugins run in-process through `monoagens.sdk`, are reviewed, and are
digest-verified. Untrusted third-party capability runs out of process through MCP and cannot
use deep hooks. In-process Python cannot be sandboxed; the tier boundary is the security
boundary.

### 4.8 Turn-taking: when the Soul speaks

Whether to answer is a core mechanism, not a per-turn LLM call, and it is separate from
permission:

- **Permission** (`roles.yaml`, per gateway): may this person's messages be processed and
  answered at all (`dm`, `groups: none | addressed`).
- **Turn-taking** (core): is this message meant for the Soul. Direct chats always are; in groups
  the deterministic addressing score of ADR-0012 §4 decides (mention passes alone; a reply to
  the Soul that tags someone else does not; plain messages need a clear continuation).

A message that is not permitted or not addressed is recorded for context and never answered,
and a shared room never receives an access refusal. The main model may still choose a reaction
or a short acknowledgement when it answers. Social initiative in groups (joining unasked) is an
expression-plugin behavior, allowed only for roles with `initiative: true`, only for messages
with a low score that are not addressed to another person.

### 4.9 Errors are visible

A failure in the core is either raised to its caller or counted: every caught exception that is
not re-raised increments a named failure counter shown by `/status` and logs once with context.
Catching and continuing silently is not allowed. Background jobs report their last failure and
count in the same place.

## 5. Configuration

- No new core keys. The expression keys of `soul.yaml` (`voice.*` sampling, affect, style,
  humanize, `mode`, `modes`) move to the expression plugin's configuration (ADR-0014).
- `pyproject.toml` gains an import-linter contract section; the verification gate runs it.

## 6. Metrics and gates

The core reaches the gold standard when every row holds (exception lists may exist during the
overhaul, dated and only shrinking, and are empty at the end):

| Criterion | Measure |
|---|---|
| Content | only the five areas of §4.1; no persona prose, no platform vocabulary (selfcheck) |
| Boundaries | import-linter: zero violations |
| Safety | zero LLM judges; each invariant of §4.6 has two deterministic guards, each tested |
| Types | Pyright strict for `monoagens/engine/`, no baseline entries |
| Errors | §4.9 holds; no silent `except` in the core (selfcheck) |
| Async | zero synchronous store, database, NER, or embedding calls in `async def` (AST test) |
| Configuration | every option is read by code and carries a generated comment; zero dead options |
| Cost | one top-level LLM call per reply by default (today up to seven) |
| Tests | every golden scenario of ADR-0017 passes, including its decision reasons |

## 7. Acceptance criteria

- `monoagens/engine/` contains only the five areas of §4.1.
- Removing every plugin leaves a working Soul with the essential tools and commands.
- Adding a gateway or plugin requires no edit under `monoagens/engine/`.
- Every row of the table in §4.6 has a test for each guard.

## 8. Risks and mitigations

- **Large refactor regresses behavior.** Mitigation: phased plan, invariant tests and golden
  conversation scenarios before each removal (ADR-0012 §8).
- **Grey areas between core and gateway.** Mitigation: the rule of §4.1 (what versus how);
  undecided cases are recorded in the ADR that touches them.
- **SDK freezes too early.** Mitigation: `monoagens.sdk` carries a version declared by each
  gateway and plugin manifest (ADR-0014); start small and add on demand.

## 9. Interaction with other ADRs

- ADR-0001: keeps the topology and hexagonal seam; this ADR adds the charter.
- ADR-0003: turn lifecycle remains; the review chain is replaced per ADR-0012.
- ADR-0006: RBAC remains the Moral area; approvals move to ADR-0013.
- ADR-0009: affect and vibe routing move to the expression plugin.
- ADR-0010: per-gateway completeness stays; badge parsing and the reserved layer contract are
  replaced by ADR-0013 and ADR-0014.

## 10. Code references

- Core today: `monoagens/engine/` (persona prose in `turn/prompts.py`, `character/`,
  `stores/affect.py`, `stores/vibe.py`, `jobs/ambient.py`).
- Gateway seam today: `monoagens/engine/inbound.py`, `monoagens/gateways/base.py`.
- Audit evidence: `.audit/00-synthesis.md` and `.audit/01..09-*.md` (local, not committed).

*This ADR lives at `docs/v0.1.0/adr/0011-layer-boundaries-and-core-charter.md`.*

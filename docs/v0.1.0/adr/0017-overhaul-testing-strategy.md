# ADR-0017: Overhaul Testing Strategy — Invariant Tests, Architecture Contracts, and Golden Conversation Scenarios

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** The overhaul (ADR-0011 to ADR-0016, ADR-0019, ADR-0020) removes LLM judges, moves privacy enforcement, and restructures the core. Today many tests pin the compensation layers instead of the behavior, invariants such as lease fencing and non-blocking async are not tested mechanically, and observed conversation failures have no regression tests. This ADR fixes the safety net that must exist before each removal.
- **Supersedes:** in part ADR-0008 §4 (testing discipline of 4.5 is extended)
- **Related:** ADR-0011 (boundaries), ADR-0012 (envelope), ADR-0013 (flows), ADR-0016 (memory), ADR-0019 (grounding), ADR-0020 (presets and roles)
- **Implementation:** partial — P0 (2026-11-10): the fencing and async invariant tests exist with dated, shrinking exception lists; structural tests cover the envelope, addressing, taint, and the P0 fixes. Still open: import-linter contracts, golden conversation scenarios with recorded model responses, transport recordings, live suites, and the coverage gate.

---

## 1. Context

Audit evidence: tests pin the claim judge and review chain rather than labels
(`.audit/02-turn-pipeline.md` F21); invariant gaps around fencing and blocking calls
(`.audit/08-stores-llm-providers.md` F22, `.audit/06-jobs.md` F23); thin command and gateway
coverage (`.audit/04-access-commands-guard.md` F17, `.audit/01-ingress-gateways.md` F14).
The real failures the owner observed (approval hint on a summary, "I approved #5", a quote-reply
to the Soul read the wrong way, forgetting another room's activity) have no tests.

## 2. Decision drivers

- Every removal of a safety layer is preceded by tests that prove its replacement.
- Tests assert structure (labels, rendered lines, events, decisions), not model wording.
- Invariants are checked by machines in CI, not by review.
- The suite stays deterministic without network access.

## 3. Options considered

1. **Live-model evaluations in CI.** Rejected: nondeterministic, costly, needs keys.
2. **Unit tests only.** Rejected: the failures happen across layers.
3. **Four layers of tests with recorded model responses (chosen).**

## 4. Decision

### 4.1 Invariant tests (written first, phase P0)

- Fencing: every mutating store method carries the lease fence; a deposed instance writes zero
  rows (store-by-store test plus a generic test over registered stores).
- Async integrity: an AST check fails on synchronous store, database, NER, or embedding calls
  inside `async def` outside the bounded executor.
- Schema: ORM metadata equals the Alembic head (full diff, not a name list); no runtime DDL.
- Configuration: root `config.yaml`, `roles.yaml`, `soul.yaml` equal the generator output.

### 4.2 Architecture contracts

import-linter contracts in the verification gate: the core imports neither gateways nor
plugins; gateways and plugins import only `monoagens.sdk`; a dated exception list may only
shrink. selfcheck rules: no platform vocabulary or persona prose in the core, no regex or keyword
parsing of mechanism decisions.

### 4.3 Golden conversation scenarios

- Each scenario is a scripted multi-room conversation fed through a fake gateway, with model
  responses recorded once (cassette) and replayed in CI.
- Assertions are structural: stored envelope labels, rendered transcript lines (speaker, actor,
  reply-to-self), raised requirements and decisions, audience filter output, dispatch verdicts,
  tool calls made or blocked, and the number of model calls.
- Seed scenarios from observed failures: archive summary that mentions `/approve #5`; user runs
  approvals then asks "what did you approve"; quote-reply "bro gaslighting" to the Soul's own
  message; stale Google failure after configuration is fixed; cross-room request from a member
  and from a non-member; recollection of another room's activity without `thread_history`; a
  fetched page instructing a send; forget then consolidate then recall; Google edit flow in a
  group; group addressing (mention, reply to the Soul that tags another person, plain
  continuation, plain chatter, the Soul's name in text); a guest without group permission
  posting in a group (zero replies, zero refusals); repeated direct messages from a person
  without access (one notice per window).
- Re-recording a cassette is an explicit command and the diff is reviewed.

### 4.4 Test realism ladder and real transport recordings

Current gateway tests build platform events by hand (`SimpleNamespace`, `FakeClient`), so they
test our assumptions about a platform, not the platform; the dead Discord slash-command path
passed that way. Fakes therefore sit only at the outermost boundary (network or SDK client),
never inside our code, and fake data is a recording of the real platform:

| Level | What | Where it runs |
|---|---|---|
| 1 | Invariant and contract tests (§4.1, §4.2) | CI |
| 2 | Replay of real inbound transport payloads (neonize protobuf events, Telegram updates, Discord gateway payloads, Slack events) through the real gateway ingress code | CI |
| 3 | Outbound assertions at the SDK client boundary, compared with recorded real calls | CI |
| 4 | Recorded model responses (cassettes, §4.3) | CI |
| 5 | Live suites against real accounts and real apps (§4.5) | on demand, not CI |

- Each gateway gets a record mode (`MONOAGENS_RECORD_TRANSPORT=1`) that stores inbound events
  and outbound SDK calls under `tests/data/transport/<gateway>/`.
- Recordings are anonymized automatically before they enter the repository: numbers, names,
  JIDs, ids, and message text are replaced by stable placeholders; structure is kept.
- A real observed case becomes a test by recording it once (reply to the Soul that tags
  someone, forwarded PDF, group mention, Discord slash command).
- Hand-built platform events are replaced by recordings whenever a change touches that gateway.

### 4.5 Live suites (not CI)

Live suites run on the owner's machine against dedicated test rooms, driven by Orca:

| Suite | Purpose | How |
|---|---|---|
| End-to-end and real usage | every area of the coverage matrix (§4.8) through real apps and accounts, including cross-gateway | `orca computer` script plays the human side |
| Long conversations | hundreds of turns and multi-day threads: context budget, salience, no repetition, no drift of identity or facts | `orca computer` script plus replay harness with recorded model |
| Complex requests | multi-step work (research, write a document, send it to another room, schedule a follow-up), ambiguous and multi-person requests | `orca computer` script |
| Complex automations | multi-step goals, conditional checks, chained and concurrent routines, failures and retries, quiet hours across days, missed runs | replay harness on a controlled clock plus live runs |
| Context and freshness | casual chat, questions about recent news and changing facts, asking beyond the model's knowledge boundary, remembered facts going stale | `orca computer` script daily; assertions on search, source dates, grounded specifics |
| Real workload | a realistic mix of real tasks with the real model at modest volume: time, cost, and success per task | replay harness with the real provider; never through a messaging account |
| Privacy | cross-room requests from members and non-members, consent, forgetting | `orca computer` script in two test rooms |
| Vulnerability | prompt injection through fetched pages and files, approval abuse, role escalation, secrets in chat | `orca computer` script plus red-team exploration by a worker |
| Memory | recall across rooms without tools, forgetting cascade, no re-learning | `orca computer` script over days, plus store inspection |
| Load and stress | latency, queue depth, database connections under bursts | recorded payloads replayed at high rate through the real ingress code into a recording SDK sink; never through a real messaging account |
| Soak and leaks | memory growth and stuck tasks over hours | the same replay harness run for hours, sampling RSS, tasks, pool |
| Chaos | kill mid-turn, database restart, provider outage, lease takeover | replay harness plus process and container control |
| Static security | dependency audit, secret scan, static analysis | tools in the shell |

Rules:

- GUI-driven suites are deterministic scripts calling `orca computer` (accessibility text
  snapshots filtered with `--query`, no screenshots); a model drives the GUI only to explore a
  new scenario or a failure, and that model is an Orca worker, not the supervisor.
- Only dedicated test rooms are used, created by the owner; every session is approved by the
  owner first; nobody is added to a test room without their knowledge.
- Messaging accounts are never used for load: platforms ban automated volume. Load and stress
  stay at the replay harness.
- Each live run produces a short report (scenario, decision, reasons, pass or fail) plus new
  anonymized recordings; the recordings feed level 2 and 3 tests.
- A worker's `worker_done` includes the test commands it ran and their output.

### 4.6 Live evaluation set

A small set of the same scenarios runs against real models on demand (not in CI) before a
release and after model or prompt changes, scored on the same structural assertions plus a
human read of the replies.

### 4.7 Removal gate

A judge or review layer is removed only in a change that also adds or enables the scenarios
and invariant tests covering its replacement, and the change states which tests replace it.

### 4.8 Coverage matrix

Scenarios are derived from the system, not from memory, so no capability ships untested:

- **Dimensions:** capability (every registered tool, command, gateway, plugin, requirement
  kind, and job, read from the registries the capability catalog already uses) × place (direct
  chat, group, cross-room on one gateway, cross-gateway) × role (the protected role, the
  default-for-unknown role, an operator-defined role, an unknown sender) × condition (normal, refused, needs approval, failure of tool, provider, or database,
  restart mid-turn).
- **Mandatory areas:** conversation and addressing (mention, reply, tagging, plain chatter,
  forwarded, edited, deleted, view-once); cross-room and cross-gateway (dispatch from CLI to a
  group, Telegram to Discord, `relay`, `thread_history`, linked identities); the
  agent's computer and the host (`sandbox`: execution, escape attempts, domain allowlist, reading
  `.env` or other host files must fail, per-role workspace write paths, safe for non-protected roles; `shell` and `settings`: protected role only);
  files and multimodal (`read`, `write`, `sed`, `grep`, `send_file`, images, PDF,
  stickers, voice notes STT and TTS); research and web (`web_search` with citations, `fetch`,
  injection); context and freshness (casual chat, recent news, knowledge boundary, grounding of
  numbers and dates, stale remembered facts); memory (capture, cross-room recall, L3 and L4 consent, veto, forgetting,
  consolidation, skills); approvals and flows (approve, deny, expiry, wrong decider, reaction
  and button decisions, `Connect`, `ExternalAction`, `OperatorConfig`); RBAC (every role against
  every tool and command, room roles, escalation, renamed roles); presets (Assistant and Blank
  each pass their relevant scenarios); plugins (Google option A,
  enable, disable, reload, missing secrets); MCP servers (tools and prompts as roles' tools and
  commands, server down, untrusted output); automations (`cron`, `at`, `goal`, quiet hours,
  budgets, missed runs, complex and concurrent routines); long conversations; complex
  requests; durability (crash without duplicate side effects, standby and lease takeover,
  outbox); gateway lifecycle (pairing, disable, delete, reconnect, removed from a group,
  members joining and leaving); operator flows (`init`, `update`, `reload`, `setting`,
  `doctor`, `backup`, `restore`, curl install); language packs; load, soak, chaos, real
  workload, static security.
- **Coverage gate (CI):** a test lists every registered capability and every mandatory area
  and fails when one has no scenario for its relevant places, roles, and conditions. A new tool
  or command without scenarios fails CI.
- **Every defect found live becomes a permanent scenario before it is fixed.**
- **Before a release, the full matrix runs:** CI plus every live suite, and the results form the
  release report.

## 5. Configuration

- Test-only: `MONOAGENS_TEST_DATABASE_URL` (unchanged); `MONOAGENS_RECORD=1` re-records
  cassettes; `MONOAGENS_RECORD_TRANSPORT=1` records real transport payloads; live suites read
  test-room addresses and test-account credentials from a separate `.env.test`, never `.env`.

## 6. Metrics and gates

- Verification gate adds the import-linter run and the invariant tests of §4.
- Every high-severity audit finding maps to at least one test before its fix is merged.
- Scenario count and pass rate reported per phase.
- Live suites run before each release and after any gateway change; load results record p50 and
  p95 turn latency, queue depth, and pool use.

## 7. Acceptance criteria

- All invariant tests of §4 exist and fail on the current code where the audit found
  violations (proving they detect them), then pass after the fixes.
- Every seed scenario of §4 exists, initially marked expected-to-fail where the current code
  misbehaves.
- No judge is removed without the gate of §4.
- Every gateway has replay tests built from real recordings, not hand-built events.
- The coverage gate of §4.8 passes: every registered capability and mandatory area has
  scenarios for its relevant places, roles, and conditions.

## 8. Risks and mitigations

- **Cassettes go stale when prompts change.** Mitigation: structural assertions tolerate
  wording changes; re-recording is cheap and reviewed.
- **Fake gateway diverges from real ones.** Mitigation: it is built on the same SDK base class
  as real gateways; transport smoke tests per gateway stay.
- **GUI automation is brittle.** Mitigation: scripts filter accessibility text by stable labels;
  failures fall back to a worker exploring the window; CI never depends on it.
- **Recordings leak personal data.** Mitigation: anonymization runs before writing, and a test
  rejects recordings containing phone-number or JID patterns.
- **Database requirement slows local runs.** Mitigation: unchanged throwaway pgvector container.

## 9. Interaction with other ADRs

- ADR-0008: testing discipline stays; this ADR adds the layers above.
- ADR-0012: scenarios gate the retirement of the review chain.
- ADR-0013, ADR-0016: flow and memory scenarios.
- ADR-0011: architecture contracts implement its boundary enforcement.
- ADR-0019: freshness and grounding scenarios join the coverage matrix.
- ADR-0020: preset and custom-role scenarios join the coverage matrix.

## 10. Code references

- `tests/` (existing suite), `tests/support.py`, `tests/conftest.py`.
- `scripts/selfcheck.py`, `scripts/types.py`, `pyproject.toml`.
- `monoagens/engine/llm/client.py` (recording seam).

*This ADR lives at `docs/v0.1.0/adr/0017-overhaul-testing-strategy.md`.*

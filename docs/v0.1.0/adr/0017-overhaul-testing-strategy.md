# ADR-0017: Overhaul Testing Strategy — Invariant Tests, Architecture Contracts, and Golden Conversation Scenarios

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** The overhaul (ADR-0011 to ADR-0016) removes LLM judges, moves privacy enforcement, and restructures the core. Today many tests pin the compensation layers instead of the behavior, invariants such as lease fencing and non-blocking async are not tested mechanically, and observed conversation failures have no regression tests. This ADR fixes the safety net that must exist before each removal.
- **Supersedes:** in part ADR-0008 §4 (testing discipline of 4.5 is extended)
- **Related:** ADR-0011 (boundaries), ADR-0012 (envelope), ADR-0013 (flows), ADR-0016 (memory)
- **Implementation:** draft

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

### 4.4 Live evaluation set

A small set of the same scenarios runs against real models on demand (not in CI) before a
release and after model or prompt changes, scored on the same structural assertions plus a
human read of the replies.

### 4.5 Removal gate

A judge or review layer is removed only in a change that also adds or enables the scenarios
and invariant tests covering its replacement, and the change states which tests replace it.

## 5. Configuration

- Test-only: `MONOAGENS_TEST_DATABASE_URL` (unchanged); `MONOAGENS_RECORD=1` re-records
  cassettes; live evaluation needs provider keys in `.env`.

## 6. Metrics and gates

- Verification gate adds the import-linter run and the invariant tests of §4.
- Every high-severity audit finding maps to at least one test before its fix is merged.
- Scenario count and pass rate reported per phase.

## 7. Acceptance criteria

- All invariant tests of §4 exist and fail on the current code where the audit found
  violations (proving they detect them), then pass after the fixes.
- Every seed scenario of §4 exists, initially marked expected-to-fail where the current code
  misbehaves.
- No judge is removed without the gate of §4.

## 8. Risks and mitigations

- **Cassettes go stale when prompts change.** Mitigation: structural assertions tolerate
  wording changes; re-recording is cheap and reviewed.
- **Fake gateway diverges from real ones.** Mitigation: it is built on the same SDK base class
  as real gateways; transport smoke tests per gateway stay.
- **Database requirement slows local runs.** Mitigation: unchanged throwaway pgvector container.

## 9. Interaction with other ADRs

- ADR-0008: testing discipline stays; this ADR adds the layers above.
- ADR-0012: scenarios gate the retirement of the review chain.
- ADR-0013, ADR-0016: flow and memory scenarios.
- ADR-0011: architecture contracts implement its boundary enforcement.

## 10. Code references

- `tests/` (existing suite), `tests/support.py`, `tests/conftest.py`.
- `scripts/selfcheck.py`, `scripts/types.py`, `pyproject.toml`.
- `monoagens/engine/llm/client.py` (recording seam).

*This ADR lives at `docs/v0.1.0/adr/0017-overhaul-testing-strategy.md`.*

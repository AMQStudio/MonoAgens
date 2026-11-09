# ADR-0018: Overhaul Roadmap and Release Decisions — Phases, Owner Decisions, Deferred Gaps, and the Path to a Gold-Standard Core

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** ADR-0011 to ADR-0017 define the target architecture. This ADR records the order of work, the owner decisions that have no ADR of their own yet, the gaps deliberately deferred, and the release prerequisites, so the overhaul has one plan of record.
- **Supersedes:** in part ADR-0007 §4 (the installer becomes a real thin bootstrap with pinned, verified releases)
- **Related:** ADR-0011, ADR-0012, ADR-0013, ADR-0014, ADR-0015, ADR-0016, ADR-0017
- **Implementation:** draft

---

## 1. Context

The audit of 2026-11-09 (184 findings, 42 tagged high) and the owner discussion that followed
produced seven architecture ADRs and a set of decisions about install, language packs, phase
order, and documentation. The product is not released, which allows breaking changes, a
migration squash, and an ADR rebaseline before the first release.

## 2. Decision drivers

- Each phase stands on the safety net of the previous one.
- Urgent defects are fixed early without waiting for the architecture.
- Version 1 of every mechanism is the simplest that passes the golden scenarios; additions need
  evidence.
- Nothing ships to users from a mutable branch.

## 3. Options considered

1. **Big-bang rewrite.** Rejected: no safety net, long time without a working system.
2. **Fix findings one by one without phases.** Rejected: root causes keep producing findings.
3. **Ordered phases, tests first, smallest viable mechanisms (chosen).**

## 4. Decision

### 4.1 Phases

| Phase | Content | Main ADRs |
|---|---|---|
| P0 | Invariant tests and architecture contracts; urgent fixes: subject veto, approval authority, `settings get` never parks, Google refused in groups until its flow exists, lease fences on unfenced stores, Discord interaction acknowledgement and payload, empty reply on content-filtered responses | ADR-0017, ADR-0013 |
| P1 | Message envelope, speaker rendering, addressing version 1, secret redaction, taint, turn trace | ADR-0012 |
| P2 | Flow protocol and runner, requirement kinds, core and gateway events, approvals rebuilt, `/link` versus account connection, identity merge carries approvals and grants | ADR-0013 |
| P3 | Privacy by construction at context assembly; judges and review layers removed one at a time behind tests; activity stream and forgetting cascade | ADR-0012, ADR-0016 |
| P4 | `monoagens.sdk`; gateway, tool, and plugin skeleton; core tools moved into the `assistant`, `power`, and `google` tool packages; hooks version 1; expression plugin (disabled by default); presets Assistant and Blank; custom roles with `protected` and `default_for_unknown` | ADR-0014, ADR-0020 |
| P5 | String ownership and optional packs | ADR-0015 |
| P6 | Gateways onto the shared skeleton and base; Discord readiness and direct-message labels | ADR-0014 |
| P7 | Admin, CLI, configuration cleanup; thin installer and pinned releases (§4) | this ADR |
| Release | Migration squash, ADR rebaseline, README re-check | this ADR |

Dead code is deleted in whichever phase touches its area. The knowledge graph is refreshed after
each phase.

### 4.2 Owner decisions recorded here

- Expression plugin (affect, mood, vibe, style, humanize, polish, ambient): disabled by default;
  `monoagens init` offers to enable it.
- Indonesian stays in the repository as an optional pack; only English is mandatory (ADR-0015).
- Google: option A of ADR-0013 §4 (service account executes; the person proves identity with
  `openid email`; no restricted Drive scope).
- Tombstones keep a keyed hash only; no semantic fingerprint (ADR-0016 §4).
- Terminology: "plugin" for capability packages and "gateway" for bodies; never "extension"
  (PostgreSQL extensions already use that word in this project).

### 4.3 Installer and releases

- `bash <(curl -fsSL .../install.sh)` and its flags keep working.
- `install.sh` becomes a thin bootstrap: install a pinned, digest-checked `uv`; download a
  pinned release archive (tag plus published sha256, or a signature); verify it; hand off to
  `monoagens init` for everything else. Today's duplicated deployment logic in shell is removed.
- `monoagens update` uses the same pinned, verified release source; `main` is never executed on
  user hosts. A developer flag (`--revision`) allows a branch explicitly.
- Release automation publishes checksums with every tag.

### 4.4 Before the first release

- Squash the Alembic chain into one baseline after the envelope schema lands, with a reset
  script for development databases.
- Rebaseline ADRs: move `docs/v0.1.0/adr/` to `docs/archive/` and write a clean baseline that
  describes the final architecture.
- Re-check every README promise against the code.

### 4.5 Deferred gaps

| Gap | Phase or trigger |
|---|---|
| Per-person and per-room LLM budgets | P3 |
| Retention for `messages`, `turns`, `actions`, `automation_runs` | P1 or P7 |
| Admin API contract for the web panel | before the panel is built |
| Realtime voice and embodiment seam (advertised capability that always refuses today) | after P4 |
| Asynchronous database driver (psycopg async with SQLAlchemy asyncio) | separate ADR after P0 invariant tests |
| Addressing "Later" items and weight calibration (ADR-0012 §4) | when scenarios or corrections demand it |
| Google through the user's own token with `drive.file` and the Google Picker | when a web page exists |

## 5. Configuration

No new keys in this ADR. Installer flags: `--revision` (developer only) in addition to the
existing ones.

## 6. Metrics and gates

- Each phase ends with the verification gate green and its golden scenarios passing.
- The gold-standard table of ADR-0011 §6 is fully met at release.
- Release archives verify against published checksums in the installer test.

## 7. Acceptance criteria

- Every phase has its ADR moved from `draft` to `partial` to `implemented` as it lands.
- No install or update path executes code from a mutable branch by default.
- At release, `docs/v0.1.0/adr/` holds only the rebaselined set.

## 8. Risks and mitigations

- **Phases drag on.** Mitigation: P0 delivers user-visible fixes; each phase is independently
  shippable to the owner's own deployment.
- **Deferred gaps are forgotten.** Mitigation: §4 is the register; each phase reviews it.
- **Release pinning slows iteration.** Mitigation: the developer revision flag.

## 9. Interaction with other ADRs

- ADR-0007: distribution stays; the installer and update source change per §4.
- ADR-0011 to ADR-0017: this ADR orders their implementation.

## 10. Code references

- `install.sh`, `monoagens/admin/service.py` (`UPDATE_ARCHIVE`), `monoagens/admin/init.py`,
  `Dockerfile`.
- `migrations/`, `docs/v0.1.0/adr/`, `docs/archive/`, `README.md`.

*This ADR lives at `docs/v0.1.0/adr/0018-overhaul-roadmap-and-release-decisions.md`.*

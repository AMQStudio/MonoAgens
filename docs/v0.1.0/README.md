# ADR — v0.1.0

The authoritative decision records for MonoAgens v0.1.0. Each ADR states one architectural decision,
the options rejected, the configuration it introduces, metrics, and code references.

**English is the sole language** for all ADRs (`adr/NNNN-slug.md`, ADR-0008 §4.1).

> [!NOTE]
> The historical pre-rebaseline decisions (ADR 0000–0055) are preserved in `docs/archive/adr-pre-rebaseline/`.
> This directory holds the unified clean-slate baseline for v0.1.0.

## How to work with it

```bash
python -P scripts/adr.py --check            # structure gate (also run by tests/test_adr.py)
python -P scripts/adr.py --new "ADR Title"  # scaffold the next number
```

`--check` verifies the contract of ADR-0008 §4.1: canonical sections, monotonic numbering, fence-aware
headings, resolvable `§` references, register synonyms scoped to definition lines, a footer naming the
file's own path, and an `Implementation` line (`implemented`, `partial`, or `draft`).

## Index

| ADR | Title | Folder | Implementation |
|---|---|---|---|
| [0001](adr/0001-sovereign-agent-and-invariants.md) | Sovereign Agent and Core Invariants — One Soul, One Database, One Primary, and Hexagonal Seam | root | implemented |
| [0002](adr/engine/0002-unified-memory-and-privacy.md) | Unified Memory, Layered Privacy, and Detector Precision — Native PostgreSQL Hybrid Search and Word Boundaries | engine | implemented |
| [0003](adr/engine/0003-turn-lifecycle-lease-and-durability.md) | Turn Lifecycle, Primary Lease, and Crash Safety — Atomic Fencing, Non-Blocking Loop, and Idempotent Resume | engine | implemented |
| [0004](adr/engine/0004-core-tools-and-cross-gateway-messaging.md) | Core Tools and Cross-Gateway Messaging — Universal Delivery, Origin Verification, and Tool Contracts | engine | implemented |
| [0005](adr/engine/0005-unified-job-engine-and-extensions.md) | Unified Job Engine and Extension Architecture — Workflows, Tasks, Plugins, and Provider Adapters | engine | implemented |
| [0006](adr/engine/0006-access-control-roles-and-guard.md) | Access Control, Role-Based Security, and Sandbox Guard — RBAC, Identity Linking, and Command Screening | engine | implemented |
| [0007](adr/0007-distribution-install-and-initialization.md) | Distribution, Installation, and Initialization — Terminal Wizard, Thin Bootstrap, and Container Packaging | root | implemented |
| [0008](adr/0008-modular-architecture-and-code-hygiene.md) | Modular Architecture, Code Hygiene, and Honest Budgets — ADR Home, Typed Pipeline, and Testing Discipline | root | implemented |
| [0009](adr/engine/0009-language-agnostic-intent-and-privacy-routing.md) | Language-Agnostic Intent, Affect, and Privacy Routing — Retiring Static Per-Language JSON Wordlists | engine | implemented |
| [0010](adr/0010-deep-native-gateway-embodiment-and-runtime-layers.md) | Deep Native Gateway Embodiment and Runtime Layer Architecture — Discord, Telegram, Slack, WhatsApp Completeness, and Layered Extensions | root | implemented |
| [0011](adr/0011-layer-boundaries-and-core-charter.md) | Layer Boundaries and Core Charter — Core as Engine, Soul, Moral, Privacy, and Memory; Gateways as Body and UX; Plugins as Capability | root | draft |
| [0012](adr/engine/0012-message-envelope-and-privacy-by-construction.md) | Message Envelope and Privacy by Construction — Label Once at Ingress, Render Speakers Explicitly, Filter Context by Audience | engine | draft |
| [0013](adr/engine/0013-flow-protocol-requirements-and-approvals.md) | Flow Protocol, Requirements, and Generic Approvals — Plugins Declare Needs, the Core Runs Durable Flows, Gateways Render | engine | draft |
| [0014](adr/0014-gateway-and-plugin-packaging-and-hooks.md) | Gateway and Plugin Packaging and Hooks — Two Concepts, One Skeleton, Typed Deep Integration | root | draft |
| [0015](adr/0015-string-ownership-and-optional-language-packs.md) | String Ownership and Optional Language Packs — English Mandatory, Every Owner Keeps Its Strings | root | draft |
| [0016](adr/engine/0016-self-model-activity-stream-and-memory-layers.md) | Self-Model, Activity Stream, and Human-Like Memory Layers — Remember What I Just Did, Everywhere, Without Re-Reading Threads | engine | draft |
| [0017](adr/0017-overhaul-testing-strategy.md) | Overhaul Testing Strategy — Invariant Tests, Architecture Contracts, and Golden Conversation Scenarios | root | draft |
| [0018](adr/0018-overhaul-roadmap-and-release-decisions.md) | Overhaul Roadmap and Release Decisions — Phases, Owner Decisions, Deferred Gaps, and the Path to a Gold-Standard Core | root | draft |
| [0019](adr/engine/0019-grounding-freshness-and-research.md) | Grounding, Freshness, and Research — Know What Time It Is, Know What You Do Not Know, Never Guess Current Facts | engine | draft |

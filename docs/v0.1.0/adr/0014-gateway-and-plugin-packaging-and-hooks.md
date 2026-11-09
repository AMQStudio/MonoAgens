# ADR-0014: Gateway and Plugin Packaging and Hooks — Two Concepts, One Skeleton, Typed Deep Integration

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** Gateways live inside the package with a Python descriptor; plugins live in `plugins/` with a YAML manifest that can also claim gateways. Plugin hooks offer only static prompt literals, so persona and emotion cannot leave the core, plugin tables are created outside Alembic, and declared secrets do not gate tool exposure. This ADR fixes one package skeleton for both concepts, keeps them distinct for operators, and defines the typed hooks plugins use for deep integration.
- **Supersedes:** in part ADR-0005 §4 (gateway lifecycle of 4.2 and plugin loader of 4.3), ADR-0010 §4 (reserved runtime layer contract of 4.6)
- **Related:** ADR-0011 (charter and trust tiers), ADR-0013 (flows), ADR-0015 (strings)
- **Implementation:** draft

---

## 1. Context

Audit evidence (`.audit/05-tools-plugins.md`, `.audit/01-ingress-gateways.md`,
`.audit/06-jobs.md`):

- Plugin hooks are static literal prompt parts; plugin handlers merge into the core registry.
- `plugins/google/plugin.yaml` declares one secret while the login flow needs three; tools are
  offered regardless of configured-ness.
- Plugin tables are created at runtime, outside the Alembic chain.
- Gateways are four near-identical implementations, and gateway strings live in the core
  catalog.
- Persona and cognition jobs (ambient) live in the core for lack of a periodic-job hook.

## 2. Decision drivers

- Operators see gateways and plugins as different things (CLI groups, panel sections).
- Developers learn one package layout.
- Plugins may reshape runtime and persona, never bypass invariants.
- Missing configuration hides a capability instead of failing in front of a user.

## 3. Options considered

1. **One merged concept, gateway as a plugin kind.** Rejected by the owner: the panel and CLI
   treat them differently, and the distinction helps operators.
2. **Keep two unrelated mechanisms.** Rejected: duplicated loaders, digests, and installers.
3. **Two concepts, one skeleton, shared internals (chosen).**

## 4. Decision

### 4.1 Package skeleton

```
gateways/<name>/            plugins/<name>/
  gateway.yaml                plugin.yaml
  lang/en.yaml                lang/en.yaml
  config.py                   config.py
  migrations/                 migrations/
  <code>                      <code>
  tests/                      tests/
```

`monoagens.sdk` follows semantic versioning: a breaking change raises the major version, and
each manifest declares the supported range in `sdk`. Manifest fields shared by both: `name`, `version`, `sdk` (supported `monoagens.sdk` range),
`requires` (Python dependencies), `secrets` (all keys the package needs), `digest`, `credits`.
Gateways add `capabilities` (rendering and transport features). Plugins add `hooks` (§4) and
`provides` (`tools`, `commands`, `flows`); `provides.gateways` is removed.

### 4.2 Shared internals, separate surfaces

- One manifest parser, digest verifier, dependency installer, and migration runner serve both.
- Separate contracts, CLI groups (`monoagens gateway ...`, `monoagens plugins ...`), and panel
  sections.
- Each package owns its Alembic branch; `monoagens db upgrade` applies core then enabled
  packages. Runtime DDL is forbidden.
- A package whose `secrets` are missing is listed as `needs configuration`; its tools are not
  offered to the model (ADR-0013 `OperatorConfig`).

### 4.3 Typed hooks for plugins

Before implementation, the hook registry is compared with pluggy (the hook library behind
pytest: ordering, first-result calls, hookspec validation, documented through Context7). The
choice is pluggy unless the guarantees below cannot be enforced through it; a thin own registry
is the fallback.

| Hook | Purpose | Guarantee enforced by the core |
|---|---|---|
| Later: `envelope_enricher` | add non-authoritative labels (for example a mood estimate) | cannot change `speaker`, `addressed_to`, `privacy`, `trust` |
| `context_contributor` | add prompt parts with priority and token budget | runs after audience filtering; sees only allowed content |
| `tool_provider`, `command_provider` | add tools and commands | every call passes the tool door and RBAC |
| `flow` | plugin flows (ADR-0013) | durable runner, lease, decision validation |
| `outbound_transformer` | style, pacing, bubble splitting | cannot change audience or add content outside the draft |
| `post_turn_observer` | learn after a turn (affect update, style) | read-only view of the turn |
| `periodic_job` | background work (ambient) | runs only on the primary, fenced |
| Later: `service` | expose state to other plugins (for example affect) | typed handle through the SDK |

Version 1 ships the hooks without the "Later" prefix; a later hook is added when a plugin needs it.
A plugin receives only the hooks it declares. Hooks are first-party only (ADR-0011 trust
tiers); third-party capability uses MCP.

### 4.4 First consumers

- `plugins/expression/` (name may change): affect, mood, vibe, style, humanize, polish,
  ambient, voice sampling knobs, `mode`/`modes`; disabled by default.
- `plugins/google/`: tools plus the flow of ADR-0013 §4; secrets completed.
- `gateways/whatsapp`, `telegram`, `discord`, `slack`: moved to the skeleton one per change,
  sharing a common base in the SDK.

## 5. Configuration

- `config.yaml` keeps `gateways.<name>.enabled` and `plugins.<name>.enabled`; package settings
  live under the package's own section, validated by the package's `config.py`.
- The expression plugin is disabled by default; `monoagens init` offers to enable it.
- Per-role gateway policy in `roles.yaml` uses one vocabulary for every gateway (it replaces
  `groups: none | mention | reply | both | all`), generated with these comments:

```yaml
member:
  gateways:
    whatsapp:
      # Who may talk to the Soul through this gateway, and how.
      dm: true               # true = this role may talk to the Soul in a private chat.
                             # false = private messages are ignored; a refusal is sent at most
                             # once per access_notice_hours.
      groups: addressed      # none = group messages are only kept as context, never answered.
                             # addressed = answered only when meant for the Soul
                             #   (see engine.addressing in config.yaml: mention, reply, continuation).
      skepticism: normal     # strict | normal | lenient: how sure the Soul must be that a group
                             # message is for it before answering. strict suits busy groups.
      initiative: false      # true = with the expression plugin enabled, the Soul may join a
                             # group conversation unasked. Never for messages meant for someone else.
```

  A room in `roles.yaml` `rooms:` may override `skepticism` for that room only.

## 6. Metrics and gates

- Import-linter: gateways and plugins import only `monoagens.sdk`.
- No `CREATE TABLE` outside Alembic (test over SQL emitted at runtime).
- A package with missing secrets exposes zero tools (test).

## 7. Acceptance criteria

- The four gateways and the Google plugin load through the shared parser and verifier.
- Persona and emotion run entirely from the expression plugin; disabling it removes their
  prompt parts, stores, and jobs without errors.
- `monoagens gateway list` and `monoagens plugins list` show disjoint sets.

## 8. Risks and mitigations

- **Hook surface becomes a long-term API.** Mitigation: ship only the hooks of §4 that a first
  consumer needs; new hooks require an ADR.
- **Alembic multi-branch ordering.** Mitigation: core first, packages depend on a named core
  revision.
- **Moving gateways breaks pairing state.** Mitigation: session files keep their paths; one
  gateway per change with its tests.

## 9. Interaction with other ADRs

- ADR-0005: job engine stays; plugin loader and gateway lifecycle are replaced.
- ADR-0010: per-gateway feature lists remain; the reserved layer contract is replaced by §4.
- ADR-0011: implements the gateway and plugin areas and trust tiers.
- ADR-0013: plugins declare flows and requirements.
- ADR-0015: each package owns its strings.

## 10. Code references

- `monoagens/engine/plugins/` (loader, registry), `monoagens/admin/extensions.py`.
- `plugins/google/plugin.yaml`, `plugins/google/plugin.py`.
- `monoagens/gateways/base.py`, `monoagens/gateways/*/descriptor.py`.
- `monoagens/engine/jobs/ambient.py`, `monoagens/engine/stores/affect.py`,
  `monoagens/engine/stores/vibe.py`, `monoagens/engine/character/`.

*This ADR lives at `docs/v0.1.0/adr/0014-gateway-and-plugin-packaging-and-hooks.md`.*

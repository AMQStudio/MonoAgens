# ADR-0014: Gateway, Tool, and Plugin Packaging and Hooks — Three Concepts, One Skeleton, Typed Deep Integration

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** Gateways live inside the package with a Python descriptor; plugins live in `plugins/` with a YAML manifest that can also claim gateways; tools are hard-wired into the core whatever the deployment is for. Plugin hooks offer only static prompt literals, so persona and emotion cannot leave the core, plugin tables are created outside Alembic, and declared secrets do not gate tool exposure. This ADR fixes three concepts that operators and developers can tell apart at a glance, one package skeleton for all three, and the typed hooks plugins use for deep integration.
- **Supersedes:** in part ADR-0005 §4 (gateway lifecycle of 4.2 and plugin loader of 4.3), ADR-0010 §4 (reserved runtime layer contract of 4.6), ADR-0004 §4 (core tools of 4.1 become tool packages)
- **Related:** ADR-0011 (charter and trust tiers), ADR-0013 (flows), ADR-0015 (strings), ADR-0020 (framework and presets)
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
- Every deployment gets a generic-assistant toolset, although deployments may be customer
  service, coaching, personal, or organization assistants (ADR-0020).

## 2. Decision drivers

- An operator can tell each concept apart from one sentence.
- Developers learn one package layout.
- Plugins may reshape runtime and persona, never bypass invariants.
- Missing configuration hides a capability instead of failing in front of a user.
- Follow existing standards: MCP is a tool protocol, so MCP servers are tools.

## 3. Options considered

1. **One merged concept.** Rejected: the panel and CLI treat bodies, tools, and runtime
   extensions differently.
2. **Two concepts, gateway and plugin, with tools inside plugins.** Rejected: a package that
   only adds tools is not a runtime extension, and MCP would have no natural place.
3. **Three concepts, one skeleton, shared internals (chosen).**

## 4. Decision

### 4.1 Three concepts

| Concept | Panel label | One sentence | Examples |
|---|---|---|---|
| Gateway | App | the Soul's application: where it lives and how it looks there | WhatsApp, Telegram, Discord, Slack, CLI |
| Tool | Tool | the Soul's instrument: something the model calls | `assistant` pack, `power` pack, `google`, any MCP server |
| Plugin | Plugin | an additional ability that changes how the Soul behaves; it may also bring tools and commands | `expression` (affect, humanize), a news follower |

Rule of thumb: called by the model, it is a tool; changes how the Soul thinks or speaks, it is
a plugin; a place where the Soul lives, it is a gateway. A package that only adds tools is a
tool package, not a plugin. A tool package may carry commands that belong to its tools (for
example `/task` with `task`).

### 4.2 Package skeleton

```
gateways/<name>/        tools/<name>/          plugins/<name>/
  gateway.yaml            tool.yaml              plugin.yaml
  lang/en.yaml            lang/en.yaml           lang/en.yaml
  config.py               config.py              config.py
  migrations/             migrations/            migrations/
  <code>                  <code>                 <code>
  tests/                  tests/                 tests/
```

`monoagens.sdk` follows semantic versioning: a breaking change raises the major version, and
each manifest declares the supported range in `sdk`. Manifest fields shared by all three:
`name`, `version`, `sdk`, `requires` (Python dependencies), `secrets` (all keys the package
needs), `digest`, `credits`. Gateways add `capabilities` (rendering and transport features).
Tool packages add `provides` (`tools`, `commands`, `flows`). Plugins add `hooks` (§4) and may
add `provides`. An MCP server is registered as a tool entry with its launch command and is run
out of process.

### 4.3 Shared internals, separate surfaces

- One manifest parser, digest verifier, dependency installer, and migration runner serve all
  three.
- Separate contracts, CLI groups (`monoagens gateway ...`, `monoagens tools ...`,
  `monoagens plugins ...`), and panel sections (Apps, Tools, Plugins).
- Each package owns its Alembic branch; `monoagens db upgrade` applies core then enabled
  packages. Runtime DDL is forbidden.
- A package whose `secrets` are missing is listed as `needs configuration`; its tools are not
  offered to the model (ADR-0013 `OperatorConfig`).
- Individual tools inside an enabled pack can be switched off in the panel; this is an
  advanced setting, not a choice the operator has to make.
- Every tool, first-party or MCP, passes the same tool door and RBAC.

### 4.4 First-party tool packages

| Package | Tools | Commands | Default |
|---|---|---|---|
| `assistant` | `read`, `grep`, `write`, `sed`, `send_file`, `web_search`, `fetch`, `send_message`, `relay`, `task`, `recipe` | `/send`, `/relay`, `/task` | on in the Assistant preset |
| `power` | `sandbox`, `shell` (protected role only), `settings`, MCP client bridge | — | off |
| `google` | `google` with the flow of ADR-0013 §4 | — | off |

The machinery stays in the core: the job engine behind `task`, `Agent.dispatch()` behind
`send_message` and `relay`, the sandbox runtime and safety policy behind `sandbox` and `shell`.
A package provides the door, never its own machinery.

### 4.5 Typed hooks for plugins

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

Version 1 ships the hooks without the "Later" prefix; a later hook is added when a plugin needs
it. A plugin receives only the hooks it declares. Plugins are first-party only (ADR-0011 trust
tiers); third-party capability arrives as MCP tools.

### 4.6 First consumers

- `plugins/expression/`: affect, mood, vibe, style, humanize, polish, ambient, voice sampling
  knobs, `mode`/`modes`, and the `/mood` and `/calmdown` commands; disabled by default.
- `tools/assistant/`, `tools/power/`, `tools/google/`: moved out of `monoagens/engine/tools/`
  and `plugins/google/`.
- `gateways/whatsapp`, `telegram`, `discord`, `slack`, `cli`: moved to the skeleton one per
  change, sharing a common base in the SDK; `/steps`, `/thinking` move into that base.

## 5. Configuration

- `config.yaml` keeps `gateways.<name>.enabled`, `tools.<name>.enabled`, and
  `plugins.<name>.enabled`; package settings live under the package's own section, validated by
  the package's `config.py`.
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

  A room in `roles.yaml` `rooms:` may override `skepticism` for that room only. Role names are
  free (ADR-0020); `member` above is only an example.

## 6. Metrics and gates

- Import-linter: gateways, tools, and plugins import only `monoagens.sdk`.
- No `CREATE TABLE` outside Alembic (test over SQL emitted at runtime).
- A package with missing secrets exposes zero tools (test).
- A tool-only package declaring hooks fails validation (it must be a plugin).

## 7. Acceptance criteria

- The gateways, the three tool packages, and the expression plugin load through the shared
  parser and verifier.
- Persona and emotion run entirely from the expression plugin; disabling it removes their
  prompt parts, stores, and jobs without errors.
- With every tool package and plugin disabled, the Soul still runs with the core of ADR-0011.
- `monoagens gateway list`, `monoagens tools list`, and `monoagens plugins list` show disjoint
  sets.

## 8. Risks and mitigations

- **Hook surface becomes a long-term API.** Mitigation: ship only the hooks of §4 that a first
  consumer needs; new hooks require an ADR.
- **Alembic multi-branch ordering.** Mitigation: core first, packages depend on a named core
  revision.
- **Moving gateways breaks pairing state.** Mitigation: session files keep their paths; one
  gateway per change with its tests.
- **Grey area between tool package and plugin.** Mitigation: the rule of §4: hooks make a
  plugin; no hooks make a tool package.

## 9. Interaction with other ADRs

- ADR-0004: tool contracts stay; the tools move into packages.
- ADR-0005: job engine stays; plugin loader and gateway lifecycle are replaced.
- ADR-0010: per-gateway feature lists remain; the reserved layer contract is replaced by §4.
- ADR-0011: implements the gateway, tool, and plugin areas and trust tiers.
- ADR-0013: tool packages and plugins declare flows and requirements.
- ADR-0015: each package owns its strings.
- ADR-0020: presets choose which packages are enabled.

## 10. Code references

- `monoagens/engine/tools/`, `monoagens/engine/plugins/` (loader, registry),
  `monoagens/admin/extensions.py`.
- `plugins/google/plugin.yaml`, `plugins/google/plugin.py`.
- `monoagens/gateways/base.py`, `monoagens/gateways/*/descriptor.py`.
- `monoagens/engine/jobs/ambient.py`, `monoagens/engine/stores/affect.py`,
  `monoagens/engine/stores/vibe.py`, `monoagens/engine/character/`.

*This ADR lives at `docs/v0.1.0/adr/0014-gateway-and-plugin-packaging-and-hooks.md`.*

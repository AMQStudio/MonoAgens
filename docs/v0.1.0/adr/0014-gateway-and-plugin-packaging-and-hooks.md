# ADR-0014: Gateway and Plugin Packaging and Hooks — Apps and Plugins, One Skeleton, Per-Role Toggles

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** Gateways live inside the package with a Python descriptor; plugins live in `plugins/` with a YAML manifest that can also claim gateways; tools are hard-wired into the core whatever the deployment is for. Plugin hooks offer only static prompt literals, so persona and emotion cannot leave the core, plugin tables are created outside Alembic, and declared secrets do not gate tool exposure. This ADR fixes two concepts that operators tell apart at a glance, gateways (the Soul's apps) and plugins (everything it can do beyond the core), one package skeleton, per-role toggles for every tool and command, and the typed hooks plugins use for deep integration.
- **Supersedes:** in part ADR-0005 §4 (gateway lifecycle of 4.2 and plugin loader of 4.3), ADR-0010 §4 (reserved runtime layer contract of 4.6), ADR-0004 §4 (core tools of 4.1 move into plugins)
- **Related:** ADR-0011 (charter and trust tiers), ADR-0013 (flows), ADR-0015 (strings), ADR-0020 (framework, presets, roles)
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

- As few concepts as possible; an operator understands each from one sentence.
- Developers learn one package layout.
- Every tool and command can be switched on or off, and allowed per role.
- Plugins may reshape runtime and persona, never bypass invariants.
- Missing configuration hides a capability instead of failing in front of a user.

## 3. Options considered

1. **Three concepts: gateway, tool package, plugin.** Rejected: the line between a tool package
   and a plugin kept moving (Google has tools, tables, accounts, and flows), and operators gain
   nothing from the distinction.
2. **Everything one concept, gateways included.** Rejected: bodies and capabilities have
   different contracts, CLI groups, and panel sections.
3. **Two concepts, gateways and plugins, with per-role toggles inside plugins (chosen).**

## 4. Decision

### 4.1 Two concepts

| Concept | Panel label | One sentence | Examples |
|---|---|---|---|
| Gateway | App | the Soul's application: where it lives and how it looks there | WhatsApp, Telegram, Discord, Slack, CLI |
| Plugin | Plugin | everything the Soul can do beyond the core; it may bring tools (the model calls them), commands (a person types them), flows, hooks, and its own data | `files`, `research`, `google`, `expression` |

Terms: a **tool** is one instrument the model calls; a **command** is typed by a person; both
come from plugins (or from the core, ADR-0011 §4).

### 4.2 Package skeleton

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
each manifest declares the supported range in `sdk`. Shared manifest fields: `name`, `version`,
`sdk`, `requires` (Python dependencies), `secrets` (all keys the package needs), `digest`,
`credits`. Gateways add `capabilities` (rendering and transport features). Plugins add
`provides` (`tools`, `commands`, `flows`) and `hooks` (§4); both are optional, and the panel
shows what each plugin uses (tools, commands, flows, hooks, own tables, linked accounts) so the
operator sees how deep a plugin reaches.

### 4.3 Shared internals, separate surfaces

- One manifest parser, digest verifier, dependency installer, and migration runner serve both.
- Separate contracts, CLI groups (`monoagens gateway ...`, `monoagens plugins ...`), and panel
  sections (Apps, Plugins).
- Each package owns its Alembic branch; `monoagens db upgrade` applies core then enabled
  packages. Runtime DDL is forbidden.
- A package whose `secrets` are missing is listed as `needs configuration`; its tools are not
  offered to the model (ADR-0013 `OperatorConfig`).
- Every tool and command, from the core, a plugin, or MCP, passes the same tool or command door
  and RBAC.

### 4.4 Toggles

Two independent switches, both shown in the panel:

- **Deployment toggle:** a plugin is enabled or disabled; inside an enabled plugin each tool and
  command can be switched off for everyone.
- **Role toggle:** `roles.yaml` lists which tools and commands each role may use, per tool or
  per whole plugin (vocabulary in ADR-0020 §4). The panel renders this as a matrix of roles
  against the tools and commands of enabled plugins.

A tool is offered to the model only when its plugin is enabled and configured, the tool is not
switched off, and the sender's role allows it.

### 4.5 First-party plugins

| Plugin | Tools | Commands | Also | Default |
|---|---|---|---|---|
| `files` | `read`, `grep`, `write`, `sed`, `send_file` | `/send` | — | on in the Assistant preset |
| `research` | `web_search`, `fetch` | — | — | on in the Assistant preset |
| `messaging` | `send_message`, `relay` | `/relay` | — | on in the Assistant preset |
| `automation` | `task`, `recipe` | `/task` | — | on in the Assistant preset |
| `system` | `sandbox`, `shell` (protected role only), `settings` | — | — | off |
| `mcp` | the tools of each configured MCP server, named `mcp:<server>/<tool>` | — | runs servers out of process | off |
| `google` | `google` | — | own tables, linked Google accounts, the flow of ADR-0013 §4 | off |
| `expression` | — | `/mood`, `/calmdown` | hooks: affect, mood, vibe, style, humanize, polish, ambient | off |

The machinery stays in the core: the job engine behind `task`, `Agent.dispatch()` behind
`send_message` and `relay`, the sandbox runtime and safety policy behind `sandbox` and `shell`,
the MCP client behind `mcp`. A plugin provides the door, never its own copy of core machinery.

### 4.6 Typed hooks

Before implementation, the hook registry is compared with pluggy (the hook library behind
pytest: ordering, first-result calls, hookspec validation, documented through Context7). The
choice is pluggy unless the guarantees below cannot be enforced through it; a thin own registry
is the fallback.

| Hook | Purpose | Guarantee enforced by the core |
|---|---|---|
| Later: `envelope_enricher` | add non-authoritative labels (for example a mood estimate) | cannot change `speaker`, `addressed_to`, `privacy`, `trust` |
| `context_contributor` | add prompt parts with priority and token budget | runs after audience filtering; sees only allowed content |
| `outbound_transformer` | style, pacing, bubble splitting | cannot change audience or add content outside the draft |
| `post_turn_observer` | learn after a turn (affect update, style) | read-only view of the turn |
| `periodic_job` | background work (ambient) | runs only on the primary, fenced |
| Later: `service` | expose state to other plugins (for example affect) | typed handle through the SDK |

Tools, commands, and flows are declared in `provides`, not as hooks. Version 1 ships the hooks
without the "Later" prefix; a later hook is added when a plugin needs it. A plugin receives only
the hooks it declares. In-process plugins are first-party only (ADR-0011 trust tiers);
third-party capability arrives through the `mcp` plugin, out of process.

### 4.7 First consumers

- The plugins of §4, moved out of `monoagens/engine/tools/`, `monoagens/engine/commands/`,
  and `plugins/google/`.
- `gateways/whatsapp`, `telegram`, `discord`, `slack`, `cli`: moved to the skeleton one per
  change, sharing a common base in the SDK; `/steps`, `/thinking` move into that base.

## 5. Configuration

- `config.yaml`, generated with comments:

```yaml
plugins:
  files:
    enabled: true            # Turn the whole plugin on or off for this deployment.
    disabled_tools: []       # Tools of this plugin switched off for everyone, e.g. [sed].
    disabled_commands: []    # Commands of this plugin switched off for everyone, e.g. [send].
  system:
    enabled: false           # sandbox, shell, settings. Off by default: these act on the host.
gateways:
  whatsapp:
    enabled: true            # Run this app. Who may talk through it is set per role in roles.yaml.
```

- The expression plugin is disabled by default; `monoagens init` offers to enable it.
- Per-role gateway policy in `roles.yaml` uses one vocabulary for every gateway (it replaces
  `groups: none | mention | reply | both | all`), generated with these comments:

```yaml
staff:
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
  free (ADR-0020); `staff` above is only an example.

## 6. Metrics and gates

- Import-linter: gateways and plugins import only `monoagens.sdk`.
- No `CREATE TABLE` outside Alembic (test over SQL emitted at runtime).
- A package with missing secrets exposes zero tools (test).
- A tool is offered only when plugin enabled, configured, not switched off, and allowed for the
  role (test over the full matrix).

## 7. Acceptance criteria

- The gateways and the plugins of §4 load through the shared parser and verifier.
- Persona and emotion run entirely from the expression plugin; disabling it removes their
  prompt parts, stores, and jobs without errors.
- With every plugin disabled, the Soul still runs with the core of ADR-0011.
- `monoagens gateway list` and `monoagens plugins list` show disjoint sets.

## 8. Risks and mitigations

- **Hook surface becomes a long-term API.** Mitigation: ship only the hooks of §4 that a first
  consumer needs; new hooks require an ADR.
- **Plugins of very different depth look alike.** Mitigation: the panel shows what each plugin
  uses (tools, commands, flows, hooks, tables, accounts).
- **Alembic multi-branch ordering.** Mitigation: core first, packages depend on a named core
  revision.
- **Moving gateways breaks pairing state.** Mitigation: session files keep their paths; one
  gateway per change with its tests.

## 9. Interaction with other ADRs

- ADR-0004: tool contracts stay; the tools move into plugins.
- ADR-0005: job engine stays; plugin loader and gateway lifecycle are replaced.
- ADR-0010: per-gateway feature lists remain; the reserved layer contract is replaced by §4.
- ADR-0011: implements the gateway and plugin areas and trust tiers.
- ADR-0013: plugins declare flows and requirements.
- ADR-0015: each package owns its strings.
- ADR-0020: presets choose which plugins are enabled; roles allow tools and commands.

## 10. Code references

- `monoagens/engine/tools/`, `monoagens/engine/commands/`, `monoagens/engine/plugins/`
  (loader, registry), `monoagens/admin/extensions.py`.
- `plugins/google/plugin.yaml`, `plugins/google/plugin.py`.
- `monoagens/gateways/base.py`, `monoagens/gateways/*/descriptor.py`.
- `monoagens/engine/jobs/ambient.py`, `monoagens/engine/stores/affect.py`,
  `monoagens/engine/stores/vibe.py`, `monoagens/engine/character/`.

*This ADR lives at `docs/v0.1.0/adr/0014-gateway-and-plugin-packaging-and-hooks.md`.*

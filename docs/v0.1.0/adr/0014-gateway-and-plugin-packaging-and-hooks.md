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

### 4.1 Two concepts and one alternative

| Concept | Panel label | One sentence | Examples |
|---|---|---|---|
| Gateway | App | the Soul's application: where it lives and how it looks there | WhatsApp, Telegram, Discord, Slack, CLI |
| Plugin | Plugin | everything the Soul can do beyond the core; it may bring tools (the model calls them), commands (a person types them), flows, hooks, and its own data | `computer`, `research`, `google`, `expression` |
| MCP server (alternative to a plugin) | MCP | the same kind of capability from outside, run out of process; it brings tools and commands but no hooks, flows, or own data | a GitHub or a calendar MCP server |

An MCP server is not a plugin and not a third concept of the Soul: it is the other way to add
capability. It has its own entry in the configuration, its own panel section, and its own CLI
group, and it appears in roles exactly like a plugin does (§4.5, ADR-0020 §4).

Terms: a **tool** is one instrument the model calls; a **command** is typed by a person; both
come from plugins, MCP servers, or the core (ADR-0011 §4).

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
- Separate contracts, CLI groups (`monoagens gateway ...`, `monoagens plugins ...`,
  `monoagens mcp ...`), and panel sections (Apps, Plugins, MCP).
- Each package owns its Alembic branch; `monoagens db upgrade` applies core then enabled
  packages. Runtime DDL is forbidden.
- A package whose `secrets` are missing is listed as `needs configuration`; its tools are not
  offered to the model (ADR-0013 `OperatorConfig`).
- Every tool and command, from the core, a plugin, or MCP, passes the same tool or command door
  and RBAC.

### 4.4 Toggles

Two independent switches, both shown in the panel:

- **Deployment toggle:** a plugin or an MCP server is enabled or disabled; inside an enabled one
  each tool and command can be switched off for everyone.
- **Role toggle:** `roles.yaml` lists which tools and commands each role may use, per tool or
  per whole plugin (vocabulary in ADR-0020 §4). The panel renders this as a matrix of roles
  against the tools and commands of enabled plugins.

A tool is offered to the model only when its plugin is enabled and configured, the tool is not
switched off, and the sender's role allows it.

### 4.5 First-party plugins

| Plugin | Tools | Commands | Also | Default |
|---|---|---|---|---|
| `computer` | `read`, `grep`, `write`, `sed`, `send_file`, `sandbox` | `/send` | the Agent's own computer: files in the workspace, and commands in an OS-level sandbox that cannot read the host's secrets or write outside the role's workspace (runtime below); safe to grant to any role; without the sandbox runtime's requirements only the `sandbox` tool is withheld (`needs configuration`) | on in the Assistant preset |
| `research` | `web_search`, `fetch` | — | search the internet and open pages | on in the Assistant preset |
| `messaging` | `send_message`, `relay` | `/relay` | — | on in the Assistant preset |
| `automation` | `task`, `recipe` | `/task` | — | on in the Assistant preset |
| `google` | `google` | — | own tables, linked Google accounts, the flow of ADR-0013 §4 | off |
| `host` | `shell`, `settings` | — | the operator's machine: `shell` runs commands directly on the host, `settings` changes the Soul's own configuration from chat; protected role only | off |
| `expression` | — | `/mood`, `/calmdown` | hooks: affect, mood, vibe, style, humanize, polish, ambient | off |

The machinery stays in the core: the job engine behind `task`, `Agent.dispatch()` behind
`send_message` and `relay`, the sandbox runtime and safety policy behind `sandbox` and `shell`,
the configuration writer behind `settings`, the MCP client behind MCP servers. A plugin provides
the door, never its own copy of core machinery.

#### Sandbox runtime of the `sandbox` tool

The `sandbox` tool runs commands through Anthropic's sandbox-runtime (`srt`: bubblewrap,
seccomp, and namespaces on Linux; Seatbelt on macOS) instead of Podman or Docker. There is no
image and no daemon. Facts from its documentation, which the design depends on:

- **Network is denied by default**; traffic can only leave through proxies on the host that
  enforce a domain allowlist. This replaces the Podman egress proxy.
- **Writes are allow-only**: the generated policy allows only the role's workspace path.
- **Reads are allowed everywhere by default.** That is unsafe for a role that is not
  protected (`.env`, `config.yaml`, other roles' workspaces would be readable), so MonoAgens
  always generates a deny-then-allow policy: deny the deployment and home roots, re-allow only
  the role's workspace and system paths. If the policy cannot be built, the tool is withheld
  (fail closed). A test must prove that `.env` cannot be read from the sandbox.
- **Per call, a generated settings file** is passed with `srt --settings`; the default
  `~/.srt-settings.json` is never used. The process is started through the bounded executor or
  asyncio subprocess, never blocking the event loop.
- **Requirements**: Node.js with `@anthropic-ai/sandbox-runtime` installed (`npm install -g`),
  plus `bwrap` and `socat` on Linux. `monoagens init` checks them; if any is missing only the
  `sandbox` tool reports `needs configuration`. The package is pre-1.0, so the version is pinned
  and recorded in the release.
- **Limits accepted**: the kernel is shared with the host (weaker than a microVM), and the
  Agent cannot install system packages, only things inside its workspace. A microVM backend
  (for example microsandbox) can be added later behind the same tool through its own ADR.
- `AGENTS.md` §5 ("rootless container isolation") is updated to "OS-level sandbox" when this is
  implemented.

#### MCP servers: the alternative to a plugin

Third-party capability that cannot be reviewed and digest-verified arrives as an MCP server, run
out of process (ADR-0011 trust tiers). The MCP specification defines tools (called by the model)
and prompts (user-controlled templates that clients typically expose as slash commands), so an
MCP server maps onto the two lists a role already has:

| MCP primitive | Becomes | Role list | Name |
|---|---|---|---|
| Tool | a tool the model calls | `tools` | `mcp:<server>/<tool>` |
| Prompt | a command a person types | `commands` | `mcp:<server>/<prompt>` |

- Resources and sampling are not exposed in version 1.
- An MCP server cannot use hooks, flows, or own tables; its output is `untrusted_content`
  (ADR-0012 taint).
- A server with missing secrets, or that fails to start, is listed as `needs configuration` and
  offers nothing.
- Both primitives pass the same tool or command door and RBAC as every other tool and command.

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
third-party capability arrives as MCP servers, out of process.

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
  computer:
    enabled: true            # The Agent's own computer: workspace files and an OS-level sandbox for commands.
    sandbox_allowed_domains: []  # Domains the sandbox may reach. Empty = no network at all.
  host:
    enabled: false           # shell (commands on the host) and settings (change the Soul's config from chat).
                             # Off by default; protected role only.
mcp:
  servers:
    github:                  # An MCP server is not a plugin: it has its own entry here.
      enabled: true          # Its tools and prompts appear in roles as mcp:github/<name>.
      command: [npx, -y, "@modelcontextprotocol/server-github"]
      disabled_tools: []     # Tools of this server switched off for everyone.
      disabled_commands: []  # Prompts of this server switched off for everyone.
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
- **The sandbox reads host files by default.** Mitigation: the generated deny-then-allow read
  policy of §4.5 plus a test that `.env` is unreadable; the tool is withheld if the policy
  cannot be built.
- **sandbox-runtime is pre-1.0 and adds Node.js as a dependency.** Mitigation: pinned version,
  checked by `monoagens init`, only the `sandbox` tool depends on it.

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

# Plugins — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the core
([constitution §1](constitution.md)). Decision numbers (Dxx) trace to the history in git. Work:
[tasks/](tasks/README.md).

## 1. What a plugin is
A feature built on the middleware (a *package* in core.md). A plugin is a folder that bundles any
of: tools, middleware, commands, installed skills, standing rules, knob-offset bounds,
permissions, a state namespace and dependencies, declared in one manifest (§3). It reaches the core only through
the middleware's six levers ([middleware.md §2](middleware.md)) (D9, D41), and sees only the
contract's facts ([contract.md](contract.md)); like the middleware it never parses message text to
understand it. Every tool names facts and a permission; a plugin's permissions live in its own
namespace and never redefine a middleware permission (D86, D87).
- Tools (search, files, email) are shipped by plugins (D35).
- Role plugins (assistant, customer service, sysadmin) may ship a birth certificate, used only at
  birth; they can never overwrite identity afterwards (D20, D35).
- Installed, run and uninstalled by the install module ([middleware.md §5](middleware.md)).
  Uninstalling removes the plugin's tools, middleware, rules and knob offsets; its events stay
  in the log as history.

## 2. Catalog
| Plugin | Kind | Status | What it brings |
|---|---|---|---|
| **Soul Tuning** | behavior | Task 4 | modes and example conversations (§5.1) |
| **Web Search** | tool | Task 4 | `web_search`, `web_fetch` over many providers (§5.2) |
| **Workspace** | tool | Task 4, on by default | `read`, `write`, `edit`, `grep`, `glob`, `run` in a sandbox (§5.3) |
| **PDF Reader** | extension of Workspace | Task 4 | a native reader for `.pdf` (§5.4) |
| Word / Spreadsheet readers | extensions of Workspace | planned | the same pattern as PDF Reader |
| simulated emotions | behavior | idea | own state via events, notes, rules, knob offsets |
| plugin suggestions | feature | **Draft** | a note to the owner from the local catalog |
| moderation, content filters | feature | idea | pass/replace/stop |

- Removed: memory-view (2026-11-12). Roles and permissions are not a plugin: they are the
  middleware's `roles.py` ([middleware.md §8](middleware.md)), because every plugin brings tools
  and commands that need limits.
- **Plugin suggestions** (Draft, D28): only to the owner, only from the local catalog, at most
  once per missing capability per habituation window, never installs anything.

## 3. Manifest
One file at the root of a plugin, `plugin.yaml`, declares everything it brings and every default
(D41). The install module reads it; the core never does. The owner never edits it: it is
replaced on update and removed on uninstall.

```yaml
manifest_version: 1                  # format of this file (D58)
name: workspace
version: 0.1.0                       # the plugin's release
description: Read, write and run code in a sandboxed folder per person.
dependencies: []                     # must load first; missing or failing fails boot
optional_dependencies: []            # used when present
state_namespace: workspace           # its state keys are workspace:* (projected from its events); its
                                     # event kinds are workspace.<kind> (a `-` in the name becomes `_`)
features:                            # each can be switched off in config.d/
  tools: [read, write, edit, grep, glob, run]
  commands: [workspace]
  middleware: []
tools:                               # facts per tool (middleware.md §4)
  read:  { facts: [reads] }
  write: { facts: [changes] }
  run:   { facts: [runs_code, network] }
settings:                            # schema with defaults; the owner overrides in config.d/
  base: { type: string, default: debian-slim }
  timeout_seconds: { type: int, default: 120 }
permissions:                         # roles.py; base = lowest tier that gets it by default
  workspace.read:  { label: "Read files",        base: guest }
  workspace.write: { label: "Write and edit",    base: member }
  workspace.run:   { label: "Run code",          base: trusted }
role_settings:                       # a value per tier; roles may override
  quota: { guest: 0, member: 1G, trusted: 5G }
extension_points:
  opens: [reader]                    # other plugins may fill it
  fills: []
skills: []
rules: []                            # standing rules shown in <self>, in this order
knob_offsets: {}                     # bounded, temporary; recorded as disposition.updated
```

## 4. Configuration
| File | Owner | Holds |
|---|---|---|
| `plugins/<name>/plugin.yaml` | the plugin | every default; never edited by the owner |
| `config.yaml` | the owner | which plugins are on (`plugins:`), `roles:`, `people:` ([project.md §2](project.md)) |
| `config.d/<plugin>.yaml` | the owner | one plugin: `features`, `options`, and `permissions` / `role_settings` (written by `/plugin enable`) |

```yaml
# config.d/workspace.yaml — written by /plugin enable from workspace 0.1.0
version: 1
features: { run: true }
options:  { timeout_seconds: 300 }
permissions:                   # lowest tier per permission; edit to move a permission
  workspace.read:  guest
  workspace.write: member
  workspace.run:   trusted
role_settings:
  quota: { guest: 0, member: 1G, trusted: 5G }
```

- **Merge rules** (as Helm values): maps merge, lists replace whole, `[]` switches everything
  in that list off. The boot report warns when a list replaces a default.
- An unknown key or a wrong value fails at boot with a clear message. A `config.d/` file for a
  plugin that is not installed is reported and ignored; it stays, so the settings return if the
  plugin does.
- A feature switched off is removed from the registry, not hidden.
- The files are the source of truth. The program writes them only through `/plugin
  enable|disable|update` and `/role add|remove`, narrowly and after a backup; plugin updates
  follow the dpkg rule ([middleware.md §5](middleware.md)). `/plugin show <name>` prints the
  defaults; `/roles show <role>` prints the merged permissions per role.
- Every file carries `version:` (D58); see [middleware.md §5](middleware.md).
- Which tier gets a permission is in `config.d/`; which roles exist and who holds them is in
  `config.yaml` (`roles:`, `people:`).

## 5. First plugins

### 5.1 Soul Tuning
Tunes expression, never identity; only the owner changes it (absolute line 1).
- **Modes** (daily, professional, …): standing rules in `<self>`, knob offsets within bounds,
  the `/mode` command.
- **Example conversations**: short exchanges in the stable prompt, in a fixed order.
- Who a person is to the soul ("a customer; help with orders") is not here: it is each role's
  and person's `context` in `roles.py` ([middleware.md §8](middleware.md)).

### 5.2 Web Search
- Tools `web_search` and `web_fetch`; permissions `web-search.search` and `web-search.fetch`,
  base `guest`.
- Providers in an ordered fallback chain set in `options`: SearXNG, Tavily, Brave, Exa,
  Serper, Google, and DDGS always last. Each normalizes its results to one shape and fails
  with a structured reason, never a crash. Port V1's `engine/tools/web_search.py` (ADR
  `engine/0026-search`).
- `web_fetch` runs in the MonoAgens process, so it refuses internal addresses (SSRF): resolve
  the host, check every address against the same blocked set as the sandbox (§5.3), connect to
  the checked address only. Port V1's `fetch.py`.

### 5.3 Workspace
On by default through the shipped `config.yaml`; off with one line.
- Tools: `read` (line ranges; also attachments received by adapters, through their
  reference), `write`, `edit` (exact text replacement), `grep`, `glob`, `run`. Output is capped
  with a "continue from here" marker ([middleware.md §4](middleware.md)).
- **Progress note for long work** (a standing rule, lever 3): work of more than a few steps
  keeps `progress.md` in the workspace it works in — the goal, what is done, the next step,
  the files involved — updated after each step and read first whenever the work resumes. The
  owner can open and edit it like any file.
- Opens the `reader` extension point: one reader per file type; `read` without a reader for a
  type says so and names the extension it needs, never returns raw bytes.
- Folders: each person's own workspace; **shared folders** named by the setting
  `workspace.shared` on the person's roles (a person in two group roles sees two).

**Sandbox for `run`** (D60). Python drives two native tools, bubblewrap (files) and pasta from
`passt` (network); no Node, no container runtime.
- **Base**: one shared, read-only Debian slim tree (Python, pip/uv, bash, coreutils, git, curl,
  CA certificates), prepared once when the plugin is first enabled and used by everyone.
- **Layer per person**: an overlay on the base where `pip install` and `apt install` land;
  quota from the role setting `workspace.quota` as a soft limit (checked after each run; the
  next run is refused while over); can be reset.
- **Visible inside**: the base and the person's layer, their workspace, shared folders they
  may use, an empty `/tmp` per run, a private `/proc` and `/dev`, copies of `resolv.conf` and
  the CA certificates. The host's system, home, `.env` and database are not visible at all.
- **Network**: outbound to the internet allowed. Blocked: the host's loopback (`--no-map-gw`
  plus `-T none -U none`), private ranges by blackhole routes inside the sandbox's network
  namespace (10/8, 172.16/12, 192.168/16, 100.64/10, 169.254/16, fc00::/7, fe80::/10) and the
  server's own public addresses. No inbound (`-t none -u none`).
- **Limits**: a computed environment without secrets (V1 ADR `engine/0024-tool-input-contract`),
  time, CPU, memory and process limits, capped output; the sandbox dies with MonoAgens.
- **Guard**: marks a call `sensitive` (sudo, mount, network scans, download-and-execute, …),
  which asks for approval even in `yolo`. It is a marker, not the wall: patterns can always be
  evaded; the wall is the sandbox and the limits. Port V1's two-stage guard.
- **Fail-closed**: no pasta → no network at all (`--unshare-net`); no bubblewrap or no base →
  `run` unavailable. The boot report says which.
- Shares the host kernel: right for a personal or community soul; a microVM runner can later be
  an extension.

### 5.4 PDF Reader
Fills Workspace's `reader` point for `.pdf`; native (pypdf), no cloud; `dependencies:
[workspace]`. Its library is installed only with it.

# Workspace plugin

A plugin of MonoAgens: it follows the rules every plugin follows ([spec.md](../../docs/spec.md)
*Plugins*). This file holds only what is specific to Workspace.

On by default through the shipped `config.yaml`; off with one line.
- Tools: `read` (line ranges; also attachments received by adapters, through their
  reference), `write`, `edit` (exact text replacement), `grep`, `glob`, `run`. Output is capped
  with a "continue from here" marker.
- **Progress note for long work** (a standing rule, lever 3): work of more than a few steps
  keeps `progress.md` in the workspace it works in — the goal, what is done, the next step,
  the files involved — updated after each step and read first whenever the work resumes. The
  owner can open and edit it like any file.
- Opens the `reader` extension point: one reader per file type; `read` without a reader for a
  type says so and names the extension it needs, never returns raw bytes.
- Folders: each person's own workspace; **shared folders** named by the setting
  `workspace.shared` on the person's roles (a person in two group roles sees two).

**Sandbox for `run`**. Python drives two native tools, bubblewrap (files) and pasta from
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


**Role settings.** `workspace.shared` names the shared folders a role may use;
`workspace.quota` is the soft limit of a person's layer. A role that only allows running code
looks like `runner: { allow: [workspace.run] }`.

**Manifest** (abridged):

```yaml
tools:
  read:  { facts: [reads] }
  write: { facts: [changes] }
  run:   { facts: [runs_code, network] }
settings:
  base: { type: string, default: debian-slim }
  timeout_seconds: { type: int, default: 120 }
permissions:
  workspace.read:  { label: "Read files",     base: guest }
  workspace.write: { label: "Write and edit", base: member }
  workspace.run:   { label: "Run code",       base: trusted }
role_settings:
  quota: { guest: 0, member: 1G, trusted: 5G }
extension_points:
  opens: [reader]
```

**Dependencies** (approved by the owner, installed only with this plugin): the system tools `bubblewrap` and `passt`.

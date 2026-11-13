# Operator — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the core
([constitution §1](constitution.md)). Decision numbers (Dxx) trace to the history in git. Work:
[tasks/](tasks/README.md).

## 1. What the operator layer is (D73)
The tools the owner uses to install, run and look after one soul: the `monoagens` command line,
the installer, and **MonoAgens Console**, a web app. They live outside the soul.

- **Not a fourth layer of the soul.** The operator tools are not core, middleware, adapter or
  plugin. The core does not know they exist; every change they make passes through the
  middleware like any owner command ([middleware.md §1](middleware.md)).
- **The command line first; Console is a face on it.** Every Console action calls the same code
  as its command. One implementation, two ways to use it.
- **Clean by default.** A fresh install holds the soul, the CLI adapter and the default plugins
  only. Anything else is installed when the owner asks; until then its code is not on the
  machine.
- **Console is a separate process.** It keeps working when the soul has crashed or its
  configuration is broken, so the owner can see why and fix it.
- **One owner.** Console serves the owner of this deployment only; there are no other accounts.

## 2. The command line (D74)
One global command, `monoagens`, installed in `~/.local/bin`. With the compose stack (§3) it
forwards the commands that need the runtime into the container, so every command works the same
on both stacks.

| Command | Does |
|---|---|
| `install`, `init` | install; the setup wizard (§3) |
| `update` | backup → pinned, verified release → `uv sync` → migration check → restart; back to the previous version when boot fails; never runs `main` |
| `plugin install\|enable\|disable\|remove [name]` | a plugin's life cycle (§4) |
| `adapter install\|enable\|disable\|remove [name]` | an adapter's life cycle (§4) |
| `start`, `stop`, `restart`, `status` | the service, through the commands the owner sets (§2.1) |
| `logs [-f]` | the process log (§2.2) |
| `chat` | talk to the soul through the CLI adapter |
| `backup`, `restore <archive>` | one archive: database dump, `config.yaml`, `config.d/`, `soul.yaml` |
| `doctor [--fix]` | health check: database and extensions, migrations, file permissions, provider reachable, adapters, sandbox |
| `config schema` | the schema of `config.yaml` and `config.d/` for Console's form mode (§7) |
| `console` | run Console |
| `uninstall [--purge]` | remove the program; data stays unless `--purge` is confirmed |

- **One function, two doors.** `/plugin enable` typed in chat and `monoagens plugin enable` in a
  terminal call the same loader function, which writes only its own line after a backup
  ([middleware.md §5](middleware.md)). When the soul runs, the command line asks it to
  `/reload`; when it does not, the change applies at the next start.
- **Look and feel.** Arrow-key choices, a spinner while the soul thinks, status lines for steps,
  colors per speaker, input history, completion of `/commands`, progress bars, a logo. Built on
  Rich and prompt_toolkit (the `cli` extra, installed by default). When the output is not a
  terminal, plain text; `NO_COLOR` is honored.

### 2.1 Service control
How the soul runs as a service belongs to the deployment (OpenRC, systemd, compose). The owner
sets the commands once in `config.yaml` → `operator:`; `start`, `stop`, `restart`, `status` and
Console's buttons call them, and a command left empty hides its button.

```yaml
# config.yaml
operator:
  service:
    start:   "rc-service monoagens start"
    stop:    "rc-service monoagens stop"
    restart: "rc-service monoagens restart"
    status:  "rc-service monoagens status"
    logs:    "tail -n 500 /var/log/monoagens.log"
  console:
    listen: 127.0.0.1:8470
```

### 2.2 The process log
The soul's own output (boot report, pairing code, adapter status, errors). It never holds
conversation content: only ids, numbers and states. A line that names an event carries its id,
so Console links it to the Timeline, where the privacy chip shows who may see it (§6.3).
`monoagens logs` and Console's Logs page show the same lines.

## 3. Installer (D74)
Modelled on V1 (branch `archived`, ADR-0007, ADR-0018 §4.3, ADR-0039), with its lesson: V1's
`install.sh` grew to over 2,000 lines because shell and Python held the same logic twice.

- **`install.sh` is a thin bootstrap.** It installs a pinned `uv` checked against its sha256,
  downloads a pinned release checked against its published checksum, and hands over to
  `monoagens init`. Everything else lives in Python.
- **Two stacks.** `--stack native`: the soul runs on the host, Postgres in a container.
  `--stack compose`: both in containers. The container engine is **Podman or Docker**; an
  external Postgres is given with `--database-url`.
- **The wizard** (`monoagens init`): location first (the folder and what is already there) →
  provider, model and key → database → adapters → plugins → review → write. The left key goes
  back with earlier answers kept; Escape cancels without writing anything. Keys are masked and
  written only to `.env` at mode `0600`, never shown in the review or the log. Every list of
  choices comes from the registries and manifests, never from a list in the installer.
- **What a fresh install holds**: the soul, the CLI adapter (built in: it is the owner's door,
  the pairing code and Console's Chat use it) and the default plugins
  ([plugins.md §2](plugins.md)). No other adapter.
- **The read-only database role** for Console (§6.1) is created here, once, with the admin
  credentials the owner gives for the duration of the install. Neither the soul nor Console keeps
  those credentials.
- **Tested, not hoped**: the container image carries the migrations, and the compose stack
  installs the extras of the enabled adapters (both broke in V1).

## 4. Plugins and adapters: life cycle and catalog (D75)
- **One life cycle for both**: available → installed → enabled ⇄ disabled → removed. Every step
  is an owner action and is recorded as an event.
- **Install** downloads the archive, checks its digest, unpacks it into `plugins/<name>/` or
  `adapters/<name>/` and installs its dependencies there. A digest that does not match writes no
  file and changes no configuration. No automatic updates, no background fetch.
- **Enable** writes the configuration lines ([middleware.md §5](middleware.md)); a new plugin's
  or adapter's code needs a restart, and the command says so.
- **Remove** deletes the code and its dependencies; the events stay in the log as history and
  the owner's files are not touched.
- **One repository.** Every plugin and adapter is developed in this repository. The split happens
  on the owner's machine: during development the installer copies only the default parts from
  the checkout; from the first release, each plugin and adapter is its own archive with a
  sha256, attached to the same GitHub release, and listed in a catalog index file (name,
  version, digest, description, dependencies).
- **Adapters move to their own folders** with a manifest, as plugins have, once Task 5 is done
  (its work happens in `monoagens/adapters/`). The CLI adapter stays inside the package.
- Plugins and adapters are trusted code that runs in the soul's process
  ([middleware.md §5](middleware.md), D32.11): a digest proves the archive is the published one,
  not that the code is safe.

## 5. Console: what it shows (D76)
A sidebar of pages, grouped:

| Group | Page | Holds |
|---|---|---|
| Main | **Chat** | the CLI adapter in the browser: talk to the soul, type `/commands` |
| | **Timeline** | each percept and what followed: the gate's decision with its numbers (`s`, `theta`, `p`, `u`, reason), thoughts, acts, model calls and tokens; filter by place, person, decision |
| | **Memory** | what the soul remembers, with forget |
| | **Promises** | active expectations: promises, waits, reminders (read only) |
| Bodies and capabilities | **Adapters** | each adapter: on or off, pairing (the WhatsApp QR), its surfaces from the boot report ([contract.md §4.1](contract.md)), its `render:` options ([adapters.md §4](adapters.md)) |
| | **Plugins** | installed and available plugins, on or off, version, "restart needed" |
| | **Skills**, **Tools** | what the registry holds, with facts and permissions |
| | **MCP** | locked until the MCP Connect plugin exists ([project.md §5](project.md)) |
| People and rules | **People** | persons, linked accounts, `/link`, the owner pairing code |
| | **Roles** | roles, who holds which, approvals waiting |
| Settings | **Model** | provider, model, fallbacks; tokens and the daily budget |
| | **Keys** | `.env`: write only, never shown |
| | **Config** | `config.yaml` and `config.d/` (§7) |
| | **Soul** | the living identity (name, language, persona, principles), editable (§7.1); `soul.yaml` shown as the birth certificate ([core.md §9.3](core.md)) |
| Footer | — | the soul running or not, adapters up, tokens today; Restart, Update; Logs |

- **Locked pages.** A page that belongs to a plugin or an adapter has its on/off switch at the
  top; when it is off, the rest of the page is grey with the way to turn it on. When the soul is
  not running, everything that needs a command is locked; reading, Config, Logs and Start stay
  open. Pages of the middleware (People, Roles, Config) are never locked by a plugin.
- **Not included**: sessions (the soul has none; it lives on), profiles (one deployment is one
  soul), webhooks, a hosted portal (BYOK, [project.md §1](project.md)).

## 6. Console: how it works (D76, D77)

### 6.1 Paths
| Need | Path |
|---|---|
| Timeline, status, tokens, memory | the database, through a **read-only role** (`SELECT` only) |
| Every change the soul applies | a `/command` line to the CLI adapter's socket; the reply is read from the Log (the `act.out` whose `caused_by` is the command) |
| Editing files | §7: validate in a short-lived process, back up, write, `/reload` |
| Start, stop, restart | the service commands (§2.1) |
| Logs | the process log (§2.2) |

- Console never writes the database. It sends only lines that start with `/`: any other line is
  a message to the soul and would be appraised as one (Chat sends those on purpose).
- Console runs as the same system user as the soul, on the same machine: the socket is `0600`
  and the files are the soul's.
- Console does not import the embedder or plugin code; the validation that needs them runs in a
  short-lived process.

### 6.2 Login and network
- **Network**: Console listens on `127.0.0.1` by default. The owner opens it to a private network
  (Tailscale) by setting the address; TLS comes from that network or a reverse proxy. Console is
  never meant for the open internet: it shows every conversation of every person.
- **First start**: Console prints a one-time **setup code** in the process log. The first page is
  registration, which needs that code; whoever registers is the Console owner, and the code is
  then spent. Only someone who can read the server's log can register.
- **Password**: hashed with scrypt, stored in Console's own state file outside the repository
  (not in `.env`, not in `config.yaml`); a session
  cookie (`HttpOnly`, `Secure` when served over TLS, `SameSite=Strict`) with a CSRF token on
  every change; repeated wrong passwords slow down.

### 6.3 Privacy
The owner may see everything (D46: the owner's view is a view, not ownership). So that he knows
what belongs to others, every line in Timeline and Memory carries a **privacy chip**: its
audience labels ([middleware.md §3](middleware.md)), a private place marked as such. Hard
forgotten content shows as forgotten, never reconstructed.

## 7. Config editor (D78)
- **Two modes**: a **form** generated from `monoagens config schema` (the loader's own knowledge
  of `config.yaml` and the plugin manifests' `settings`): a switch for a boolean, a menu for a
  choice, a bounded number, text, chips for a list; anything a form cannot hold says "edit in
  raw". And **raw**: the file in an editor with YAML highlighting.
- **Comments survive.** The form writes only the lines that changed, with the loader's narrow
  writers; raw saves the text exactly as typed.
- **Saving** is the same in both modes: show the diff → validate (short-lived process) → back up
  → write → `/reload` → show the result (ok, "restart needed", or refused with a button to
  restore the backup). An invalid file is never written; the error names the line. When the soul
  is not running, the file is still saved once valid, marked "applies at start".
- **Conflicts**: Console remembers the file's hash when it opens it; if the file changed on disk
  since (a command in chat, an edit over SSH), saving is refused and the change is shown.
- Switching modes asks to save or discard first.
- Editable: `config.yaml`, `config.d/`. Read only: `prior/`. `.env` only on Keys.
  People and Roles use the same editor on their sections.

### 7.1 The soul's identity
- **After birth** `soul.yaml` is never read again; identity lives in `state: identity`
  ([core.md §9.3](core.md)). Editing the file would change nothing, so Console never writes it
  after birth and shows it as the birth certificate.
- **The Soul page edits the living identity** with the same two modes: a form (name, language,
  persona, and principles as a list to add, remove and reorder) and raw (the current identity
  as YAML). Saving shows the diff, then sends the owner command `/identity set …`.
- `/identity` calls the core's own `set_identity` capability through the action hook, the same
  door the soul uses when the owner asks in chat: the owner check of absolute line 1 holds, the
  change is recorded as `identity.updated`, and the soul sees the new identity from its next
  turn.
- **Before birth** (a fresh install), `soul.yaml` is edited directly, in the wizard or in
  Console.

## 8. Technology and weight (D79)
- **Server**: Python, Starlette, uvicorn, Jinja2 (autoescaping on), as the `console` extra.
- **Browser**: HTML rendered on the server, one CSS file, small plain JavaScript, htmx for partial
  updates, Server-Sent Events for live lines. Libraries only on the page that needs them: uPlot
  (charts), CodeMirror (Config), SortableJS (ordering). Every file is served by Console itself:
  no npm, no build step, no CDN.
- **Not used**: Node, React, FastAPI, Django, Flask.
- **Light**: one process (about 40–70 MB), no embedder, no plugin code, one or two database
  connections, every list paged by id, never a whole table.
- **Look**: dark, system fonts, works on a phone (the sidebar folds). Modern through CSS: custom
  properties (the design tokens), grid, container queries, View Transitions, `<dialog>`,
  `popover`, SVG icons, `prefers-reduced-motion` honored.
- **Design hand-off**: the owner designs in FigCraft; what is taken from it is the tokens, the
  components with their states and the screens, never generated framework code.

## 9. Changes this needs elsewhere
Written into the operator task, applied when that task runs:
- `middleware.md` §5: one enable function for chat and the command line; `config schema`; the
  loader accepts the `operator:` section of `config.yaml` and leaves it to the operator tools.
- `project.md` §2: the `operator:` section in the `config.yaml` example.
- `middleware.md`: an owner command `/forget` (the core has `forget_hard`; no command reaches it).
- `middleware.md`: an owner command `/identity show|set` that calls the core's `set_identity`
  through the action hook (§7.1).
- `adapters.md` §2: adapters in their own folders with a manifest; the CLI adapter built in.
- `glossary.md`: operator, Console, catalog; the layout row for `monoagens/console/`.
- `AGENTS.md`: the `console` and `cli` extras approved by the owner (2026-11-13).

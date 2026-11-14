# MonoAgens Console — the operator's web panel

The design of MonoAgens Console, the owner's web panel. It belongs to the operator layer
([spec.md](spec.md) *Operator*): every Console action calls the same code as its command line,
and the core does not know Console exists.

- - **Console is a separate process.** It keeps working when the soul has crashed or its
  configuration is broken, so the owner can see why and fix it.
- **One owner.** Console serves the owner of this deployment only; there are no other accounts.

## What it shows
A sidebar of pages, grouped:

| Group | Page | Holds |
|---|---|---|
| Main | **Chat** | the CLI adapter in the browser: talk to the soul, type `/commands` |
| | **Timeline** | each percept and what followed: the gate's decision with its numbers (`s`, `theta`, `p`, `u`, reason), thoughts, acts, model calls and tokens; filter by place, person, decision |
| | **Memory** | what the soul remembers, with forget |
| | **Promises** | active expectations: promises, waits, reminders (read only) |
| Bodies and capabilities | **Adapters** | each adapter: on or off, pairing (the WhatsApp QR), its surfaces from the boot report, its `render:` options |
| | **Plugins** | installed and available plugins, on or off, version, "restart needed" |
| | **Skills**, **Tools** | what the registry holds, with facts and permissions |
| | **MCP** | locked until the MCP Connect plugin exists |
| People and rules | **People** | persons, linked accounts, `/link`, the owner pairing code |
| | **Roles** | roles, who holds which, approvals waiting |
| Settings | **Model** | provider, model, fallbacks; tokens and the daily budget |
| | **Keys** | `.env`: write only, never shown |
| | **Config** | `config.yaml` and `config.d/` |
| | **Soul** | the identity (name, language, persona, principles), editable; edits `soul.yaml`, the one source |
| Footer | — | the soul running or not, adapters up, tokens today; Restart, Update; Logs |

- **Locked pages.** A page that belongs to a plugin or an adapter has its on/off switch at the
  top; when it is off, the rest of the page is grey with the way to turn it on. When the soul is
  not running, everything that needs a command is locked; reading, Config, Logs and Start stay
  open. Pages of the middleware (People, Roles, Config) are never locked by a plugin.
- **Not included**: sessions (the soul has none; it lives on), profiles (one deployment is one
  soul), webhooks, a hosted portal (BYOK, [spec.md](spec.md) *Rulings*).

## How it works

### Paths
| Need | Path |
|---|---|
| Timeline, status, tokens, memory | the database, through a **read-only role** (`SELECT` only) |
| Every change the soul applies | a `/command` line to the CLI adapter's socket; the reply is read from the Log (the `act.out` whose `caused_by` is the command) |
| Editing files | *Config editor*: validate in a short-lived process, back up, write, `/reload` |
| Start, stop, restart | the service commands |
| Logs | the process log |

- Console never writes the database. It sends only lines that start with `/`: any other line is
  a message to the soul and would be appraised as one (Chat sends those on purpose).
- Console runs as the same system user as the soul, on the same machine: the socket is `0600`
  and the files are the soul's.
- Console does not import the embedder or plugin code; the validation that needs them runs in a
  short-lived process.

### Login and network
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

### Privacy
The owner may see everything (the owner's view is a view, not ownership). So that he knows
what belongs to others, every line in Timeline and Memory carries a **privacy chip**: its
audience labels, a private place marked as such. Hard
forgotten content shows as forgotten, never reconstructed.

## Config editor
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

### The soul's identity
- `soul.yaml` is the one source of identity ([spec.md](spec.md) *Identity*). The Soul page edits it with the same
  two modes: a form (name, language, persona, and principles as a list to add, remove and
  reorder) and raw (YAML). Saving shows the diff, then applies it through the same door the soul
  uses when the owner asks in chat: the owner check of absolute line 1 holds while the valve is
  on, the change is recorded as an identity change, and the soul sees the new identity from its
  next turn.
- An edit over SSH is picked up on reload the same way; the file-hash conflict rule above keeps
  two editors from overwriting each other.

## Technology and weight
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

## Changes this needs elsewhere
Written into the panel task, applied when that task runs (sections of [spec.md](spec.md)):
- *Install*: one enable function for chat and the command line; `config schema`; the
  loader accepts the `operator:` section of `config.yaml` and leaves it to the operator tools.
- *Deployment files*: the `operator:` section in the `config.yaml` example.
- *Middleware*: an owner command `/forget` (the core has `forget_hard`; no command reaches it).
- *Middleware*: an owner command `/identity show|set` that calls the core's `set_identity`
  through the action hook.
- *Rules for every adapter*: adapters in their own folders with a manifest; the CLI adapter built in.
- *Terms*: operator, Console, catalog; the layout row for `monoagens/console/`.
- `AGENTS.md`: the `console` and `cli` extras approved by the owner (2026-11-13).

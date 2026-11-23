# TUI gap audit — 2026-11-23

Method: (1) the TUI run in tmux (release build of `a91aea0`), (2) code reading, (3) every `monoagens` CLI
subcommand (anything the CLI can do, the TUI must be able to do), (4) references: omp v19.1.0 run in tmux
plus its docs (`tui-runtime-internals.md`, `keybindings.md`, `providers.md`, `approval-mode.md`), and
the commonly known behaviour of Claude Code, opencode and Crush.
MonoAgens is a companion soul with adapters, not a coding agent: coding-specific features (LSP, diffs,
file tree, plan mode) are out of scope on purpose.

Legend — **Have**: works · **Broken**: exists but fails · **Missing** · **Decision**: needs the owner.

## A. Foundation
| # | Area | State (verified) | Target | Ref |
|---|---|---|---|---|
| A1 | One gateway per data dir | **Broken**: no lock anywhere; TUI and `serve` (or two TUIs) each boot a gateway on the same store → two queue consumers, against the ARCHITECTURE invariant "one queue, one consumer"; plugins spawned twice | Exclusive data-dir lock; second process either refuses with a clear message or **attaches** to the owner | **Decision D-1** |
| A2 | TUI ↔ daemon | **Missing**: TUI always hosts its own gateway; no control channel for `serve` | TUI can attach to a running `monoagens serve` as body-tui over the unix socket + an admin channel for settings/plugins/logs | **Decision D-1** |
| A3 | Startup with bad config | **Broken** (verified: `provider = "nonsense"` → prints the error and exits 1, TUI never opens) | TUI opens anyway, shows the error, opens the offending setting; `/doctor` checks config, secrets, provider reachability, plugin binaries, embedder | Claude Code `/doctor` |
| A4 | Width-safe rendering | **Broken**: emoji with VS16 shift columns | Every row width-fit by display width (grapheme + unicode-width), sanitised tabs/control chars | omp rendering rules |
| A5 | Resize / focus / suspend | **Broken**: input drained on full redraw; nothing on Resize/FocusGained; no Ctrl+Z | Redraw on resize/focus; Ctrl+Z suspend/resume restores raw mode; `Alt+L` display reset | omp `app.display.reset` |
| A6 | Small terminals | Unverified | Defined layout < 80 cols (sidebar hidden, single-line status) and < 20 rows | — |

## B. Conversation
| # | Area | State | Target | Ref |
|---|---|---|---|---|
| B1 | Turn end | **Broken**: busy ends only on `chat.speak` | Ends on reply / error / finish+grace, "chose not to reply" notice | — |
| B2 | Interrupt | **Missing**: no way to stop a running turn; gateway has no turn cancel | Esc (twice) interrupts: stop waiting, mark turn cancelled; gateway support needed | Claude Code Esc · **Decision D-2** |
| B3 | Queue while busy | **Missing** | Enter queues, shows queued bubble; `Alt+Up` pulls it back to edit | omp follow-up/dequeue |
| B4 | Retry | **Missing** | `F5`/`Alt+R` retries last failed turn | omp `app.retry` |
| B5 | Composer | Have: multiline, history | + `Ctrl+R` history search, `Ctrl+G` external editor, recall cleared draft (`Ctrl+C` then Up) | omp |
| B6 | Streaming | **Missing**: replies arrive whole via `chat.speak` | Token streaming needs a protocol/capability change | **Decision D-3** (protocol invariant) |
| B7 | Reasoning visibility | Partial: "Reasoning…" phase text | Collapsible per-turn "what happened" block: model calls, capability steps, valve verdicts (`Ctrl+O` expand) | omp tool collapse |
| B8 | Message actions | Partial: `/copy` last reply | Select a message (mouse/keys): copy, quote into composer, `/why` (valve explanation = `monoagens log --explain`) | — |
| B9 | Chats | Partial: new/rename/delete; titles broken | Titles fixed; chat search; pin; delete asks to confirm; `/export` chat to Markdown | omp session picker |
| B10 | Notify when done | **Missing** (the "minimize" complaint) | When the terminal is unfocused and a reply arrives: bell + OSC 9/777 desktop notification + title badge; setting | Claude Code notifications |
| B11 | Attachments | Have | Keep; add image preview where the terminal supports it (kitty/sixel) — optional | omp images |

## C. Governance (Valve, RBAC, identity)
| # | Area | State | Target | Ref |
|---|---|---|---|---|
| C1 | Approvals | Partial: inline card only for TUI-originated prompts | `/approvals` panel = `monoagens approvals list/grant/deny`, includes actions held from WhatsApp/Telegram turns; badge count in status bar | omp approval mode |
| C2 | Valve toggles | Have (settings) | Keep; show what each toggle does + live effect preview | — |
| C3 | RBAC | **Broken**: read-only | Full editor (see design D7) | — |
| C4 | People & links | **Missing**: `[[person]]` and κ links only via CLI `link` | `/people`: persons, their platform accounts, roles; initiate/verify/unlink links | CLI `link` |
| C5 | Policy inspector | **Missing** in TUI | Read-only view of active Cedar rules and toggles = `monoagens inspector` | CLI `inspector` |

## D. Plugins / adapters
| # | Area | State | Target |
|---|---|---|---|
| D1 | Autostart | **Broken** | design D1 |
| D2 | Crash visibility / restart | **Missing** | design D1 + log |
| D3 | Pairing | Partial: suspends TUI and runs a subprocess | In-TUI pairing page: QR code / pair code rendered inside the TUI, live status until paired |
| D4 | Install / remove | Partial: `i` on catalog row | Guided install, remove with confirm, binary path check, "build hint" when binary missing |
| D5 | Adapter activity | **Missing** | Read-only **Activity** feed of adapter traffic (incoming message → turn → reply/hold), filter by plugin; no chatting through it | **Decision D-4** |
| D6 | Required secrets | **Missing** | Warn + set from detail page |

## E. Providers / models
| # | Area | State | Target | Ref |
|---|---|---|---|---|
| E1 | Catalog | **Missing**: one `openai_compatible` | design D6 (27 presets, 2 dialects) | omp providers |
| E2 | Discovery & picker | **Missing** | `/model` two-pane picker, cache, refresh | omp `/model` |
| E3 | Login | **Missing** | `/login` provider list + key entry + test | omp `/login` |
| E4 | Model per purpose | **Missing**: one model for reason/classify/consolidate | Optional model per purpose (titler/classify can use a small fast model) | omp model roles · **Decision D-5** |
| E5 | Cost & usage | **Broken**: status shows `$0.00` always (no prices anywhere) | Prices from discovery when the provider returns them (OpenRouter does), else user-entered per model, else hide the cost; `/usage` shows tokens per purpose and budget left | — |
| E6 | Provider errors | Unverified | Rate limit / auth / timeout shown as a clear notice with the provider's message and a link to `/log` | — |

## F. Observability
| # | Area | State | Target | Ref |
|---|---|---|---|---|
| F1 | Runtime log | **Missing** | `/log` (design D3) | — |
| F2 | Event log | **Missing** in TUI | `/events`: the soul's event log with the CLI `log` filters (place, source, kind, time) and explain-on-Enter | CLI `log` |
| F3 | Traces | **Missing** in TUI | Trace view by `traceparent` | CLI `inspector` |
| F4 | Replay / export-learning | **Missing** in TUI | Lab-only actions with progress, no model calls | CLI `replay`, `export-learning` |
| F5 | Status | Partial | `/status`: gateway owner, data dir, provider+model, embedder, plugins, queue depth, budget | Claude Code `/status` |

## G. Settings & interaction infrastructure
| # | Area | State | Target | Ref |
|---|---|---|---|---|
| G1 | Every key editable | **Missing** | Coverage test (design D7) | — |
| G2 | Selectors | **Missing** | omp-style selector with description, preview, search, scrollbar | omp settings |
| G3 | Scrolling | **Broken** | Shared scroll + scrollbar everywhere | omp |
| G4 | Help | Partial: one hint line | `?`/`F1` help overlay listing every key for the current screen; slash descriptions show live state | omp |
| G5 | Keybindings | **Missing** | `keybindings.toml` with action ids, editable from settings | omp `keybindings.yml` |
| G6 | Command palette | **Missing** | `Ctrl+K`: fuzzy over commands, settings, chats | opencode/Claude Code |
| G7 | First-run wizard | Partial | Uses the provider list + model picker + plugin selection; never asks for a raw endpoint unless "custom" | omp `/login` |

## H. Verification harness
| # | Area | State | Target | Ref |
|---|---|---|---|---|
| H1 | PTY tests | **Missing** | `scripts/tui-smoke.sh` (tmux) now; later a Rust PTY test driver (`portable-pty` + `vt100` screen parser) so screen assertions run in `cargo test` | omp `.omp/tools/tui.ts` + real-PTY regressions |
| H2 | Buffer snapshots | Partial: TestBackend asserts | Snapshot tests (`insta`) of key screens at 80×24, 120×40, 200×50 | — |
| H3 | Fakes | Partial: scripted provider | Fake stdio plugin (configurable crash/stderr/latency), fake provider HTTP server for discovery tests | — |

## Decisions needed from the owner
- **D-1 Gateway ownership.** (a) Exclusive lock only: a second MonoAgens refuses to start on the same data dir. (b) Lock + attach: the TUI attaches to a running `serve` as a client (needs an admin channel; the TUI already speaks Waist-2 as body-tui). (b) is what makes "bot runs in the background, TUI opens and closes" work.
- **D-2 Turn interrupt.** The gateway has no turn cancel. Interrupt only in the TUI (stop waiting; the soul's turn still completes and is recorded), or add cancel to the gateway?
- **D-3 Streaming replies.** Needs a change to how `chat.speak` reaches a body (a protocol-level change, covered by the "protocol is small, generic and stable" invariant). Yes, later, or no?
- **D-4 Adapter activity in the TUI.** The current design keeps WhatsApp/Telegram conversations out of `CHATS`. Is a read-only Activity feed wanted?
- **D-5 Model per purpose.** Allow a different model for classify/titling/consolidate?

## Proposed split (each one an OpenSpec change, done in order)
1. `tui-foundation` — A1–A6 (after D-1), B1, B3, D1, D2, F1, H1–H3. Everything after this is tested on this harness.
2. `model-provider-catalog` — E1–E6 (+ D-5).
3. `tui-settings-complete` — G1–G7, C2, C3, D3, D4, D6.
4. `tui-conversation` — B2 (after D-2), B4–B11.
5. `tui-governance-observability` — C1, C4, C5, D5 (after D-4), F2–F5.

## Owner decisions (2026-11-23)
- **D-1 → Lock + attach.** Exclusive lock on the data dir (`<data_dir>/gateway.lock`, held for the process
  life). The lock holder serves an **admin socket** `<data_dir>/gateway.sock` (owner-only perms 0600,
  NDJSON JSON-RPC like Waist-2). The TUI always talks to the gateway through one `GatewayClient` trait with
  two impls: in-process (TUI won the lock and hosts the gateway, and also serves the socket so another TUI
  can attach) and socket (attached to `monoagens serve` or another TUI). Closing an attached TUI leaves the
  daemon and its plugins running. Admin methods (initial set): `status`, `config.get`, `config.apply(patch)`
  (owner writes the file with toml_edit + .bak, then hot-reloads what changed: provider rebuild, plugin
  restart, valve reload), `plugins.list/start/stop/restart`, `logs.subscribe`, `events.query`,
  `events.explain`, `approvals.list/resolve`, `models.discover`, `turn.submit`, `turn.cancel`,
  `activity.subscribe`, `preview.subscribe`. The attached TUI's chat body (`body-tui`) is served through the
  same socket so its `chat.speak` actions reach it.
- **D-2 → Cancel in the gateway.** `turn.cancel` aborts waiting on the in-flight model call (drop the
  `spawn_blocking` join; the HTTP call is abandoned) and submits the existing core input
  `Input::ModelFailure { request_id, reason: "cancelled_by_owner" }` (`crates/core/src/lib.rs:136`), so the
  cancel is recorded as an event and `monoagens replay` stays bit-identical. No core change.
- **D-3 → Streaming now, safe form.** ARCHITECTURE: "the valve runs first and last in every door"; a
  `chat.speak` exists only after the model's full structured output is parsed and passes the action door.
  Therefore streaming is an **owner-only draft preview**: the provider uses SSE (`stream: true`) when the
  dialect supports it, the gateway incrementally extracts the draft speech text from the partial structured
  output and publishes it on `preview.subscribe` to the owner's TUI only, shown as a dim "draft" bubble. Adapters
  (WhatsApp, Telegram), robots and every other body still receive only the final, valve-approved
  `chat.speak`, byte-for-byte as today. When the turn ends the draft is replaced by the real reply, or by
  the valve verdict (held/denied) or by "chose not to reply". Previews are never recorded as events and never
  influence decisions. No protocol change. Setting: `Appearance → Live draft preview` (default on).
- **D-4 → Read-only Activity feed.** `/activity`: per adapter message in → turn → reply / held / denied,
  filter by plugin and place, Enter opens the explain view; no sending.
- **D-5 → (not asked; default)** Optional model per purpose (`[model.purposes.classify] provider/model`), off
  by default; titler uses classify's model when set.

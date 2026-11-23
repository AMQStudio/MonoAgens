# Design — `tui-governance-observability`

Audit: [../tui-foundation/audit.md](../tui-foundation/audit.md).

## Context (facts checked on 2026-11-23)
- CLI subcommands to match: `approvals list/grant/deny`, `link`, `inspector`, `log` (filters and `--explain`),
  `replay`, `export-learning`. Their implementations live in `crates/cli`; reuse their library functions
  rather than re-implementing. Exact function names: verify during implementation.
- `events.query` (by trace) and `events.explain` (by event id) are added by `tui-conversation`; the admin socket,
  `status`, `GatewayClient` come from `tui-foundation`.
- Git history note: approval commands were removed from the slash command registry (`3b63dce`); approvals are
  reached today only through inline cards and the CLI.

## C1. Approvals panel
- Admin `approvals.list` returns pending approvals (id, action capability, place, requesting actor label, age,
  originating plugin or "tui"); `approvals.resolve {id, decision: grant|deny}` goes through the same code path as
  the CLI. `/approvals` shows the list with Enter = detail, `g` grant, `d` deny (each asks to confirm).
  Adapter-originated holds appear exactly like TUI ones. The status bar shows `⚑ N` (symbol per the Icons
  setting) when N > 0; a subscription (or polling every 2 s; decide during implementation) keeps it live.
  The existing inline card for TUI-originated prompts stays and shares the same resolve path.

## C4. People and links
- `/people`: list of persons (`[[person]]`), each with its platform accounts and roles (roles come from
  `[rbac]`, read-only here; editing is in the RBAC editor). Actions mirror CLI `link`: initiate a link (shows
  the code/instructions the CLI prints), verify, unlink (with confirm). Proposed admin methods `people.list`,
  `links.initiate`, `links.verify`, `links.unlink` - confirm in review. Owner-only: the gateway rejects them
  from any non-owner connection (the admin socket is owner-only by mode 0600).
- The kappa link semantics and exact CLI flow: verify during implementation by reading `crates/cli` `link`.

## C5. Policy inspector
- `/policy`: read-only. Sections: valve toggles with on/off state (descriptions reuse the table from
  `tui-settings-complete`), the two absolute lines, active Cedar rules (rule text, grouped), RBAC summary.
  Data from proposed admin method `policy.get` backed by whatever `monoagens inspector` prints - confirm in
  review. Enter on a rule shows its source text; there is no edit.

## D-4. Activity feed (adapter activity, read-only)
- `/activity` (also a sidebar section under `PLUGINS` is optional, decide during implementation): rows
  `time · plugin · place · actor label · summary · outcome` where outcome is `replied`, `held`, `denied`,
  `silent` or `failed`; a row is the chain message-in -> turn -> reply/held/denied linked by trace id.
- Admin `activity.subscribe` pushes new rows; the initial page is built from `events.query` over recent events
  so it survives restarts without in-process memory. Filters: plugin, place, outcome (keys; type-to-search).
  Enter opens the explain view (`events.explain`). No composer, no send, no actions.
- Message text shown is truncated to one line; the full text appears in explain only when the CLI `log` would
  show it to the owner. Privacy filtering of private content: verify during implementation.

## F2-F4. Event log, traces, lab actions
- `/events`: table of events with the CLI `log` filters: place, source, kind, time range (relative like `1h`
  or absolute); `events.query` gains these filters; Enter = `events.explain`. Paged by cursor.
- Traces view: enter a `traceparent` (or press `t` on an event) to list every event of that trace in order,
  as `monoagens inspector` does.
- Replay and export-learning: shown only when the Lab is enabled (`/lab` mode); each runs as a background
  job with a progress bar and cancel, with no model calls (replay uses recorded decisions). Proposed admin
  methods `lab.replay` and `lab.export_learning` with progress notifications - confirm in review. Replay must
  not touch the live event log: how the CLI isolates it (copy or read-only open): verify during implementation.

## F5. `/status`
- Shows: gateway owner (pid, mode: hosting TUI or `serve`, attached or not), data dir, provider and model
  (and per-purpose models if set), embedder, plugins with state, queue depth, budget used/left, lock path.
  Data from admin `status` (extended with additive fields).

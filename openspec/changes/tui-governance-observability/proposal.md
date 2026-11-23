# Proposal — `tui-governance-observability`

Part 5 of 5. Audit: [audit.md](../tui-foundation/audit.md) (covers C1, C4, C5, adapter activity D5 with
decision D-4, and F2-F5).

**Depends on `tui-foundation`** (admin socket, `GatewayClient`, `status`, `/log`, PTY driver),
**`tui-settings-complete`** (scrollable lists, selectors, keybinding action table, command palette, RBAC editor
whose actor picker reuses `/people` data) and **`tui-conversation`** (`events.query` by trace and
`events.explain`, extended here).

## Why
Verified on 2026-11-23 unless stated (every row below is something the `monoagens` CLI can do and the TUI cannot):
- Approvals: only an inline card for TUI-originated prompts; actions held from WhatsApp/Telegram turns are
  reachable only by `monoagens approvals list/grant/deny` (C1).
- People and links: `[[person]]` entries and kappa links exist only through the CLI `link` command (C4).
- Policy: no TUI view of the active Cedar rules and valve toggles, available as `monoagens inspector` (C5).
- Adapter traffic: the current design keeps WhatsApp/Telegram conversations out of `CHATS`, so the owner cannot
  see what the soul does through adapters (D5/D-4).
- Event log, traces, replay and export-learning exist only as CLI `log`, `inspector`, `replay`,
  `export-learning` (F2-F4); `/status` is partial (F5).

## What Changes
1. **`/approvals` panel (C1)** = `approvals list/grant/deny`, including actions held from adapter turns;
   pending count badge in the status bar.
2. **`/people` (C4)**: persons, their platform accounts and roles; initiate, verify and unlink links.
3. **Policy inspector (C5)**: read-only view of active Cedar rules and valve toggles.
4. **`/activity` feed (D-4, read-only)**: per adapter message in -> turn -> reply / held / denied; filter by
   plugin and place; Enter opens the explain view; no sending.
5. **`/events` (F2)** with the CLI `log` filters (place, source, kind, time) and explain on Enter; **traces
   view (F3)** by `traceparent`; **Lab-only replay and export-learning (F4)** with progress and no model calls;
   **`/status` (F5)**.

## Impact
- Crates: `crates/tui` (views), `crates/gateway` (admin methods `approvals.list/resolve`,
  `activity.subscribe`, `events.query` filters, proposed `people.*`, `links.*`, `policy.get`, `lab.*`),
  `crates/cli` (reuse of existing command code paths where possible).
- No core change; no protocol change. Admin methods are gateway-local and call the same functions the CLI calls.
- Invariants kept:
  - *valve first and last in every door*: approvals are resolved through the same valve path as
    `monoagens approvals`; nothing here approves outside it; the policy view is read-only and the valve stays
    deterministic;
  - *one queue, one consumer*: resolving an approval, linking, and replay use the existing inputs and paths;
    replay runs on a copy and never feeds the live queue (verify during implementation how the CLI isolates it);
  - *protocol additive only by the owner*: none made;
  - *gateway generic and never learns*: activity entries are derived from recorded events and are not stored in
    process memory across restarts (the feed is rebuilt from the event log; live updates are a subscription);
  - *core sees only opaque ids*: the TUI renders names and platform labels the gateway already maps; the core
    is not changed;
  - *two absolute lines*: only the owner can change who the soul is (people/link edits require the owner role)
    and private content is shown to the owner only through the same audience filtering as `monoagens log`
    (verify during implementation).
- Activity feed adds no sending path: it cannot submit input or actions.

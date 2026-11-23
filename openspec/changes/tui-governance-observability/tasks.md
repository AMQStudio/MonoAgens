# Tasks — `tui-governance-observability`

## Worker rules
- One cargo command at a time, always with `CARGO_BUILD_JOBS=6 CARGO_PROFILE_TEST_DEBUG=0`.
- Never build or create worktrees in `/tmp`.
- No live paid model calls: use the `scripted` provider and fakes.
- Before editing a symbol run `node .gitnexus/run.cjs impact "<symbol>" --direction upstream --repo .` and report callers, processes and risk.
- Finish each group with `cargo test` for the touched crates and `cargo clippy --workspace --all-targets -- -D warnings`.
- Never read `lab/scenarios/sealed/`.
- Read `design.md` first; the shared audit is [audit.md](../tui-foundation/audit.md).

## 1. Approvals (`crates/gateway`, `crates/tui`)
- [x] 1.1 Admin `approvals.list` and `approvals.resolve` calling the same functions as the CLI. Acceptance: test grants and denies a held action; same outcome as the CLI path.
- [x] 1.2 `/approvals` panel with confirm, adapter-origin rows; status-bar badge; inline card uses the same resolve path. Acceptance: the three `tui-approvals-panel` scenarios.

## 2. People and policy (`crates/gateway`, `crates/tui`)
- [x] 2.1 Admin `people.list`, `links.initiate/verify/unlink` (shapes proposed, confirm in review), owner-only. Acceptance: non-owner rejection test; result equals `monoagens link`.
- [x] 2.2 `/people` view and link actions with unlink confirmation. Acceptance: scenarios in `tui-people-and-links`.
- [x] 2.3 Admin `policy.get` from the inspector data and the `/policy` read-only view. Acceptance: same rule ids as the CLI; no edit keys.

## 3. Activity feed (`crates/gateway`, `crates/tui`)
- [x] 3.1 Rows built from events by trace; `events.query` gains place/source/kind/time filters and a cursor; admin `activity.subscribe`. Acceptance: test with a fake adapter message -> reply, and a held action.
- [x] 3.2 `/activity` view with filters, explain on Enter, no composer; adapter chats not in `CHATS`. Acceptance: scenarios in `tui-activity-feed` including restart rebuild.

## 4. Observability (`crates/tui`, `crates/gateway`)
- [x] 4.1 `/events` view with filters and explain. Acceptance: filter scenario; explain equals CLI output.
- [x] 4.2 Trace view (`t` on an event, or typed id). Acceptance: trace scenario.
- [x] 4.3 Lab-only replay and export-learning jobs with progress/cancel (first check how the CLI isolates replay from the live log); proposed `lab.*` methods. Acceptance: replay scenario; no model call (scripted provider asserts zero calls); hidden outside Lab.
- [x] 4.4 `/status` view and additive `status` fields. Acceptance: attached-status scenario.

## 5. Finish
- [x] 5.1 Extend `scripts/tui-smoke.sh`: `/approvals` with a held fake action, `/activity` row from a fake adapter, `/status` against an attached daemon. Acceptance: script exits 0.
- [x] 5.2 Snapshots at 80x24, 120x40, 200x50 for `/approvals`, `/activity`, `/events`, `/status`. Acceptance: snapshot tests pass.
- [x] 5.3 `openspec validate tui-governance-observability --strict`; GitNexus `detect_changes --scope all` reviewed; `cargo test` for touched crates; clippy as in the rules above.

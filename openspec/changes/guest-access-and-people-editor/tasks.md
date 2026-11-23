# Tasks — `guest-access-and-people-editor`

## Worker rules
- `source "$HOME/.cargo/env"`; `CARGO_BUILD_JOBS=4 CARGO_PROFILE_TEST_DEBUG=0`; one cargo command at a time; never /tmp; never `tmux kill-server`.
- Never touch the owner's `~/.config/monoagens` or `~/.local/share/monoagens`; an owner `monoagens` may be running — do not stop or signal it. No paid model calls, no real WhatsApp/Telegram/provider network calls.
- GitNexus impact before editing a symbol (`node .gitnexus/run.cjs impact "<symbol>" --direction upstream --repo .`); UNKNOWN → confirm by text search. Valve decisions stay deterministic and from facts; do not edit ARCHITECTURE.md.

## 1. Guest access
- [x] 1.1 Valve: `[valve] guest_access` (default true) admits unregistered adapter actors with the fallback role; keep the refusal when off. Find how `known_actors`/attested roles feed `pass1_inbound` (`crates/valve/src/{valve.rs,roles.rs}`) and the Cedar inbound policy. Acceptance: spec scenario "New WhatsApp sender" incl. restart.
- [x] 1.2 Guest facts recorded via events (no process memory) and replayed into the valve snapshot on boot for both `serve` and the TUI host. Acceptance: restart test.
- [x] 1.3 `agent.converse` permission checked before waking the agent; refusal recorded as `agent_not_permitted`; no model call. Acceptance: both RBAC scenarios.
- [x] 1.4 Adapter slash commands (`/help`, `/whoami`, `/link`) answered by the gateway with zero model calls for admitted senders, filtered by permission (find whether adapter text commands are handled today; wire it if not). Acceptance: guest `/whoami` scenario with a fake adapter body.
## 2. People editor
- [x] 2.1 `/people`: create/delete person, single owner, add/remove accounts (recent senders from `events.query` or typed), roles; writes via `config.apply`; live effect. Remove the `[[person]]` owner/accounts exclusion from the coverage test. Acceptance: both people scenarios.
## 3. TUI runtime fixes
- [x] 3.1 Queue turns until body-tui capabilities are registered. Acceptance: scenario test.
- [x] 3.2 Exactly one terminal receipt per body-tui action (find the second submit path). Acceptance: two-turn test with a log check.
- [x] 3.3 Titler sends `[model.extra_headers]` with `{conversation_id}` filled. Acceptance: fake-server header test.
- [x] 3.4 One-time migration notice. Acceptance: two-start test.
## 4. Finish
- [x] 4.1 Tests for touched crates, clippy `-D warnings`, `scripts/tui-smoke.sh` PASS (add: guest `/whoami` via fake adapter, `/people` add account), `openspec validate guest-access-and-people-editor --strict`.

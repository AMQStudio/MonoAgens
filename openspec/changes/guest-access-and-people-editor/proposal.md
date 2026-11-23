# Proposal — `guest-access-and-people-editor`

## Why
Found 2026-11-24 on the owner's live setup (release build of 0fa3369):
- After the LID fix, WhatsApp DMs now arrive as `wa:user:6285111242960@s.whatsapp.net` (events #812, #872, #876, #901) and are still refused: `valve.inbound.cedar: refuse: unknown_audience`. The valve's `known_actors` is seeded only from `[[person]]` + owner (`crates/cli/src/commands/serve.rs` ~L154, TUI host likewise), and the owner's config has no `[[person]]` (only the commented example). So in the Rust MonoAgens **no unregistered sender has ever been served**. The replies the owner saw on 2026-11-23 (`/help` listing `registry.describe`, `/whoami` "person:p_15b623ff (Unverified) … tier guest") came from the older Python project `~/Projects/MonoAgens` (`monoagens/middleware/roles.py`), which was connected to the same WhatsApp number.
- The owner cannot fix this from the TUI: the RBAC editor edits only the `roles` of a `[[person]]`; `owner` and `accounts` are excluded from the settings coverage test (`tui-settings-complete`).
- TUI chat: the first message after start is refused `door Action unknown_capability: refuse: capability 'chat.speak' not disclosed` (event #886, receipt Refused) — body-tui's capabilities are not registered yet when the first turn dispatches; the next turn's `chat.speak` (act-19) then makes `Core::step failed: duplicate terminal receipt for action 'act-19'` (tui.log), i.e. two terminal receipts are submitted for one action.
- Titles fail on OpenCode: `HTTP 400 MissingSessionID … x-opencode-session` — the titler request does not carry `[model.extra_headers]` (the owner's config sets `x-opencode-session = "{conversation_id}"`).
- The provider migration (0fa3369) leaves no visible notice; the owner only finds `.bak` files.

Owner decisions (2026-11-24): unregistered senders become **guests automatically**; **RBAC decides whether a guest may use the agent**. The owner's own WhatsApp account is to be added **through the TUI**, not by editing config.

## What Changes
- Guest access (valve config + gateway): an unregistered sender on an adapter place is admitted as a guest (attested role from `[roles] fallback`, default `guest`) instead of refused as unknown; whether a guest's message may wake the agent is a permission (`agent.converse`) decided by RBAC like any other; gateway slash commands (`/help`, `/whoami`, `/link`) answer guests without the model, filtered by their permissions. Guests are recorded through events, never process memory.
- People editor: `/people` can create a person, set owner, add/remove platform accounts (picked from recent senders seen in the event log, or typed), assign roles; settings coverage includes `[[person]]` owner/accounts.
- TUI fixes: body-tui capabilities registered before the first turn; exactly one terminal receipt per action; titler sends `[model.extra_headers]` with the chat's conversation id; a one-time notice after config migration.

## Impact
`crates/valve` (inbound guest admission + `agent.converse` permission — config-driven, deterministic), `crates/gateway` (guest facts as events, command handling for adapter messages, identity), `crates/tui` (people editor, body-tui registration, receipts, titler headers, migration notice), `crates/cli` (serve seeding). ARCHITECTURE: the valve still decides from facts; guest admission is a configured rule, recorded mode; no core or protocol change.

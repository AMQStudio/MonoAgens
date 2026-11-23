# Tasks — `adapter-and-provider-hotfixes`

## Worker rules
- `source "$HOME/.cargo/env"`; `CARGO_BUILD_JOBS=4 CARGO_PROFILE_TEST_DEBUG=0`; one cargo command at a time; never /tmp; never `tmux kill-server`.
- `plugins/whatsapp` is its own cargo project (own Cargo.lock/target): build/test it there.
- Never touch the owner's `~/.config/monoagens` or `~/.local/share/monoagens`; a release `monoagens` may be running there — do not stop it. No paid model calls, no real WhatsApp/Telegram network calls (fakes/unit tests only).
- Impact analysis before editing a symbol: `node .gitnexus/run.cjs impact "<symbol>" --direction upstream --repo .` (the WhatsApp plugin may not be indexed: confirm by text search).

## 1. WhatsApp identity
- [x] 1.1 In `plugins/whatsapp/src/inbound.rs` resolve DM sender/place and group sender to the phone-number JID: prefer `inbound.info.source.sender_alt` when the sender is a LID, else the client's LID→PN mapping (`whatsapp-rust` 0.7 `client/lid_pn.rs` `get_lid_pn_entry`), else the LID with one info log line. Same for the approval-reaction resolver and group audience members. Outbound replies to the PN place must still reach the chat. Acceptance: unit tests for both spec scenarios plus group.
## 2. Provider key scope
- [x] 2.0 SUPERSEDES the legacy-key part of 2.1 (owner decision 2026-11-24: remove the legacy endpoint). Implement `provider-config-cleanup`: one-time migration of `openai_compatible` + `model_endpoint`/`model_api_key` to a catalog id (match by base URL) or a host-named custom provider, `.bak` for monoagens.toml and secrets.toml, then remove runtime use of the legacy kind and keys (keep `scripted`); named custom providers (`[providers.custom.<name>]` base_url + dialect in monoagens.toml, key in secrets `[providers.<name>] api_key`), "Add custom provider" in `/provider`; remove the unnamed `custom` catalog entry; `/model` lists models only for logged-in providers, others greyed below a separator with no discovery; local keyless providers count as logged in only after the owner enables them in `/provider`. Update the settings coverage test and any spec/config docs that mention `model_endpoint`. Acceptance: the three `provider-config-cleanup` scenarios (migration test uses a temp home with the exact OpenCode Go legacy shape).
- [x] 2.1 `crates/reason/src/secrets.rs` / `crates/gateway/src/provider.rs`: legacy key only for the legacy provider or a catalog entry whose base URL host equals the legacy endpoint host. Acceptance: spec scenario test against the fake server (no request with the key to another host).
- [x] 2.2 Discovery on demand only (active provider at start; others when opened in `/model`/`/provider` and keyed or keyless); unreachable keyless locals show "not running" without a WARN. Acceptance: quiet-startup test counting discovery requests and log warnings.
## 2b. Rename
- [x] 2b.1 Owner decision 2026-11-24: the `/provider` slash command becomes `/provider` (no alias). Rename the command, its slash-menu entry and live-state description, help overlay, `Ctrl+K` palette entry, keybindings action id/label, settings links that open it, smoke script, tests and snapshots. Specs in this change already say `/provider`. Acceptance: `/provider` opens the provider list; `/provider` is not a known command; smoke PASS.
## 3a. Adapter readiness
- [x] 3a.1 Readiness: `status --json` subcommand in `plugins/whatsapp` (paired session present → ready, else `needs: ["pairing"]`, no network) and in the Telegram body (token env present → ready, else `needs: ["secret:<NAME>"]`); gateway supervisor runs it (5 s timeout) before every start; falls back to argument-named secrets; new state `NeedsSetup { needs }`; refuse Start/Restart/enable with the reason; auto-start after pairing succeeds or the secret is set. TUI: sidebar/plugin page show "needs pairing" / "needs NAME" with the fixing action. Acceptance: the three `adapter-readiness` scenarios with the fake plugin extended to emit a status.
## 3. Runtime
- [x] 3.1 Wire the provider rebuild hook in the TUI-hosted gateway path (find where `serve` sets it and do the same in the TUI boot). Acceptance: spec scenario test.
- [x] 3.2 Telegram body: answer `body.discover` before network; honour `retry_after` for 429 on `getUpdates` and `getMe`. Acceptance: tests with a fake Telegram HTTP server.
- [x] 3.3 Supervisor: intentional stop/restart logs once at info, no error "crashed" entry. Acceptance: spec scenario test.
## 4. Finish
- [x] 4.1 Tests for touched crates + `plugins/whatsapp` tests, workspace clippy `-D warnings`, `scripts/tui-smoke.sh` PASS, `openspec validate adapter-and-provider-hotfixes --strict`.

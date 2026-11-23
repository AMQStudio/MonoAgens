# Tasks

Worker rules:
- Run `source "$HOME/.cargo/env"` before `cargo`; use `CARGO_BUILD_JOBS=6 CARGO_PROFILE_TEST_DEBUG=0` for builds/tests.
- Run `node .gitnexus/run.cjs impact "<symbol>" --direction upstream --repo .` before editing functions/methods.
- Never read `lab/scenarios/sealed/`; never run paid live models.
- Finish with `cargo clippy --workspace --all-targets -- -D warnings` and `node .gitnexus/run.cjs detect-changes --scope all --repo .`.

## 1. Adapter DM Addressing & Core Reply Disclosure

- [x] 1.1 Update `WhatsAppInboundTranslator::translate_inbound_action` (`plugins/whatsapp/src/inbound.rs`) so 1:1 DMs set `addressees = vec![ActorId::new("soul")]` and include `"soul"` in `audience`, and add a real-translator unit/integration test (`cargo test -p monoagens-body-whatsapp`) verifying DM `addressees` and `audience`.
- [x] 1.2 Update Telegram inbound translation (`plugins/telegram/src/lib.rs`) so 1:1 DMs include `ActorId::new("soul")` in `addressees` and `audience`, and group mentions/replies to the bot include `"soul"` in `addressees`; verify with `cargo test -p monoagens-body-telegram`.
- [x] 1.3 Update `ToolRagIndex::to_core_catalog` (`crates/gateway/src/prequeue.rs`) to mark `chat.speak` with `is_static_core: true`, and add an E2E test (`crates/gateway/tests/it/guest_tests.rs`) feeding an actual `WhatsAppInboundTranslator` DM percept `"testtt"` (with 28 capabilities registered, `model == None`, guest having `agent.converse` + `chat.speak`) and verifying the agent wakes and replies.

## 2. RBAC Default-Deny, Deduplication & `/help` Line-1 Filtering

- [x] 2.1 Update `RbacConfig::evaluate_with_default_base` and `agent_converse_permitted` (`crates/valve/src/rbac.rs`) to enforce non-owner default-DENY when neither `role.allow` nor an explicit `[permission_bases]` tier grants access, audit/update affected tests in `crates/valve`, and verify with `cargo test -p monoagens-valve`.
- [x] 2.2 Deduplicate `BodyRouter::all_capabilities()` (`crates/gateway/src/waist2.rs`) by `CapabilityId`, add `BodyRouter::capabilities_by_body()`, and update `CommandRegistry::render_help` (`crates/gateway/src/commands.rs`) + `percept_commands::answer` / `GatewayOwner::command_run` to filter out `flags.identity_changing` capabilities for non-owners; verify with `cargo test -p monoagens-gateway` (multi-body `/help` guest test showing zero duplicates and no `workspace.*` or `soul_tuning.set_mode`).

## 3. Enriched `capabilities.list` & Stopped-Plugin Cache

- [x] 3.1 Add a deterministic friendly capability labeler (`display_name` lookup + Title-Case fallback) and `<data_dir>/capability_cache.json` persistence in `crates/gateway`, updating `GatewayOwner::capabilities_list` (`crates/gateway/src/owner.rs`) to return plugin/domain `groups` (`core` + running/stopped plugins with `description`, `flags`, `risk_tier`, `owner_only`) and deduplicated `capabilities`; verify with `cargo test -p monoagens-gateway` covering both connected and stopped cached plugins.

## 4. `config.reload` RPC, `/reload` & `/retry`

- [x] 4.1 Implement `GatewayOwner::config_reload` (`crates/gateway/src/owner.rs`) and `"config.reload"` in `crates/gateway/src/admin.rs` with fail-closed validation and `ReloadReport`, plus `GatewayClient::config_reload` in `crates/tui/src/client.rs`; verify with integration tests for both valid reload and fail-closed invalid TOML in `cargo test -p monoagens-gateway`.
- [x] 4.2 Add `/reload` slash command in `crates/tui` (refreshing config, plugins, and capability hints in both hosting and attached modes) and update `/retry` to retry boot on `config_error` or retry the last interrupted/failed turn when running; verify with `cargo test -p monoagens-tui`.

## 5. Final Verification

- [x] 5.1 Run `openspec validate rbac-guest-converse-and-reload --strict`, `cargo test -p monoagens-valve -p monoagens-gateway -p monoagens-body-whatsapp -p monoagens-body-telegram -p monoagens-tui`, `cargo clippy --workspace --all-targets -- -D warnings`, and `node .gitnexus/run.cjs detect-changes --scope all --repo .`.

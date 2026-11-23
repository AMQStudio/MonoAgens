# Proposal

## Why

Live deployment of commit `2b5f3fc` uncovered three interrelated runtime, security, and operational blockers:
1. **Guests with `agent.converse` still receive no reply on WhatsApp DMs**: Even when `guest` is granted `agent.converse` (confirmed by `/whoami` reporting `use the agent: yes`), `plugins/whatsapp/src/inbound.rs` emits `addressees: Vec::new()` and omits `"soul"` from `audience`. As a result, `CoreState::appraise_percept_record` evaluates `turn_allocated = false`, and without a neural model (`model == None`) `decide_gate_with_actor` short-circuits to `DecisionOutcome::Silent` (`NeutralFallback`). The same DM addressing gap exists in `plugins/telegram/src/lib.rs`, and `crates/gateway/src/prequeue.rs` marks `chat.speak` with `is_static_core: false`, risking eviction under the `k_cap <= 8` Tool RAG budget when 28 capabilities are registered.
2. **Guest `/help` exposes 28 capabilities with duplicates and an RBAC default-ALLOW hole**: `RbacConfig::evaluate_with_default_base` (`crates/valve/src/rbac.rs`) falls through to `Allow` when `[permission_bases]` has no entry and `role.allow` does not match, causing non-owners (including `guest`) to inherit access to `workspace.run`, `workspace.write`, `pdf.delete`, and `soul_tuning.set_mode` in both `/help` and `Valve::pass1_action`. Meanwhile `BodyRouter::all_capabilities` (`crates/gateway/src/waist2.rs`) does not deduplicate across bodies (listing `approval.prompt` and `chat.speak` twice), `CommandRegistry::render_help` ignores `CapabilityFlags::identity_changing`, and `GatewayOwner::capabilities_list` strips `body_id`, `description`, `flags`, and plugin grouping needed for an ergonomic plugin-aware RBAC UI.
3. **No live config reload command exists**: `/retry` only works when startup fails with `config_error`, and there is no `/reload` command or `config.reload` admin RPC to re-read edited configuration and secrets on disk.

## What Changes

- **Adapter DM addressing & reply disclosure**: WhatsApp and Telegram 1:1 DM percepts include `ActorId("soul")` in `addressees` and `audience` (and group mentions/replies to the bot include `"soul"` in `addressees`). `ToolRagIndex::to_core_catalog` keeps `chat.speak` pinned (`is_static_core: true`) so communicative reply capability is never evicted by Tool RAG `k_cap`.
- **RBAC default-DENY for non-owners + capability deduplication & Line-1 `/help` filtering**: **BREAKING** for unconfigured/open non-owner defaults: when RBAC is active, non-owner actors are denied by default unless matched by a role `allow` pattern or an explicit `[permission_bases]` entry satisfied by their tier (and not denied by role/place `deny`). Owner remains `Allow` (or `Hold` under explicit `ApprovalMode::Strict`). `BodyRouter::all_capabilities` deduplicates by `CapabilityId`, and `CommandRegistry::render_help` excludes `identity_changing` capabilities for non-owners.
- **Plugin-grouped enriched `capabilities.list` RPC + stopped-plugin cache**: `capabilities.list` returns deduplicated capability metadata grouped by domain/plugin (`core` plus configured/connected plugins with `id`, `label`, `kind`, `running`, `body_id`, `display_name`, `description`, `flags`, `risk_tier`, `owner_only`) and persists the last discovered capability metadata to `<data_dir>/capability_cache.json` so installed-but-stopped plugins remain discoverable.
- **`config.reload` admin RPC, `/reload` TUI command, and `/retry` cleanup**: Add `config.reload` to `GatewayOwner` / admin socket / `GatewayClient` (fail-closed validation, hot-reloading Valve/RBAC/Persons, PluginSupervisor, and ModelProvider routes), add `/reload` in the TUI, and clarify `/retry` to retry boot on `config_error` or retry the last interrupted/failed turn when running.

## Capabilities

### New Capabilities
- `guest-dm-converse`: Edge adapter DM addressing (`addressees` and `audience` containing `soul` on 1:1 DMs and bot mentions/replies) and static-core `chat.speak` disclosure in Tool RAG.
- `rbac-default-deny-and-discovery`: Non-owner default-deny RBAC evaluation, capability deduplication and Line-1 `/help` filtering, and plugin-grouped `capabilities.list` metadata with `<data_dir>/capability_cache.json` persistence.
- `config-reload`: Fail-closed `config.reload` admin RPC, `/reload` TUI command, and dual-mode `/retry` semantics.

### Modified Capabilities
<!-- No existing main specs under openspec/specs/ require delta modifications; guest-access and tui-settings-complete are unarchived active changes whose behaviors are preserved and tightened here. -->

## Impact

- Crates & plugins affected: `plugins/whatsapp`, `plugins/telegram`, `crates/valve`, `crates/gateway`, `crates/tui` (`client.rs`, `/reload`, `/retry`).
- Architecture invariants preserved: Core remains generic over opaque IDs (`plugins/whatsapp` and `plugins/telegram` translate DM intent at the edge); Valve remains deterministic and first-and-last in every door; Owner supremacy and Absolute Line 1 (`identity_changing` owner-only) are enforced consistently in both enforcement and `/help` display.
- Dependency note: `tui-architecture-redesign` consumes the enriched `capabilities.list` and `config.reload` contracts introduced by this change.

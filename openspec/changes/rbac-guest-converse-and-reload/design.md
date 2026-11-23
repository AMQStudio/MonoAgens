# Design

## Context

See `proposal.md` for motivation and observed live failures. This design covers the backend/protocol-edge, Valve RBAC, Gateway discovery/reload, and TUI client wiring needed before `tui-architecture-redesign`.

Constraints:
- `ARCHITECTURE.md` invariants: Core sees only opaque IDs and never knows about WhatsApp/Telegram; translation happens strictly at the edge (`plugins/whatsapp`, `plugins/telegram`); Valve is deterministic and first-and-last in every door; Owner supremacy and Absolute Line 1 (`identity_changing` owner-only) cannot be weakened.
- `crates/gateway` must remain platform-agnostic.

## Goals / Non-Goals

**Goals:**
- Ensure 1:1 DMs on WhatsApp and Telegram address `"soul"` so permitted guests wake the agent deterministically even with `model == None`, and `chat.speak` is never dropped by Tool RAG `k_cap`.
- Make `RbacConfig` default-deny for non-owners, deduplicate `BodyRouter::all_capabilities()`, and filter `identity_changing` capabilities out of non-owner `/help`.
- Expose plugin-grouped, friendly-labeled capability metadata via `capabilities.list` backed by `<data_dir>/capability_cache.json`.
- Add fail-closed `config.reload` RPC and `/reload` command.

**Non-Goals:**
- TUI layout, focus isolation, paste fixes, or RBAC toggle matrix rendering (handled in `tui-architecture-redesign`).
- Changing Waist-2 wire protocol schemas (uses existing `BodyDiscoverResult` fields + Gateway-side metadata enrichment).

## Decisions

### 1. Edge DM Addressing in WhatsApp & Telegram (`plugins/whatsapp/src/inbound.rs`, `plugins/telegram/src/lib.rs`)
- **Decision**: In `WhatsAppInboundTranslator::translate_inbound_action`, when `!is_group` (1:1 DM), set `addressees = vec![ActorId::new("soul")]` and include `ActorId::new("soul")` in `audience`. In `plugins/telegram/src/lib.rs`, when `!is_group` (private DM), include `ActorId::new("soul")` in `addressees` and `audience`; in groups, push `ActorId::new("soul")` (alongside `self.bot_actor()`) when the bot is `@mentioned` or replied to.
- **Alternative**: Lowering `CoreState::appraise_percept_record` threshold for all `Taint::Untrusted` percepts. *Rejected*: violates Core's silence-in-groups/ambient-noise discipline and Core's platform-agnostic design.

### 2. Core Reply Capability Disclosure (`crates/gateway/src/prequeue.rs`)
- **Decision**: In `ToolRagIndex::to_core_catalog`, set `is_static_core: e.descriptor.id.as_str() == "chat.speak"` (or `e.descriptor.flags.communicative && e.descriptor.id.as_str() == "chat.speak"`).
- **Rationale**: Prevents `k_cap <= 8` Tool RAG truncation from hiding `chat.speak` on short messages like `"testtt"` when 28+ capabilities are active.

### 3. RBAC Default-Deny Semantics (`crates/valve/src/rbac.rs`)
- **Decision**: In `RbacConfig::evaluate_with_default_base`:
  - Step 1 (Owner): `Allow` (or `Hold` if explicit `ApprovalMode::Strict` role).
  - Step 2 (`permission_bases` explicitly `None`): `Deny` (owner-only).
  - Step 3 (Place `deny` / non-empty `allow`): `Deny` if blocked.
  - Step 4 (Role `deny` patterns): `Deny` if matched.
  - Step 5 (Role `allow` patterns): `allow_or_strict_hold` if matched.
  - Step 6 (`permission_bases` / `default_base`): if `Some(Some(required_tier))`, check `tier >= required_tier` -> `allow_or_strict_hold`, else `Deny`. **If `required_base` is `None` (no `permission_bases` entry and no `role.allow` match): return `RbacDecision::Deny` (`"capability '<target>' is not granted to role/tier"`).**
- **Note**: `agent_converse_permitted` when `rbac == Some(cfg)` passes `default_base = Some(RoleTier::Member)` for unregistered guests with `guest_access`, and for registered persons (`registered == true`) passes `default_base = Some(RoleTier::Member)` so registered members keep converse access unless restricted, while guests must explicitly have `agent.converse` in `allow` or have tier `>= Member`.
- **Risk & Mitigation**: Existing unit tests in `crates/valve` and `crates/gateway` that constructed an empty `RbacConfig` expecting non-owners to be allowed will fail if they relied on the old default-ALLOW bug. Those tests will be audited and updated to assert default-deny or explicitly grant the test permission.

### 4. Deduplication, Line-1 `/help` Filtering & Enriched `capabilities.list`
- **Decision**:
  - `BodyRouter::all_capabilities(&self)` deduplicates by `CapabilityId` (merging `places` and preserving the strictest flags if two bodies declare the same ID).
  - Add `BodyRouter::capabilities_by_body(&self) -> BTreeMap<BodyId, Vec<CapabilityDescriptor>>`.
  - `CommandRegistry::render_help` accepts `&[CapabilityDescriptor]` (or a helper `render_help_descriptors`) so it filters out `cap.flags.identity_changing && actor != owner` and marks `approval.prompt` / internal protocol capabilities cleanly. Keep backward-compatible wrapper if needed for existing unit callers.
  - `GatewayOwner::capabilities_list` builds:
    - `groups`: `core` (`agent.converse`, `display_name: "Allow Conversation"`) + every configured plugin in `config.effective_plugins()` joined with `supervisor.list()` (`running`, `body_id`) and live `router.capabilities_by_body()` or `<data_dir>/capability_cache.json` fallback.
    - Deterministic friendly `display_name` helper in `crates/gateway` (shared lookup table for built-in capabilities + Title-Case suffix humanizer for unknown/third-party IDs, e.g., `pdf.read_page` -> `Read Page`).
    - `risk_tier` derived via `classify_action_risk` (`R0`/`R1`/`R2`/`R3`) and `owner_only: cap.flags.identity_changing`.
    - `capabilities`: deduplicated flat list (`[{ id, display_name, description, plugin_id, flags, risk_tier, owner_only }]`) preserving `{ "id": ... }` compatibility.

### 5. `config.reload` & `/reload`
- **Decision**: Add `GatewayOwner::config_reload(&self) -> Result<ReloadReport, AdminError>` reusing the reload pipeline in `GatewayOwner::config_apply` (`load_config_file` -> validate -> diff against previous `GatewayConfig` -> `reload_valve` / `supervisor.reload` / `rebuild_model` without writing a `.bak` or modifying `monoagens.toml`). Expose `"config.reload"` in `crates/gateway/src/admin.rs` and `GatewayClient::config_reload` in `crates/tui/src/client.rs`.

## Risks / Trade-offs

- **RBAC breaking change for misconfigured setups** → Anyone who relied on `[roles.guest]` with `allow = []` previously getting all tools will now get none until enabled; this is the intended security fix and matches user confirmation.
- **Stale `capability_cache.json`** → Mitigated by overwriting per-plugin cache entries atomically on every successful `body/discover` and marking unrunning cached plugins clearly with `running: false`.

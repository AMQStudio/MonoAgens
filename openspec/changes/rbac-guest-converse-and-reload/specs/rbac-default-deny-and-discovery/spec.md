# Spec Delta

## Purpose

Enforces fail-closed default-deny RBAC evaluation for non-owners, deduplicates capability listings across connected bodies, filters owner-only capabilities from `/help`, and exposes plugin-grouped capability discovery metadata.

## ADDED Requirements

### Requirement: RBAC Is Default-Deny for Non-Owners
When RBAC is configured, `RbacConfig::evaluate*` SHALL deny non-owner actors unless matched by a role `allow` pattern or an explicit `[permission_bases]` entry satisfied by their tier (and not blocked by place or role `deny`). The owner remains `Allow` (or `Hold` under explicit `ApprovalMode::Strict`).

#### Scenario: Guest allowlist blocks unlisted capabilities
- **WHEN** `guest` role has `allow = ["chat.*", "web.search", "web.fetch", "agent.converse"]` and `[permission_bases]` is empty
- **THEN** evaluating `workspace.run`, `workspace.write`, `pdf.delete`, and `soul_tuning.set_mode` for a guest returns `RbacDecision::Deny`, while `chat.speak`, `web.search`, `web.fetch`, and `agent.converse` are permitted.

#### Scenario: Explicit permission base still grants tier-based access
- **WHEN** `[permission_bases]` sets `"web.search" = "member"` and an actor holds a `member`-tier role without `"web.search"` in `role.allow`
- **THEN** `"web.search"` is permitted for that `member` actor and denied for a `guest`-tier actor.

### Requirement: Capabilities Are Deduplicated and Filtered by Line 1 in `/help`
`BodyRouter::all_capabilities()` SHALL deduplicate capabilities by `CapabilityId`. `CommandRegistry::render_help` SHALL exclude capabilities marked `flags.identity_changing == true` for non-owners and list each permitted capability at most once.

#### Scenario: Guest `/help` across multiple bodies
- **WHEN** `body-tui`, `body-whatsapp` (both declaring `chat.speak` and `approval.prompt`), `workspace`, `pdf-reader`, `web-search`, and `soul-tuning` (declaring `identity_changing` `soul_tuning.set_mode`) are connected, and a guest with `allow = ["chat.*", "web.search", "web.fetch", "agent.converse"]` sends `/help`
- **THEN** `/help` lists `chat.*`, `web.search`, and `web.fetch` once each, excludes `approval.prompt` and `soul_tuning.set_mode`, excludes `workspace.*` and `pdf.*`, and emits zero duplicate entries.

### Requirement: Enriched Plugin-Grouped Capability Discovery (`capabilities.list`)
`GatewayOwner::capabilities_list` SHALL return plugin/domain `groups` (`core` plus configured/connected plugins with `id`, `label`, `kind`, `running`, `body_id`, and capability entries carrying `id`, `display_name`, `description`, `flags`, `risk_tier`, `owner_only`) alongside a deduplicated flat `capabilities` array.

#### Scenario: Core and connected plugin capabilities include friendly names and metadata
- **WHEN** `capabilities.list` is queried while `whatsapp` is running
- **THEN** `groups` starts with `core` (`agent.converse`, `display_name: "Allow Conversation"`) followed by `whatsapp` (`running: true`) with each capability's `description`, `flags`, `risk_tier`, and `owner_only`.

### Requirement: Cached Capabilities Keep Stopped Plugins Discoverable
Connected bodies' discovered capabilities SHALL be persisted to `<data_dir>/capability_cache.json` so installed-but-stopped plugins remain listed in `capabilities.list` with `running: false`.

#### Scenario: Stopped plugin stays visible in `capabilities.list`
- **WHEN** a plugin such as `telegram` was previously discovered and is later stopped (`running: false`)
- **THEN** `capabilities.list` still includes the `telegram` group with `running: false` and its cached capability metadata from `<data_dir>/capability_cache.json`.

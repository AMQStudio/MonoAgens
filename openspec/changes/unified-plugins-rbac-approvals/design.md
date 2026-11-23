# Design — `unified-plugins-rbac-approvals`

## Core Principle: Open Core, Strict Valve, Zero Hardcoded Scenarios
- `Core` remains completely open and unrestricted; all access control, approval holds, privacy isolation, and budgets live strictly in `Valve` (`crates/valve`).
- No scenario or platform-specific approval trigger is ever hardcoded. Any action held by `Valve` (via `R2`, `ApprovalMode::Strict`, taint quarantine, or model-requested confirmation) produces a generic `PendingApproval` (`action_id`, `h_act`, `state_epoch`, `capability_id`, `target_places`, `summary`, `expires_at_ms`).

## Generic Config Separation (International Plugin Ecosystem)
- `monoagens.toml` stays lean and owns only core/gateway/owner concerns: `[soul]`, `[model]`, `[embed]`, `[valve]`, `[roles]`, `[[person]]`, `[places]`, and `[plugins.<id>]` (`enabled`, `kind = "adapter" | "capability"`, transport, `dependencies`, `optional_dependencies`). Legacy `[[bodies]]` remains accepted as an alias for `[plugins.<id>]` with `kind = "adapter"`.
- Plugin-specific settings NEVER live in `monoagens.toml`. Gateway injects:
  - `MONOAGENS_PLUGIN_CONFIG=<config_dir>/plugins/<id>.toml`
  - `MONOAGENS_PLUGIN_STATE_DIR=<data_dir>/plugins/<id>`
  - `MONOAGENS_BODY_STATE_DIR=<data_dir>/plugins/<id>`
  - `MONOAGENS_RBAC_ENABLED=1|0`
- Each plugin owns, auto-generates on first run, parses, and migrates its own `<id>.toml`. Plugins can never widen RBAC permissions from their own config.

## Generic RBAC-to-Valve Layer (`crates/valve/src/rbac.rs`)
- Ordered tiers: `Guest < Member < Trusted < Admin < Owner`.
- Named roles declare `base: RoleTier`, `allow` / `deny` glob/prefix patterns, optional `approval: Yolo | Default | Strict`, `settings` (e.g. quotas), `label`, and `context`.
- `PlaceAccessRule` allows per-place `allow` / `deny` narrowing (e.g. restricting capabilities in public groups).
- Evaluated deterministically inside `Valve` strictly as a **narrowing** check alongside `cvc5`-verified Cedar policies: it never weakens Absolute Line 1 (`owner`-only `SelfModify`/`identity_changing`), Absolute Line 2 (`VisibleScope`), or taint quarantine.

## Plugin Dependency Resolution & Command Registry (`crates/gateway`)
- `resolve_plugin_boot_order` topologically sorts enabled plugins over `dependencies` and `optional_dependencies` (recognizing virtual built-in `"rbac"`). Missing required dependencies or cycles skip only the affected plugin with a diagnostic warning—never aborting Gateway or sibling plugins.
- `CommandRegistry` indexes built-in commands (`/help`, `/whoami`, `/roles`, `/approve`, `/deny`) plus plugin-declared commands from `body/discover` `_meta.commands`.
- `/help` and registered commands execute directly in Gateway (zero LLM calls) and filter both visibility and execution through `Valve` RBAC for the requesting actor.

## Generic Adapter-Rendered Approvals (`approval.prompt` -> `approval/resolve`)
- Without adding any new Waist-2 RPC method (keeping `monoagens.body.v1` 100% additive and `B-MUST` conformant), adapters may declare capability `approval.prompt`.
- When Gateway holds an action for approval, it dispatches `approval.prompt` with generic fields `{ action_id, h_act, state_epoch, capability_id, target_places, summary, expires_at_ms }` to the adapter owning the prompt place.
- **`body-telegram`**: renders `approval.prompt` as a native message with `inline_keyboard` Approve/Deny buttons (`callback_data` bound to `action_id`); clicking a button auto-acks `answerCallbackQuery` and emits `approval/resolve` carrying the clicking user's `tg:user:<id>` as `resolver` and the bound `(h_act, state_epoch)`.
- **`body-whatsapp`**: renders `approval.prompt` as a WhatsApp message with configurable approve/deny reaction hints (`plugins/whatsapp.toml`: default `👍`/`✅` and `👎`/`❌`), tracks `prompt_message_id -> (action_id, h_act, state_epoch)`, and translates matching inbound `ReactionMessage` events into `approval/resolve` carrying the reacting user's `wa:user:<jid>` as `resolver`.
- **Verification**: Gateway/Valve deterministically checks that `resolver` resolves to `owner` (via `[[person]]`), matches exact `h_act` and `state_epoch`, is within TTL, and settles at most once. Non-owner taps or reactions are refused.

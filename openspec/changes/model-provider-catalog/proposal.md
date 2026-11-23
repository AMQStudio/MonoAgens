# Proposal — `model-provider-catalog`

Part 2 of 5. Audit: [audit.md](../tui-foundation/audit.md) (covers E1-E6 and decision D-5).

**Depends on `tui-foundation`**: the `GatewayClient` trait and admin socket (`models.discover`, `config.apply`
and the live provider hook are used here), the `/log` window (discovery and provider errors), and the fake
provider HTTP server used in tests.

## Why
Verified on 2026-11-23:
- Only one provider kind exists: `openai_compatible` with one hand-typed endpoint (`ModelProviderKind`,
  `crates/gateway/src/config.rs:37`). There is no provider list, model list, key-per-provider, or way to
  switch without editing TOML.
- One model serves reason, classify and consolidate (E4).
- The status bar shows `$0.00` always because no prices exist anywhere (E5).
- Provider errors (rate limit, auth, timeout) are not shown as a clear notice (E6, unverified in the audit:
  verify the current behaviour first).
- The owner's live provider is OpenCode Go; its `GET .../v1/models` with a Bearer key returns 30 models
  (verified with a key).

## What Changes
1. **Data-driven catalog** (`catalog.toml`) of 27 presets plus `custom`, two wire dialects, base URLs probed on
   2026-11-23 (table in `design.md`).
2. **Per-provider keys** in `secrets.toml [providers.<id>]`; lookup order secrets -> catalog env names ->
   legacy `model_api_key`. Existing `provider = "openai_compatible"` keeps working unchanged.
3. **`anthropic_messages` dialect** (native Messages API) next to the parameterised `openai_chat` dialect.
4. **Model discovery** with a per-provider cache and background refresh; admin method `models.discover`.
5. **`/model` two-pane picker and `/login` provider list** (design D10 patterns), with live provider rebuild.
6. **Model per purpose (D-5)**: optional `[model.purposes.<purpose>]` (classify, titler, consolidate), off by
   default; the titler uses classify's model when set.
7. **Cost and usage (E5)**: prices from discovery when returned (OpenRouter does), else user-entered per
   model, else hide the cost; `/usage` shows tokens per purpose and budget left.
8. **Clear provider errors (E6)**: rate limit / auth / timeout shown as a notice with the provider's message
   and a `/log` link.

## Impact
- Crates: `crates/reason` (`model/catalog.rs`, `model/discovery.rs`, `model/anthropic.rs`, `secrets.rs`),
  `crates/gateway` (`config.rs` additive keys, provider rebuild, `models.discover`), `crates/tui` (picker,
  login, `/usage`).
- No core change. The core never sees a provider name: it sees model results and token usage as numbers. No
  protocol change.
- Invariants kept: *core sees only opaque ids* (provider/model live in the gateway/reason layer); *valve first
  and last in every door* (model purposes change which model runs, not which doors run; the budget remains a
  valve rule and is counted per call as before); *gateway generic* (the catalog is data, no per-platform
  code); *reproducibility* (every model call is still a recorded gate decision with its purpose and cost;
  tests use `scripted` and the fake provider server, no paid calls).
- Config files stay backward compatible; new keys are additive.

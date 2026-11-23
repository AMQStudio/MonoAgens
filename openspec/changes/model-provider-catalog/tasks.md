# Tasks — `model-provider-catalog`

## Worker rules
- One cargo command at a time, always with `CARGO_BUILD_JOBS=6 CARGO_PROFILE_TEST_DEBUG=0`.
- Never build or create worktrees in `/tmp`.
- No live paid model calls: use the `scripted` provider and fakes.
- Before editing a symbol run `node .gitnexus/run.cjs impact "<symbol>" --direction upstream --repo .` and report callers, processes and risk.
- Finish each group with `cargo test` for the touched crates and `cargo clippy --workspace --all-targets -- -D warnings`.
- Never read `lab/scenarios/sealed/`.
- Read `design.md` first; the shared audit is [audit.md](../tui-foundation/audit.md).

## 1. Catalog and keys (`crates/reason`, `crates/gateway`)
- [ ] 1.1 `crates/reason/src/model/catalog.rs` + embedded `catalog.toml` with the D6 table (27 providers + `custom`). Acceptance: test lists every id with dialect and base URL.
- [ ] 1.2 Config: `provider = <catalog id>`, `[model.provider_options] base_url`, legacy `openai_compatible` unchanged (`crates/gateway/src/config.rs`). Acceptance: legacy-config test and catalog-id config test.
- [ ] 1.3 Per-provider keys in `crates/reason/src/secrets.rs` with lookup order and masking. Acceptance: lookup-order test; key never appears in log output.
- [ ] 1.4 `openai_chat` dialect parameterised by entry; keyless sends no Authorization header. Acceptance: tests against the fake provider server (`ollama` path, header absent; base URL override).

## 2. Anthropic dialect and discovery (`crates/reason`)
- [ ] 2.1 `anthropic_messages` dialect (check docs with context7). Acceptance: shared-decode test with the OpenAI dialect; request header/body test on the fake server.
- [ ] 2.2 `discovery.rs`: openai_models / anthropic_models / gemini_native, 10 s timeout, per-provider cache with fetch time, `models/` prefix strip. Acceptance: tests with the fake server incl. offline-with-cache.
- [ ] 2.3 Provider error classification (429/auth/timeout) and `provider` log entries. Acceptance: first a test recording today's behaviour, then fake-server 429 test.

## 3. Live rebuild and admin method (`crates/gateway`)
- [ ] 3.1 Provider rebuild hook called by `config.apply` (main and titler provider). Acceptance: test switches model and sees the new id in the next request.
- [ ] 3.2 Admin method `models.discover` (shape proposed in design, confirm in review). Acceptance: test through the socket client and the in-process client.

## 4. Picker and login (`crates/tui`)
- [ ] 4.1 `/model` two-pane picker (D10): cache first, background refresh, `r`, search, scrollbar, detail line, manual id entry, unconfigured-provider key detour. Acceptance: `TestBackend` tests + snapshots at 80x24 and 120x40.
- [ ] 4.2 `/login` list + masked key entry + connection test; slash descriptions with live state. Acceptance: test against the fake server; `secrets.toml` mode 0600.
- [ ] 4.3 Error notices for provider failures. Acceptance: 429 scenario test.

## 5. Purposes, cost, usage (`crates/reason`, `crates/gateway`, `crates/tui`)
- [ ] 5.1 `[model.purposes.<purpose>]` config + routing; titler uses classify's model. Acceptance: tests from the spec scenarios (check purpose names in code first).
- [ ] 5.2 Prices: from discovery (`ModelInfo`) or `[model.prices."<provider>/<model>"]`; hide cost when unknown. Acceptance: status bar tests for both scenarios.
- [ ] 5.3 `/usage` view. Acceptance: test with one reasoning and one classify call.

## 6. Finish
- [ ] 6.1 Extend `scripts/tui-smoke.sh`: `/model` opens, search narrows the list (fake provider server). Acceptance: script exits 0.
- [ ] 6.2 `openspec validate model-provider-catalog --strict`; GitNexus `detect_changes --scope all` reviewed; `cargo test` for touched crates; clippy as in the rules above.

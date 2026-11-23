# Proposal — `capability-plugins-port`

## Why
With unified `[plugins.<id>]` (`kind = "adapter" | "capability"`), generic RBAC narrowing in Valve, dependency resolution, and self-managed per-plugin configs (`MONOAGENS_PLUGIN_CONFIG` / `MONOAGENS_PLUGIN_STATE_DIR`) now in place, porting the four core capability plugins from the previous Python MonoAgens (`web-search`, `workspace`, `pdf-reader`, `soul-tuning`) into pure-Rust Waist-2 plugins equips Mono with real web research, sandboxed file/command workspaces, RAG-based PDF retrieval, and owner-controlled speech-mode tuning across Telegram, WhatsApp, and CLI.

## What Changes
- **Add `plugins/plugin-web-search`**: Pure-Rust Waist-2 capability plugin exposing `web.search` (SearXNG/Tavily/Brave/DuckDuckGo provider chain) and `web.fetch` (HTML-to-text with strict fail-closed SSRF private-network/port/redirect guards) plus self-managed `web-search.toml`.
- **Add `plugins/plugin-workspace`**: Pure-Rust Waist-2 capability plugin (`optional_dependencies = ["rbac"]`) exposing `workspace.read`, `workspace.write`, `workspace.edit`, `workspace.grep`, `workspace.glob`, and bounded `workspace.run` over strictly isolated per-actor directories with file-size & role-quota enforcement plus self-managed `workspace.toml`.
- **Add `plugins/plugin-pdf-reader`**: Pure-Rust RAG-first Waist-2 capability plugin (`lopdf`) exposing `pdf.index`, `pdf.search` (page-cited hybrid BM25 + optional EmbeddingGemma 2 cosine RRF retrieval), `pdf.read_page` (bounded page range), `pdf.list`, and `pdf.delete` over a persistent local RAG index in `MONOAGENS_PLUGIN_STATE_DIR` plus self-managed `pdf-reader.toml`.
- **Add `plugins/plugin-soul-tuning`**: Pure-Rust Waist-2 capability plugin exposing `soul_tuning.get_mode`, `soul_tuning.list_modes`, and owner-only (`identity_changing: true`) `soul_tuning.set_mode` with bounded `talkativeness_offset` (`[-0.2, 0.2]`), few-shot examples, persistent active mode, and self-managed `soul-tuning.toml`.
- Verify every plugin passes unit/integration tests, strict Clippy (`-D warnings`), and `monoagens-conformance body --level should` (`10/10`).

## Impact
- `Cargo.toml` & `Cargo.lock` (adds 4 workspace members under `plugins/`).
- `plugins/plugin-web-search/`, `plugins/plugin-workspace/`, `plugins/plugin-pdf-reader/`, `plugins/plugin-soul-tuning/`.
- `docs/messaging-bodies.md` (adds capability plugin examples in `[plugins.<id>]`).
- Zero changes to `crates/core`, `crates/valve`, `crates/gateway`, or `ARCHITECTURE.md`.

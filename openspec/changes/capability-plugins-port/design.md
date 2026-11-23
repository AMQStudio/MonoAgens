# Design — `capability-plugins-port`

## Shared Plugin Contract
All four plugins live under `plugins/` as pure-Rust (`#![forbid(unsafe_code)]`) Waist-2 (`monoagens.body.v1`) executables built atop `monoagens-body-kit` (`PersistentBodyRuntime`) and `monoagens-body-sdk`:
- **Self-managed config**: On startup, if `MONOAGENS_PLUGIN_CONFIG` is set and missing, each plugin writes a commented default `<name>.toml` and loads settings from it. When unset (e.g. during conformance runs), defaults apply without touching the filesystem.
- **State isolation**: Persistent state (workspace roots, PDF RAG index, active soul-tuning mode, Waist-2 crash journal) uses `MONOAGENS_PLUGIN_STATE_DIR` (falling back to `MONOAGENS_BODY_STATE_DIR`).
- **Pure NDJSON `stdout`**: All diagnostics log strictly to `stderr`.
- **No RBAC self-escalation**: Plugins never override Valve decisions; `plugin-soul-tuning`'s `soul_tuning.set_mode` sets `identity_changing: true` so Valve's `cvc5`-proven Absolute Line 1 enforces owner-only access automatically.

## RAG-First `plugin-pdf-reader`
Rather than dumping raw multi-hundred-page PDFs into reasoning context, `plugin-pdf-reader` extracts per-page text with `lopdf`, splits each page into overlapping chunks (`chunk_chars = 900`, `chunk_overlap_chars = 150`), and persists an indexed store (`pdf_rag_index.json` in `MONOAGENS_PLUGIN_STATE_DIR`). Queries via `pdf.search` always execute BM25 lexical scoring over chunks and, when an embedding HTTP sidecar (`embed_endpoint` or `MONOAGENS_EMBED_ENDPOINT`) is configured, fuse BM25 with cosine similarity via Reciprocal Rank Fusion (RRF), returning compact page-cited snippets (`[p. N]`). `pdf.read_page` allows bounded inspection (default max 5 pages/call) around cited pages.

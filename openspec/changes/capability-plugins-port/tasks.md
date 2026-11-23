# Tasks — `capability-plugins-port`

## 1. Capability Plugins (`plugins/`)
- [x] 1.1 Implement `plugins/plugin-web-search` (`web.search`, `web.fetch`, SSRF guard, self-managed `web-search.toml`, tests, clippy, `10/10` conformance).
- [x] 1.2 Implement `plugins/plugin-workspace` (`workspace.read/write/edit/grep/glob/run`, per-actor path containment, quota & optional RBAC support, self-managed `workspace.toml`, tests, clippy, `10/10` conformance).
- [x] 1.3 Implement `plugins/plugin-pdf-reader` (`pdf.index`, `pdf.search` hybrid BM25+RRF RAG, `pdf.read_page`, `pdf.list`, `pdf.delete`, `lopdf`, self-managed `pdf-reader.toml`, tests, clippy, `10/10` conformance).
- [x] 1.4 Implement `plugins/plugin-soul-tuning` (`soul_tuning.get_mode`, `soul_tuning.set_mode` with `identity_changing: true`, `soul_tuning.list_modes`, clamped `[-0.2, 0.2]` talkativeness offset, self-managed `soul-tuning.toml`, tests, clippy, `10/10` conformance).

## 2. Integration & Verification
- [x] 2.1 Update `docs/messaging-bodies.md` with capability plugin config snippets (`[plugins.<id>]` with `kind = "capability"`).
- [x] 2.2 Run `openspec validate capability-plugins-port --strict`, `monoagens-conformance body --level should` on all 4 plugins, `cargo clippy --workspace --all-targets -- -D warnings`, and GitNexus `detect_changes`.

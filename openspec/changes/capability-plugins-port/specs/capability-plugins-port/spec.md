# Specification — `capability-plugins-port`

## ADDED Requirements

### Requirement: Web Search and SSRF-Guarded Fetch Capability Plugin (`plugin-web-search`)
The system SHALL provide a pure-Rust Waist-2 capability plugin `plugin-web-search` declaring `web.search` and `web.fetch`, enforcing fail-closed SSRF protection against private, loopback, link-local, and reserved IP ranges and disallowed ports, managing its own `web-search.toml` via `MONOAGENS_PLUGIN_CONFIG`, and passing `monoagens-conformance body --level should`.

#### Scenario: SSRF-blocked URLs are refused while public pages and search results succeed
- **WHEN** `web.fetch` is called with a loopback or link-local metadata address (e.g., `http://127.0.0.1/` or `http://169.254.169.254/`)
- **THEN** `plugin-web-search` refuses the request with a typed error without issuing an outbound request
- **AND** valid `web.search` and public `web.fetch` requests return normalized text and structured results within `max_chars` / `max_results` bounds.

### Requirement: Sandboxed Per-Actor Workspace Capability Plugin (`plugin-workspace`)
The system SHALL provide a pure-Rust Waist-2 capability plugin `plugin-workspace` declaring `workspace.read`, `workspace.write`, `workspace.edit`, `workspace.grep`, `workspace.glob`, and bounded `workspace.run`, strictly containing all file operations within each actor's sandbox directory, enforcing file-size and optional RBAC role quotas, managing its own `workspace.toml`, and passing `monoagens-conformance body --level should`.

#### Scenario: Path traversal is rejected and workspace operations stay isolated per actor
- **WHEN** an actor attempts `workspace.read` or `workspace.write` with `../` traversal outside their root or exceeds their configured quota
- **THEN** `plugin-workspace` refuses the action cleanly while valid read/write/edit/grep/glob/run actions execute deterministically inside the actor's directory.

### Requirement: RAG-First PDF Reader Capability Plugin (`plugin-pdf-reader`)
The system SHALL provide a pure-Rust Waist-2 capability plugin `plugin-pdf-reader` declaring `pdf.index`, `pdf.search`, `pdf.read_page`, `pdf.list`, and `pdf.delete`, extracting per-page text via `lopdf`, persisting overlapping page-cited chunks in `MONOAGENS_PLUGIN_STATE_DIR`, retrieving top-K relevant chunks via BM25 (and hybrid cosine RRF when an embedding endpoint is configured), managing its own `pdf-reader.toml`, and passing `monoagens-conformance body --level should`.

#### Scenario: Indexing a multi-page PDF enables page-cited RAG search without dumping raw document text
- **WHEN** `pdf.index` ingests a multi-page PDF and `pdf.search` queries a topic appearing on a specific page
- **THEN** `pdf.index` returns compact document metadata without raw full text and `pdf.search` returns the matching top-K chunks with accurate page citations.

### Requirement: Speech Mode Tuning Capability Plugin (`plugin-soul-tuning`)
The system SHALL provide a pure-Rust Waist-2 capability plugin `plugin-soul-tuning` declaring `soul_tuning.get_mode`, `soul_tuning.list_modes`, and owner-only (`identity_changing: true`) `soul_tuning.set_mode`, clamping `talkativeness_offset` to `[-0.2, 0.2]`, persisting the active mode in `MONOAGENS_PLUGIN_STATE_DIR`, managing its own `soul-tuning.toml`, and passing `monoagens-conformance body --level should`.

#### Scenario: Mode switching persists across restarts and marks `set_mode` as `identity_changing`
- **WHEN** `body/discover` is queried on `plugin-soul-tuning` and `soul_tuning.set_mode` switches from `daily` to `professional`
- **THEN** `soul_tuning.set_mode` declares `identity_changing: true` and the switched mode persists across plugin restarts in `MONOAGENS_PLUGIN_STATE_DIR`.

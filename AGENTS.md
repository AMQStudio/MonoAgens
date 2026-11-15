# AGENTS.md

`openspec/specs/` is the source of truth for behavior. Every feature or behavior change goes
through a change in `openspec/changes/`. Never implement unapproved work or broad unrelated
refactors.

## OpenSpec

- `openspec list`
- `openspec validate <change-name> --strict`
- `openspec archive <change-name>`

## Commands

- One test: `cargo test -p monoagens-core <test_name>`
- Lint: `cargo clippy --workspace -- -D warnings`

## Dependencies & Documentation

- Always target the **latest stable release** of Rust (`rustup` stable, Edition 2024), TypeScript/Python runtimes, and Cargo/npm/PyPI dependencies pinned in lockfiles (`Cargo.lock`, `uv.lock`, `package-lock.json`) rather than freezing stale patch versions in specs.
- Query the **`context7`** MCP (`resolve-library-id` -> `query-docs`) for up-to-date official documentation and APIs before implementing code, SDK adapters, or external benchmark wrappers (`cedar-policy`, `cedar-policy-symcc`, `rusqlite`, `usearch`, `hnsw_rs`, `ort`, `jsonrpsee`, `schemars`, `proptest`, `criterion`, `mem0`, `letta-client`, `langgraph`, `graphiti`, etc.).

## Gotchas

- No code exists yet, and `cargo` is not installed (use `rustup`).
- Only the owner changes the invariants in `ARCHITECTURE.md`.
- Never read `lab/scenarios/sealed/`.
- Live Lab runs call a paid model. Run them only when asked.

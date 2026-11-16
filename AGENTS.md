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

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **MonoAgens** (10046 symbols, 33986 relationships, 404 execution flows).

> Index stale? Run `node .gitnexus/run.cjs analyze --index-only` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? Bootstrap with `npx`, `bunx`, or `pnpm dlx` — e.g. `bunx gitnexus@latest analyze` (npm 11 npx crash; #1939).

## Always Do

- **MUST run impact before editing.** Use `impact({target: "symbolName", direction: "upstream"})` or `node .gitnexus/run.cjs impact "symbolName" --direction upstream --repo .`; report callers, processes, and risk. Never substitute grep for graph analysis.
- **MUST analyze graph changes before committing.** Use `detect_changes({scope: "all"})` (MCP) or `node .gitnexus/run.cjs detect-changes --scope all --repo .` (CLI fallback). `partial: true` or `truncated: true` is not a clean check — a zero means unseen, not unaffected; re-run it. For regression review: `detect_changes({scope: "compare", base_ref: "main"})` or `node .gitnexus/run.cjs detect-changes --scope compare --base-ref "main" --repo .`.
- MUST warn on HIGH/CRITICAL `risk` pre-edit; never use `riskSharedAxes` to waive a HIGH/CRITICAL `risk` warning. Compare File/symbol: MCP File omits axes; Graph-RAG expands File.
- **MUST treat `risk: UNKNOWN` as unresolved, not as low.** An empty caller set is not evidence the symbol is unused — it can also mean the callers are not resolvable by the index (plain-object property access, dynamic dispatch, cross-language calls). `impact` pairs `UNKNOWN` with a `riskNote` saying so. Confirm with a text search before treating the symbol as safe to change or delete; do not proceed on the strength of a zero.
- **MUST use `query({search_query: "concept"})` for concepts/flows, `context({name: "symbolName"})` for a named symbol, or `impact` for blast radius, on read-only callers, dependencies, imports, or execution flow.** Graph first; text search only for empty/`UNKNOWN`/literals.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

## Never Do

- NEVER edit a function, class, or method before MCP/CLI impact analysis.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis, and never read `UNKNOWN` as an all-clear — it means the walk could not answer, which is the one verdict that requires confirming by other means.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit before MCP/CLI graph change analysis.

## Resources

| Resource | Use for |
| --- | --- |
| `gitnexus://repo/MonoAgens/context` | Codebase overview, check index freshness |
| `gitnexus://repo/MonoAgens/clusters` | All functional areas |
| `gitnexus://repo/MonoAgens/processes` | All execution flows |
| `gitnexus://repo/MonoAgens/process/{name}` | Step-by-step execution trace |

## CLI

| Task | Read this skill file |
| --- | --- |
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->

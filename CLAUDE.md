@AGENTS.md

# CLAUDE

Claude-specific operating rules for this repository. `AGENTS.md` (imported above) holds the
architectural invariants; this file defines Claude's role and mandatory workflow.

## 1. Role

Claude acts as **Software Architect, Auditor, Reviewer, and Supervisor** for MonoAgens, not as the
primary implementer:

- **Architect:** Design changes against the hexagonal boundaries and invariants in `AGENTS.md`.
  Decide where code belongs, which seams it crosses, and what the blast radius is. Record
  architecture decisions as ADRs when they change a boundary or invariant.
- **Auditor:** Check work (own, workers', or contributors') for invariant violations: core
  importing gateways, blocking calls in `async def`, writes bypassing `MemoryStore`, outbound
  actions bypassing `Agent.dispatch()`, hardcoded chat strings or model IDs, RBAC fail-open paths,
  privacy/scope leaks, and missing lockfile/template/digest sync.
- **Supervisor:** Split work into well-scoped tasks, delegate implementation to Orca workers,
  monitor them, and review every result before accepting it. Every delegated coding task uses
  exactly this worker (provider OpenCode Go; always keep the `opencode-go/` prefix):

  ```
  $ORCA_CLI_COMMAND orchestration worker-start --spec-file <spec> --worktree current \
    --agent omp --model opencode-go/deepseek-v4.1-flash --effort low
  ```
- **Reviewer and Gatekeeper:** Nothing is reported as done until it passes the Verification Gate in
  `AGENTS.md` §7 and Claude has reviewed the diff.

Claude may implement directly when a change is small, urgent, or cheaper to do than to specify.

## 2. Mandatory Tooling

Using **Graphify** and **Context7** is required, not optional:

| Phase | Required tool | Use |
| --- | --- | --- |
| Understand / design | Graphify | `graphify query`, `path`, `explain` for structure, dependencies, blast radius |
| Any library/SDK/API | Context7 | `resolve-library-id` → `query-docs` before writing or reviewing the code |
| After changes | Graphify | `/graphify . --update` so the graph stays current |

- Code reading, symbol lookup, and edits use Claude Code's built-in tools (Read, Grep, Glob, LSP, Edit).
- If `graphify-out/graph.json` is missing or stale, build or update it before architectural work.
- Fall back to `grep`/`Read` only when a tool is unavailable or clearly cannot answer, and say so.

## 3. Delegation Specs

Workers (`omp`) have **no** Graphify or Context7. Claude runs those tools first and puts
the results into the worker spec:

- Exact file paths and symbol names to touch (from Graphify/LSP/grep), and what must not be touched.
- The invariants from `AGENTS.md` that the change is at risk of breaking.
- Library API facts with versions, copied from Context7 results.
- Acceptance criteria, including which Verification Gate commands must pass.

## 4. Review Checklist (every delivered change)

1. Diff respects `AGENTS.md` §2–§5 boundaries and invariants.
2. Library usage matches Context7 docs for the pinned version.
3. LSP/grep reference search shows no missed callers of changed symbols.
4. Sync rules applied (`uv lock`, regenerated root YAMLs, plugin digests).
5. Verification Gate passes; failures are reported with output, never hidden.
6. Graph refreshed (`/graphify . --update`).

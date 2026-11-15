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

## Gotchas

- No code exists yet, and `cargo` is not installed (use `rustup`).
- Only the owner changes the invariants in `ARCHITECTURE.md`.
- Never read `lab/scenarios/sealed/`.
- Live Lab runs call a paid model. Run them only when asked.

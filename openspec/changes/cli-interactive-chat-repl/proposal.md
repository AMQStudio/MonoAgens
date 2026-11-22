# Proposal

## Why

`monoagens chat` currently reads all of `stdin` until EOF (`Ctrl+D`) before processing messages, preventing users from holding a natural real-time interactive conversation with MonoAgens V2.2 or testing multi-room / multi-speaker dynamics (`Silent`, `Wait`, `Go`, `compute_tier`, DM vs group privacy) directly from the terminal.

## What Changes

- Upgrade `monoagens chat` in `crates/cli/src/lib.rs` to run a live line-by-line interactive REPL whenever `stdin` is a terminal (or `--interactive` is set) while preserving 100% backward compatibility for `--message`, `--json`, and non-TTY piped `stdin`.
- Add additive `ChatArgs` options (`--model <path>`, `--place <id>`, `--show-brain`, `--interactive`) and in-REPL slash commands (`/dm`, `/room <id>`, `/group <room> <members>`, `/as <actor>`, `/brain`, `/status`, `/help`, `/quit`).
- Render a compact neural decision status line (`[otak: outcome | reason | tier | p_go | p_corr]`) from the turn's recorded `DecisionTrace` when `--show-brain` or interactive mode is active.

## Capabilities

### New Capabilities
- `cli-interactive-chat`: Real-time terminal REPL for `monoagens chat` with multi-room/multi-speaker simulation, `@mono`/`@soul` addressing in group rooms, optional `--model` override, and live neural decision trace summaries.

### Modified Capabilities
- *(none)*

## Impact

- `crates/cli/src/lib.rs` and `crates/cli/tests/` only; zero changes to `crates/core`, `crates/protocol`, `crates/valve`, `ARCHITECTURE.md`, or `lab/scenarios/sealed/`.

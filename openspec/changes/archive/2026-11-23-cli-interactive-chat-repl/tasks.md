## 1. Interactive CLI Chat REPL

- [x] 1.1 Add additive `ChatArgs` flags (`--model`, `--place`, `--show-brain`, `--interactive`, optional `--actors`) and `ReplSessionState` + slash commands (`/dm`, `/room`, `/group`, `/as`, `/brain`, `/status`, `/help`, `/quit`) in `crates/cli/src/lib.rs`, with live interactive actor registration via `CliInteractiveDoorController` and deliverable channel place preservation.
- [x] 1.2 Implement live line-by-line interactive turn loop with per-turn `[otak: ...]` `DecisionTrace` summary, immediate short-circuit when gate decides `Silent`/`Note`/`DoorBlocked`, and full backward compatibility for `--message` / piped stdin / `--json`.
- [x] 1.3 Add unit/integration tests for REPL commands, group `@mono` addressing, brain trace formatting, and build `./target/release/monoagens` (`cargo test -p monoagens-cli && cargo clippy -p monoagens-cli --all-targets -- -D warnings && cargo build --release -p monoagens-cli`).

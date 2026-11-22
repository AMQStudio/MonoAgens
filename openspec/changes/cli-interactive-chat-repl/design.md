# Design: Interactive `monoagens chat` REPL

- Extend `ChatArgs` in `crates/cli/src/lib.rs` with additive flags (`--model`, `--place`, `--show-brain`, `--interactive`) and a small `ReplSessionState` (`place`, `actor`, `audience`, `is_group`, `show_brain`).
- Extract a clean helper `parse_repl_command(line, &mut state)` + `build_turn_percept(...)` so both interactive TTY input and unit tests exercise the exact same room/group/speaker/`@mono` addressing logic.
- In interactive mode, read lines incrementally via `stdin.lock().read_line(...)` inside the active Tokio runtime, submit each percept immediately via `handle.submit_and_wait(...)`, wait for any emitted `body-cli` response bubbles (bounded timeout; immediate exit when the turn's `DecisionTrace` is `Silent`/`Note`/`DoorBlocked`), print `[otak: ...]` summary from the newly appended `Decision` event, and print Mono's response immediately.
- Preserve existing non-TTY piped & `--message` & `--json` behavior completely untouched.

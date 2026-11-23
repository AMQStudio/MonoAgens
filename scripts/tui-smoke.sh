#!/usr/bin/env bash
# TUI smoke test: drives the debug binary inside tmux with the scripted provider.
# Home lives under target/tui-smoke/ (never /tmp). No paid model calls.
# Usage: scripts/tui-smoke.sh   (after `cargo build -p monoagens-cli`)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN="${MONOAGENS_BIN:-$TARGET/debug/monoagens}"
SESSION="wt-b-smoke-$$"
HOME_DIR="$ROOT/target/tui-smoke/home-$$"
COLS=120
ROWS=40

fail() { echo "FAIL: $*" >&2; dump; exit 1; }
dump() { tmux capture-pane -p -t "$SESSION" 2>/dev/null | sed 's/^/  | /' >&2 || true; }
cleanup() { tmux kill-session -t "$SESSION" 2>/dev/null || true; rm -rf "$HOME_DIR"; }
trap cleanup EXIT

screen() { tmux capture-pane -p -t "$SESSION"; }

# wait_for <text> [timeout_seconds]
wait_for() {
  local needle="$1" timeout="${2:-15}" i=0
  while (( i < timeout * 10 )); do
    if screen | grep -qF -- "$needle"; then return 0; fi
    sleep 0.1; i=$((i + 1))
  done
  fail "timed out waiting for: $needle"
}

[[ -x "$BIN" ]] || { echo "missing binary: $BIN (build with cargo build -p monoagens-cli)" >&2; exit 2; }
command -v tmux >/dev/null || { echo "tmux not installed" >&2; exit 2; }

mkdir -p "$HOME_DIR/data"
cat > "$HOME_DIR/monoagens.toml" <<TOML
version = 1
data_dir = "$HOME_DIR/data"

[soul]
name = "MonoSmoke"
seed = 1

[model]
provider = "scripted"
model = "scripted-test"
daily_token_budget = 10000

[valve]
mode = "enforcing"
max_chain_wakes = 8
TOML

tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" \
  "env HOME='$HOME_DIR' XDG_CONFIG_HOME='$HOME_DIR' TERM=xterm-256color '$BIN' --config '$HOME_DIR/monoagens.toml'; echo \"exit=\$?\" > '$HOME_DIR/exit.code'; sleep 30"

# 1. starts and shows the composer (status bar model + composer hint)
wait_for "scripted-test"
wait_for "enter send"
echo "ok: composer visible"

# 2. quits cleanly
tmux send-keys -t "$SESSION" "/quit" Enter
i=0
while [[ ! -f "$HOME_DIR/exit.code" ]]; do
  sleep 0.1; i=$((i + 1))
  (( i < 150 )) || fail "TUI did not exit after /quit"
done
grep -qx "exit=0" "$HOME_DIR/exit.code" || fail "non-zero exit: $(cat "$HOME_DIR/exit.code")"
echo "ok: clean quit (exit 0)"

# Later tasks (8.1) add: autostart dot, /log line, crash-restart colours,
# silent-turn notice, attach-then-detach.
echo "tui-smoke: PASS"

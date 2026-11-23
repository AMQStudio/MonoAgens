#!/usr/bin/env bash
# TUI smoke test: drives the debug binary inside tmux. Home lives under target/tui-smoke/
# (never /tmp). No paid model calls: the provider is a tiny local fake HTTP server (python3) whose
# canned reply is switched by the script; the plugin is the gateway's `fake_plugin` example.
#
# Checks: composer, plugin autostart dot (green), `/log` shows the plugin stderr line, crash -> red
# -> green after restart, silent-turn notice, a spoken reply, attach to `monoagens serve` and detach
# leaving serve running.
#
# Usage: scripts/tui-smoke.sh   (after `cargo build -p monoagens-cli`; the fake plugin is built on
# demand with `cargo build --example fake_plugin -p monoagens-gateway`)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN="${MONOAGENS_BIN:-$TARGET/debug/monoagens}"
FAKE_PLUGIN="${FAKE_PLUGIN:-$TARGET/debug/examples/fake_plugin}"
SESSION="wt-c-smoke-$$"
HOME_DIR="$ROOT/target/tui-smoke/home-$$"
COLS=140
ROWS=40
PROVIDER_PID=""
SERVE_PID=""

dump() { tmux capture-pane -p -t "$SESSION" 2>/dev/null | sed 's/^/  | /' >&2 || true; }
fail() { echo "FAIL: $*" >&2; dump; exit 1; }
cleanup() {
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  [[ -n "$SERVE_PID" ]] && kill "$SERVE_PID" 2>/dev/null || true
  [[ -n "$PROVIDER_PID" ]] && kill "$PROVIDER_PID" 2>/dev/null || true
  rm -rf "$HOME_DIR"
}
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

# wait_for_absent <text> [timeout_seconds]
wait_for_absent() {
  local needle="$1" timeout="${2:-10}" i=0
  while (( i < timeout * 10 )); do
    if ! screen | grep -qF -- "$needle"; then return 0; fi
    sleep 0.1; i=$((i + 1))
  done
  fail "timed out waiting for this to disappear: $needle"
}

# dot_color <plugin-id>: prints green|yellow|red|other|none for the sidebar dot of the plugin
dot_color() {
  tmux capture-pane -e -p -t "$SESSION" | python3 -c '
import re, sys
plugin = sys.argv[1]
fg = None
for line in sys.stdin.read().split("\n"):
    plain = re.sub(r"\x1b\[[0-9;]*m", "", line)
    if plugin not in plain or " (" not in plain:
        continue
    fg = None
    pos = 0
    for m in re.finditer(r"\x1b\[([0-9;]*)m|(.)", line):
        if m.group(1) is not None:
            p = m.group(1).split(";") if m.group(1) else ["0"]
            i = 0
            while i < len(p):
                if p[i] in ("0", ""):
                    fg = None
                elif p[i] == "38" and i + 2 < len(p) and p[i + 1] == "5":
                    fg = int(p[i + 2]); i += 2
                elif p[i] == "38" and i + 4 < len(p) and p[i + 1] == "2":
                    fg = ("rgb",) + tuple(int(x) for x in p[i + 2:i + 5]); i += 4
                elif p[i].isdigit() and 30 <= int(p[i]) <= 37:
                    fg = int(p[i]) - 30
                elif p[i].isdigit() and 90 <= int(p[i]) <= 97:
                    fg = int(p[i]) - 90 + 8
                i += 1
        elif m.group(2) in ("●", "◐", "◉"):
            name = {1: "red", 9: "red", 2: "green", 10: "green", 3: "yellow", 11: "yellow"}.get(fg, "other")
            print(name); sys.exit(0)
print("none")
' "$1"
}

# wait_for_dot <plugin-id> <color> [timeout_seconds]
wait_for_dot() {
  local plugin="$1" want="$2" timeout="${3:-15}" i=0 got=""
  while (( i < timeout * 10 )); do
    got="$(dot_color "$plugin")"
    [[ "$got" == "$want" ]] && return 0
    sleep 0.1; i=$((i + 1))
  done
  fail "timed out waiting for the '$plugin' dot to be $want (now $got)"
}

# Esc is sent alone, then we wait for the overlay title to go away.
close_overlay() {
  tmux send-keys -t "$SESSION" Escape
  wait_for_absent "$1" 5
}

start_tui() {
  rm -f "$HOME_DIR/exit.code"
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" \
    "env HOME='$HOME_DIR' XDG_CONFIG_HOME='$HOME_DIR' TERM=xterm-256color '$BIN' --config '$HOME_DIR/monoagens.toml'; echo \"exit=\$?\" > '$HOME_DIR/exit.code'; sleep 30"
}

quit_tui() {
  tmux send-keys -t "$SESSION" "/quit" Enter
  local i=0
  while [[ ! -f "$HOME_DIR/exit.code" ]]; do
    sleep 0.1; i=$((i + 1))
    (( i < 150 )) || fail "TUI did not exit after /quit"
  done
  grep -qx "exit=0" "$HOME_DIR/exit.code" || fail "non-zero exit: $(cat "$HOME_DIR/exit.code")"
}

[[ -x "$BIN" ]] || { echo "missing binary: $BIN (build with cargo build -p monoagens-cli)" >&2; exit 2; }
command -v tmux >/dev/null || { echo "tmux not installed" >&2; exit 2; }
command -v python3 >/dev/null || { echo "python3 not installed (needed for the fake provider)" >&2; exit 2; }
if [[ ! -x "$FAKE_PLUGIN" ]]; then
  echo "building the fake plugin ..."
  (cd "$ROOT" && CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-3}" CARGO_PROFILE_TEST_DEBUG=0 \
    cargo build --example fake_plugin -p monoagens-gateway) >&2
fi
[[ -x "$FAKE_PLUGIN" ]] || { echo "missing fake plugin: $FAKE_PLUGIN" >&2; exit 2; }

mkdir -p "$HOME_DIR/data"

# --- fake provider: replies with $HOME_DIR/reply.json (switched by the script) -----------------
python3 - "$HOME_DIR" <<'PY'
import json, sys
home = sys.argv[1]
think = {"act": "think", "thought": "answer the owner", "motivation": 4, "confidence_0_20": 18,
         "used_ids": [], "importance": 0.6, "basis": "observed", "value": None, "cost": None,
         "domain": None}
speak = {"act": "act", "capability_id": "chat.speak", "places": ["cli:terminal"],
         "people": ["per_owner"], "payload": list("pong smoke".encode()), "motivation": 4,
         "confidence_0_20": 18, "used_ids": [], "expected": None, "domain": None}
v = {"major": 1, "minor": 0}
json.dump({"protocol_version": v, "acts": [think, speak]}, open(home + "/speak.json", "w"))
json.dump({"protocol_version": v, "acts": [think]}, open(home + "/silent.json", "w"))
PY
cp "$HOME_DIR/silent.json" "$HOME_DIR/reply.json"
cat > "$HOME_DIR/fake_provider.py" <<'PY'
import http.server, json, os, sys
home = sys.argv[1]
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _send(self, code, body):
        b = json.dumps(body).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_GET(self):
        if self.path.rstrip("/").endswith("/models"):
            self._send(200, {"object": "list", "data": [{"id": "fake-model", "object": "model"}]})
        else:
            self._send(404, {"error": "not found"})
    def do_POST(self):
        self.rfile.read(int(self.headers.get("Content-Length", 0)))
        content = open(os.path.join(home, "reply.json")).read()
        self._send(200, {"id": "x", "object": "chat.completion",
                         "choices": [{"index": 0, "finish_reason": "stop",
                                      "message": {"role": "assistant", "content": content}}],
                         "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}})
srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
open(os.path.join(home, "port"), "w").write(str(srv.server_address[1]))
srv.serve_forever()
PY
python3 "$HOME_DIR/fake_provider.py" "$HOME_DIR" &
PROVIDER_PID=$!
for _ in $(seq 1 50); do [[ -s "$HOME_DIR/port" ]] && break; sleep 0.1; done
[[ -s "$HOME_DIR/port" ]] || fail "fake provider did not start"
PORT="$(cat "$HOME_DIR/port")"

# --- plugin: crashes once (2.5 s after start, stderr `boom`), then runs normally ---------------
cat > "$HOME_DIR/flaky-plugin.sh" <<SH
#!/bin/sh
if [ -e '$HOME_DIR/crashed-once' ]; then
  exec '$FAKE_PLUGIN' --body-id body-fake --stderr-lines 'hello-from-plugin'
fi
touch '$HOME_DIR/crashed-once'
exec '$FAKE_PLUGIN' --body-id body-fake --stderr-lines 'hello-from-plugin|boom' --crash-after-ms 2500
SH
chmod +x "$HOME_DIR/flaky-plugin.sh"

cat > "$HOME_DIR/monoagens.toml" <<TOML
version = 1
data_dir = "$HOME_DIR/data"

[soul]
name = "MonoSmoke"
seed = 1

[model]
provider = "openai_compatible"
model = "fake-model"
daily_token_budget = 100000

[valve]
mode = "enforcing"
max_chain_wakes = 8

[plugins.fake]
kind = "adapter"
enabled = true
transport = "stdio"
command = "$HOME_DIR/flaky-plugin.sh"
args = []
TOML
cat > "$HOME_DIR/secrets.toml" <<TOML
[model]
api_key = "smoke-key-not-real"
endpoint = "http://127.0.0.1:$PORT/chat/completions"
TOML
chmod 600 "$HOME_DIR/secrets.toml"

# 1. starts and shows the composer
start_tui
wait_for "fake-model"
wait_for "enter send"
echo "ok: composer visible"

# 2. the plugin autostarts (green dot), crashes (red), and the supervisor restarts it (green)
wait_for_dot fake green 20
echo "ok: plugin autostarted (green dot)"
wait_for_dot fake red 15
echo "ok: crash turns the dot red"
wait_for_dot fake green 15
echo "ok: dot is green again after the restart"

# 3. /log shows the plugin stderr line and the crash reason
tmux send-keys -t "$SESSION" "/log" Enter
wait_for "Runtime log"
wait_for "plugin:fake"
wait_for "hello-from-plugin"
wait_for "boom"
echo "ok: /log shows the plugin stderr lines"
close_overlay "Runtime log"

# 4. a turn without chat.speak ends busy and shows the notice
cp "$HOME_DIR/silent.json" "$HOME_DIR/reply.json"
tmux send-keys -t "$SESSION" "say nothing please" Enter
wait_for "Mono chose not to reply to this message." 25
echo "ok: silent turn shows the notice"

# 5. a spoken reply arrives
cp "$HOME_DIR/speak.json" "$HOME_DIR/reply.json"
tmux send-keys -t "$SESSION" "say something" Enter
wait_for "pong smoke" 25
echo "ok: reply rendered"

# 6. quits cleanly
quit_tui
echo "ok: clean quit (exit 0)"

# 7. attach to `monoagens serve`, then detach: serve (and its plugin) keep running
SERVE_OUT="$HOME_DIR/serve.out"
env HOME="$HOME_DIR" XDG_CONFIG_HOME="$HOME_DIR" "$BIN" --config "$HOME_DIR/monoagens.toml" serve \
  >"$SERVE_OUT" 2>"$HOME_DIR/serve.err" &
SERVE_PID=$!
for _ in $(seq 1 100); do grep -q "monoagens serve running" "$SERVE_OUT" 2>/dev/null && break; sleep 0.1; done
grep -q "monoagens serve running" "$SERVE_OUT" || fail "serve did not start: $(cat "$HOME_DIR/serve.err")"
[[ -S "$HOME_DIR/data/gateway.sock" ]] || fail "serve did not serve the admin socket"
if env HOME="$HOME_DIR" XDG_CONFIG_HOME="$HOME_DIR" "$BIN" --config "$HOME_DIR/monoagens.toml" serve \
     >/dev/null 2>"$HOME_DIR/serve2.err"; then fail "a second serve must be refused"; fi
grep -q "already hosted by pid $SERVE_PID" "$HOME_DIR/serve2.err" || fail "second serve message: $(cat "$HOME_DIR/serve2.err")"
echo "ok: second serve refused, naming pid $SERVE_PID"

start_tui
wait_for "Attached to the gateway hosted by pid $SERVE_PID" 20
tmux send-keys -t "$SESSION" "hello through the socket" Enter
wait_for "pong smoke" 25
echo "ok: attached TUI chats through serve"
quit_tui
kill -0 "$SERVE_PID" 2>/dev/null || fail "serve died when the TUI detached"
[[ -S "$HOME_DIR/data/gateway.sock" ]] || fail "admin socket vanished after detach"
echo "ok: detach left serve running (pid $SERVE_PID)"

echo "tui-smoke: PASS"

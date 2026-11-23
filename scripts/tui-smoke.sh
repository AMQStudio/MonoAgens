#!/usr/bin/env bash
# TUI smoke test: drives the debug binary inside tmux. Home lives under target/tui-smoke/
# (never /tmp). No paid model calls: the provider is a tiny local fake HTTP server (python3) whose
# canned reply is switched by the script; the plugin is the gateway's `fake_plugin` example.
#
# Also (tui-settings-complete): the slash menu scrolls to the last command, settings lists show a
# scrollbar, Esc goes back one level, and a settings edit made through the Ctrl+K palette reaches the
# daemon's monoagens.toml (with a .bak) when the TUI is attached.
#
# Checks: composer, /model picker with search, plugin autostart dot (green), `/log` shows the plugin
# stderr line, crash -> red -> green after restart, silent-turn notice, a spoken reply, attach to
# `monoagens serve`, the governance panels (/status, /approvals, /events, /activity, /policy)
# against the attached daemon, and detach leaving serve running. Conversation checks
# (`tui-conversation`): live draft bubble from a delayed SSE stream, Esc Esc interrupt and F5 retry,
# Ctrl+O what-happened block, /why, Ctrl+R history search, /export.
#
# Usage: scripts/tui-smoke.sh   (after `cargo build -p monoagens-cli`; the fake plugin is built on
# demand with `cargo build --example fake_plugin -p monoagens-gateway`)
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
BIN="${MONOAGENS_BIN:-$TARGET/debug/monoagens}"
FAKE_PLUGIN="${FAKE_PLUGIN:-$TARGET/debug/examples/fake_plugin}"
SESSION="${SMOKE_SESSION:-wt-c-smoke}-$$"
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
  local cfg="${1:-$HOME_DIR/monoagens.toml}"
  rm -f "$HOME_DIR/exit.code"
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" \
    "env HOME='$HOME_DIR' XDG_CONFIG_HOME='$HOME_DIR' TERM=xterm-256color '$BIN' --config '$cfg'; echo \"exit=\$?\" > '$HOME_DIR/exit.code'; sleep 30"
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
import http.server, json, os, sys, time
home = sys.argv[1]
def knob(name, default=0.0):
    try:
        return float(open(os.path.join(home, name)).read().strip())
    except Exception:
        return default
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def _send(self, code, body):
        b = json.dumps(body).encode()
        self.send_response(code); self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(b))); self.end_headers(); self.wfile.write(b)
    def do_GET(self):
        if self.path.rstrip("/").endswith("/models"):
            self._send(200, {"object": "list", "data": [{"id": m, "object": "model"} for m in ("fake-model", "qwen-coder", "qwen-mini", "llama-big")]})
        else:
            self._send(404, {"error": "not found"})
    def do_POST(self):
        raw = self.rfile.read(int(self.headers.get("Content-Length", 0)))
        try:
            streaming = json.loads(raw).get("stream") is True
        except Exception:
            streaming = False
        open(os.path.join(home, "stream.log"), "a").write("stream\n" if streaming else "blocking\n")
        time.sleep(knob("delay"))  # the "slow model" for the interrupt check
        content = open(os.path.join(home, "reply.json")).read()
        if not streaming:
            self._send(200, {"id": "x", "object": "chat.completion",
                             "choices": [{"index": 0, "finish_reason": "stop",
                                          "message": {"role": "assistant", "content": content}}],
                             "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}})
            return
        self.send_response(200); self.send_header("Content-Type", "text/event-stream"); self.end_headers()
        step = int(knob("chunk", 12))
        for i in range(0, len(content), step):
            chunk = {"id": "x", "object": "chat.completion.chunk",
                     "choices": [{"index": 0, "delta": {"content": content[i:i + step]}, "finish_reason": None}]}
            self.wfile.write(("data: " + json.dumps(chunk) + "\n\n").encode()); self.wfile.flush()
            time.sleep(knob("stream_delay"))
        fin = {"id": "x", "object": "chat.completion.chunk", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}
        use = {"id": "x", "object": "chat.completion.chunk", "choices": [],
               "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}
        for c in (fin, use):
            self.wfile.write(("data: " + json.dumps(c) + "\n\n").encode())
        self.wfile.write(b"data: [DONE]\n\n"); self.wfile.flush()
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
if [ "\$1" = status ]; then exit 2; fi
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

# 5b. Ctrl+O expands the what-happened block of that turn; /why shows the valve explanation
tmux send-keys -t "$SESSION" C-o
wait_for "what happened"
wait_for "chat.speak"
echo "ok: Ctrl+O expands the what-happened block"
tmux send-keys -t "$SESSION" "/why" Enter
wait_for "decision event="
echo "ok: /why shows the valve explanation"
close_overlay "decision event="

# 5c. live draft: a slow SSE stream shows the draft bubble, then the real reply replaces it
python3 - "$HOME_DIR" <<'PY'
import json, sys
home = sys.argv[1]
d = json.load(open(home + "/speak.json"))
d["acts"][1]["payload"] = list("a draft that grows while the model writes".encode())
json.dump(d, open(home + "/long.json", "w"))
PY
cp "$HOME_DIR/long.json" "$HOME_DIR/reply.json"
echo 0.35 > "$HOME_DIR/stream_delay"
tmux send-keys -t "$SESSION" "stream me please" Enter
wait_for "not final" 20
echo "ok: draft bubble shown while the model writes"
wait_for_absent "not final" 40
wait_for "a draft that grows while the model writes" 10
echo "ok: the real reply replaced the draft"
echo 0 > "$HOME_DIR/stream_delay"
grep -q "^stream" "$HOME_DIR/stream.log" || fail "the provider was never asked to stream"

# 5d. interrupt: Esc Esc stops a slow turn, F5 retries it
cp "$HOME_DIR/speak.json" "$HOME_DIR/reply.json"
echo 10 > "$HOME_DIR/delay"
tmux send-keys -t "$SESSION" "a very slow question" Enter
wait_for "esc to cancel" 10
tmux send-keys -t "$SESSION" Escape
wait_for "Press Esc again to interrupt" 5
tmux send-keys -t "$SESSION" Escape
wait_for "Interrupted." 10
wait_for "esc to clear" 10
echo "ok: Esc Esc interrupted the slow turn"
echo 0 > "$HOME_DIR/delay"
tmux send-keys -t "$SESSION" F5
wait_for "pong smoke" 25
echo "ok: F5 retried the interrupted turn"

# 5e. Ctrl+R finds an older message; Esc restores the (empty) draft
tmux send-keys -t "$SESSION" C-r
tmux send-keys -t "$SESSION" "stream me"
wait_for "(reverse-i-search) \`stream me\`"
tmux send-keys -t "$SESSION" Escape
wait_for_absent "(reverse-i-search)" 5
echo "ok: Ctrl+R history search"

# 5f. /export writes the chat as Markdown under <data_dir>/exports/
tmux send-keys -t "$SESSION" "/export" Enter
wait_for "Exported to" 10
EXPORT_FILE="$(ls "$HOME_DIR"/data/exports/*.md | head -1)"
[[ -f "$EXPORT_FILE" ]] || fail "no export file under $HOME_DIR/data/exports"
grep -q "say something" "$EXPORT_FILE" || fail "export lacks the first message"
grep -q "a draft that grows while the model writes" "$EXPORT_FILE" || fail "export lacks a reply"
echo "ok: /export wrote $(basename "$EXPORT_FILE")"

# 6. quits cleanly
quit_tui
echo "ok: clean quit (exit 0)"

# 6b. /model opens the two-pane picker; search narrows the list (catalog provider, fake server)
cat > "$HOME_DIR/monoagens-catalog.toml" <<TOML
version = 1
data_dir = "$HOME_DIR/data"

[providers]
enabled = ["ollama"]

[model]
provider = "ollama"
model = "fake-model"

[model.provider_options]
base_url = "http://127.0.0.1:$PORT/v1"
TOML
start_tui "$HOME_DIR/monoagens-catalog.toml"
wait_for "enter send"
tmux send-keys -t "$SESSION" "/model" Enter
wait_for "Select model"
wait_for "qwen-coder" 15
wait_for "llama-big"
echo "ok: /model opens with the discovered models"
tmux send-keys -t "$SESSION" "qwen"
wait_for_absent "llama-big" 5
screen | grep -qF "qwen-mini" || fail "search removed the matching models"
echo "ok: search narrows the list"
tmux send-keys -t "$SESSION" Escape   # clears the search
sleep 0.3
close_overlay "Select model"
# /provider opens the provider list; /login is no longer a command
tmux send-keys -t "$SESSION" "/provider" Enter
wait_for "Providers" 10
wait_for "enabled" 10
echo "ok: /provider opens the provider list"
tmux send-keys -t "$SESSION" Escape
sleep 0.3
quit_tui

# 6c. scrolling slash menu, settings scrollbar, selector, nested Esc
start_tui
wait_for "enter send"
tmux send-keys -t "$SESSION" "/"
wait_for "Create a new local chat"
tmux send-keys -t "$SESSION" Up    # wraps to the last command and scrolls the menu to it
wait_for_absent "Create a new local chat" 5
screen | grep -qF "┃" || fail "slash menu scrollbar thumb missing"
screen | grep -qF "❯" || fail "slash menu selection marker missing"
echo "ok: slash menu reaches the last command (scrolled, scrollbar drawn)"
tmux send-keys -t "$SESSION" Escape
wait_for_absent "❯" 5
tmux resize-window -t "$SESSION" -x "$COLS" -y 26   # fewer rows than the Model tab has
sleep 0.5
tmux send-keys -t "$SESSION" "/settings" Enter
wait_for "Settings  ("
tmux send-keys -t "$SESSION" Right Right
wait_for "Budget & Headers"
screen | grep -qF "┃" || fail "settings scrollbar thumb missing"
echo "ok: settings list shows a scrollbar"
tmux send-keys -t "$SESSION" "output_limit_param"
sleep 0.3
tmux send-keys -t "$SESSION" Enter
wait_for "Type to search"
tmux send-keys -t "$SESSION" Escape      # 1: closes the selector
wait_for_absent "Type to search" 5
screen | grep -qF "Settings  (" || fail "Esc closed the whole overlay instead of the selector"
tmux send-keys -t "$SESSION" Escape      # 2: clears the search
sleep 0.3
screen | grep -qF "Settings  (" || fail "Esc closed the overlay instead of clearing the search"
tmux send-keys -t "$SESSION" Escape      # 3: closes the overlay
wait_for_absent "Settings  (" 5
echo "ok: Esc goes back one level (selector, search, overlay)"
quit_tui

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

# the draft preview also reaches an attached TUI (preview.subscribe over the admin socket)
cp "$HOME_DIR/long.json" "$HOME_DIR/reply.json"
echo 0.35 > "$HOME_DIR/stream_delay"
tmux send-keys -t "$SESSION" "stream through the socket" Enter
wait_for "not final" 20
wait_for_absent "not final" 40
echo 0 > "$HOME_DIR/stream_delay"
echo "ok: attached TUI shows the draft bubble over the socket"

# 7b. governance panels against the attached daemon: /status (mode serve, the daemon pid and its
# plugins), /approvals (nothing held), /events (the turns just recorded), /activity (no adapter
# traffic from the fake plugin), /policy and /people. A held action and an adapter row need
# seeded events and are covered by the gateway and TUI tests.
tmux send-keys -t "$SESSION" "/status" Enter
wait_for "owner pid"
wait_for "serve"
wait_for "$SERVE_PID"
wait_for "fake"
echo "ok: /status shows mode serve, pid $SERVE_PID and the plugin"
close_overlay "owner pid"
tmux send-keys -t "$SESSION" "/approvals" Enter
wait_for "No pending approvals."
echo "ok: /approvals opens (empty, no badge)"
close_overlay "No pending approvals."
tmux send-keys -t "$SESSION" "/events" Enter
wait_for "Events"
wait_for "percept"
echo "ok: /events lists the recorded turns"
close_overlay "Events  "
tmux send-keys -t "$SESSION" "/activity" Enter
wait_for "No adapter activity yet."
echo "ok: /activity opens read-only (empty)"
close_overlay "No adapter activity yet."
tmux send-keys -t "$SESSION" "/policy" Enter
wait_for "Cedar rules"
wait_for "default_cedar_rule_01"
echo "ok: /policy lists the Cedar rules"
close_overlay "Cedar rules"
# 7c. attached TUI: a settings edit found with the Ctrl+K palette goes through config.apply to the
# daemon (monoagens.toml changes, .bak written)
tmux send-keys -t "$SESSION" C-k
wait_for "Command palette"
tmux send-keys -t "$SESSION" "quiet_hours"
wait_for "quiet_hours"
tmux send-keys -t "$SESSION" Enter
wait_for "Settings  ("
tmux send-keys -t "$SESSION" Enter
wait_for "Saved valve.quiet_hours" 15
grep -q "quiet_hours = true" "$HOME_DIR/monoagens.toml" || fail "daemon config did not change: $(cat "$HOME_DIR/monoagens.toml")"
[[ -f "$HOME_DIR/monoagens.toml.bak" ]] || fail "no .bak next to the daemon config"
echo "ok: attached settings edit reached the daemon config (.bak written)"
tmux send-keys -t "$SESSION" Escape
wait_for_absent "Settings  (" 5
quit_tui
kill -0 "$SERVE_PID" 2>/dev/null || fail "serve died when the TUI detached"
[[ -S "$HOME_DIR/data/gateway.sock" ]] || fail "admin socket vanished after detach"
echo "ok: detach left serve running (pid $SERVE_PID)"

echo "tui-smoke: PASS"

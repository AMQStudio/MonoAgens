//! Fake stdio plugin speaking Waist-2 NDJSON, for supervision tests and the TUI smoke script.
//!
//! It answers `body/discover`, `body/heartbeat` and `body/shutdown`, and everything else with
//! a JSON-RPC "method not found". Behaviour is set by environment variables (a `--flag value`
//! argument of the same meaning overrides the variable, so tests need no process-wide env):
//!
//! | env                      | arg                | meaning                                        |
//! |--------------------------|--------------------|------------------------------------------------|
//! | `FAKE_STARTUP_MS`        | `--startup-ms`     | sleep before doing anything                    |
//! | `FAKE_STDERR_LINES`      | `--stderr-lines`   | lines written to stderr at startup, `\|`-split |
//! | `FAKE_CRASH_AFTER_MS`    | `--crash-after-ms` | exit this long after startup (default: never)  |
//! | `FAKE_EXIT_CODE`         | `--exit-code`      | exit code used for the crash (default 1)       |
//! | `FAKE_BODY_ID`           | `--body-id`        | body id answered in discover (default `fake`)  |

use std::io::{BufRead, Write};
use std::time::Duration;

use monoagens_protocol::{
    BodyDiscoverResult, BodyId, BodyShutdownResult, CURRENT_PROTOCOL_VERSION, PlaceId,
};
use serde_json::{Value, json};

fn setting(name: &str, arg: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == arg)
        && let Some(v) = args.get(pos + 1)
    {
        return Some(v.clone());
    }
    std::env::var(name).ok()
}

fn setting_u64(name: &str, arg: &str) -> Option<u64> {
    setting(name, arg).and_then(|v| v.trim().parse().ok())
}

fn main() {
    if let Some(ms) = setting_u64("FAKE_STARTUP_MS", "--startup-ms") {
        std::thread::sleep(Duration::from_millis(ms));
    }
    if let Some(lines) = setting("FAKE_STDERR_LINES", "--stderr-lines") {
        let mut err = std::io::stderr().lock();
        for l in lines.split('|') {
            let _ = writeln!(err, "{l}");
        }
        let _ = err.flush();
    }
    if let Some(ms) = setting_u64("FAKE_CRASH_AFTER_MS", "--crash-after-ms") {
        let code = i32::try_from(setting_u64("FAKE_EXIT_CODE", "--exit-code").unwrap_or(1))
            .unwrap_or(1);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            std::process::exit(code);
        });
    }
    let body_id = setting("FAKE_BODY_ID", "--body-id").unwrap_or_else(|| "fake".to_owned());

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let Ok(req) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let Some(method) = req.get("method").and_then(Value::as_str) else {
            continue; // a response to something we never asked
        };
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let (result, stop) = match method {
            "body/discover" => (
                Some(json!(BodyDiscoverResult {
                    body_id: BodyId::new(body_id.clone()),
                    protocol_version: CURRENT_PROTOCOL_VERSION,
                    capabilities: Vec::new(),
                    places: vec![PlaceId::new(format!("fake:{body_id}"))],
                })),
                false,
            ),
            "body/heartbeat" => (Some(json!({ "ok": true })), false),
            "body/shutdown" => (Some(json!(BodyShutdownResult { accepted: true })), true),
            _ => (None, false),
        };
        let resp = match result {
            Some(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            None => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }),
        };
        let _ = writeln!(stdout, "{resp}");
        let _ = stdout.flush();
        if stop {
            std::process::exit(0);
        }
    }
}

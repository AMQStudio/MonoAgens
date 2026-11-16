//! Deterministic local mock MCP server (`2026-07-28` default, `--legacy-2025` for `2025-11-25`)
//! speaking NDJSON JSON-RPC 2.0 over `stdio` for `body-mcp-bridge` tests.

#![forbid(unsafe_code)]

use std::env;
use std::io::{self, BufRead, Write};

use body_mcp_bridge::MockMcpServer;
use serde_json::Value;

fn main() {
    let legacy = env::args().any(|a| a == "--legacy-2025");
    let mut server = MockMcpServer::new(legacy);

    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    for line_res in stdin.lock().lines() {
        let Ok(line) = line_res else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if let Some(resp) = server.handle_request(&req) {
            for notif in server.drain_notifications() {
                let msg = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": notif.method,
                    "params": notif.params
                });
                if let Ok(wire) = serde_json::to_string(&msg) {
                    let _ = writeln!(stdout, "{wire}");
                }
            }
            if let Ok(wire) = serde_json::to_string(&resp) {
                let _ = writeln!(stdout, "{wire}");
            }
            let _ = stdout.flush();
        }
    }
}

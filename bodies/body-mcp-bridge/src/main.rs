//! Standalone `body-mcp-bridge` Waist-2 NDJSON `stdio` body binary.

#![forbid(unsafe_code)]

use std::env;
use std::io;

use body_mcp_bridge::{LoopbackMcpTransport, McpBridgeBody, McpBridgeConfig, StdioMcpTransport};
use monoagens_body_kit::{PersistentBodyRuntime, load_state_dir_from_env};
use monoagens_body_sdk::BodyHandler;

fn run_loop(mut runtime: PersistentBodyRuntime, handler: &mut dyn BodyHandler) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = runtime.run_stdio(stdin.lock(), stdout.lock(), handler) {
        eprintln!("body-mcp-bridge stdio error: {err}");
        std::process::exit(1);
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut server_id =
        env::var("MONOAGENS_MCP_SERVER_ID").unwrap_or_else(|_| "default".to_owned());
    let mut mcp_cmd = env::var("MONOAGENS_MCP_CMD").ok().filter(|s| !s.is_empty());
    let mut mcp_args: Vec<String> = env::var("MONOAGENS_MCP_ARGS")
        .ok()
        .map(|raw| raw.split_whitespace().map(ToOwned::to_owned).collect())
        .unwrap_or_default();
    let mut legacy_2025 = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--server-id" => {
                if let Some(val) = args.get(i + 1) {
                    server_id = val.clone();
                    i += 1;
                }
            }
            "--mcp-cmd" => {
                if let Some(val) = args.get(i + 1) {
                    mcp_cmd = Some(val.clone());
                    i += 1;
                }
            }
            "--mcp-arg" => {
                if let Some(val) = args.get(i + 1) {
                    mcp_args.push(val.clone());
                    i += 1;
                }
            }
            "--legacy-2025" => {
                legacy_2025 = true;
            }
            _ => {}
        }
        i += 1;
    }

    let config = McpBridgeConfig::new(server_id);
    let runtime = PersistentBodyRuntime::new(load_state_dir_from_env());

    if let Some(cmd) = mcp_cmd {
        let mut body = McpBridgeBody::connect(config, StdioMcpTransport::spawn(&cmd, &mcp_args));
        run_loop(runtime, &mut body);
    } else {
        let mut body = McpBridgeBody::connect(config, LoopbackMcpTransport::new(legacy_2025));
        run_loop(runtime, &mut body);
    }
}

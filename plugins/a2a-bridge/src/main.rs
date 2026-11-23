//! Standalone `plugin-a2a-bridge` Waist-2 NDJSON `stdio` body binary.

#![forbid(unsafe_code)]

use std::env;
use std::io;

use monoagens_body_kit::{PersistentBodyRuntime, load_state_dir_from_env};
use monoagens_body_sdk::BodyHandler;
use plugin_a2a_bridge::{
    A2aBridgeBody, A2aBridgeConfig, HttpLoopbackA2aTransport, LocalMockA2aPeer,
};

fn run_loop(mut runtime: PersistentBodyRuntime, handler: &mut dyn BodyHandler) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = runtime.run_stdio(stdin.lock(), stdout.lock(), handler) {
        eprintln!("plugin-a2a-bridge stdio error: {err}");
        std::process::exit(1);
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut agent_id = env::var("MONOAGENS_A2A_AGENT_ID").unwrap_or_else(|_| "peer".to_owned());
    let mut a2a_host_port = env::var("MONOAGENS_A2A_HOST")
        .ok()
        .filter(|s| !s.is_empty());

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--agent-id" => {
                if let Some(val) = args.get(i + 1) {
                    agent_id = val.clone();
                    i += 1;
                }
            }
            "--a2a-host" => {
                if let Some(val) = args.get(i + 1) {
                    a2a_host_port = Some(val.clone());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let config = A2aBridgeConfig::new(&agent_id);
    let runtime = PersistentBodyRuntime::new(load_state_dir_from_env());

    if let Some(host) = a2a_host_port {
        let mut body = A2aBridgeBody::connect(config, HttpLoopbackA2aTransport::new(host));
        run_loop(runtime, &mut body);
    } else {
        let peer = LocalMockA2aPeer::new(&agent_id, "http://127.0.0.1:0/rpc");
        let mut body = A2aBridgeBody::connect(config, peer);
        run_loop(runtime, &mut body);
    }
}

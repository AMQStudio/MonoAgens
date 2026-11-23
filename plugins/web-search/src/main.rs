//! Standalone `plugin-web-search` Waist-2 NDJSON `stdio` capability plugin binary.

#![forbid(unsafe_code)]

use std::io;

use monoagens_body_kit::PersistentBodyRuntime;
use plugin_web_search::{WebSearchBody, WebSearchConfig, resolve_state_dir_from_env};

fn main() {
    let config = match WebSearchConfig::load_or_init_from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("plugin-web-search config error: {err}");
            std::process::exit(1);
        }
    };

    let mut runtime = PersistentBodyRuntime::new(resolve_state_dir_from_env());
    let mut body = match WebSearchBody::with_live(config) {
        Ok(body) => body,
        Err(err) => {
            eprintln!("plugin-web-search init error: {err}");
            std::process::exit(1);
        }
    };

    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = runtime.run_stdio(stdin.lock(), stdout.lock(), &mut body) {
        eprintln!("plugin-web-search stdio error: {err}");
        std::process::exit(1);
    }
}

//! Standalone `plugin-workspace` Waist-2 (`monoagens.body.v1`) NDJSON `stdio` binary.

#![forbid(unsafe_code)]

use std::env;
use std::io;

use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{BodyId, PlaceId};
use plugin_workspace::{
    DEFAULT_BODY_ID, DEFAULT_PLACE_ID, WorkspaceBody, WorkspaceConfig, resolve_state_dir_from_env,
};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut body_id_override: Option<String> = None;
    let mut place_override: Option<String> = None;
    let mut root_override: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--body-id" => {
                if let Some(val) = args.get(i + 1) {
                    body_id_override = Some(val.clone());
                    i += 1;
                }
            }
            "--place" => {
                if let Some(val) = args.get(i + 1) {
                    place_override = Some(val.clone());
                    i += 1;
                }
            }
            "--root" => {
                if let Some(val) = args.get(i + 1) {
                    root_override = Some(val.clone());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let mut config = match WorkspaceConfig::load_or_init_from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("plugin-workspace config error: {err}");
            std::process::exit(1);
        }
    };
    if let Some(root) = root_override {
        config.root = Some(root);
    }

    let mut body = WorkspaceBody::with_config(config)
        .with_body_id(BodyId::new(body_id_override.as_deref().unwrap_or(DEFAULT_BODY_ID)))
        .with_place(PlaceId::new(place_override.as_deref().unwrap_or(DEFAULT_PLACE_ID)));

    let mut runtime = PersistentBodyRuntime::new(resolve_state_dir_from_env());
    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = runtime.run_stdio(stdin.lock(), stdout.lock(), &mut body) {
        eprintln!("plugin-workspace stdio error: {err}");
        std::process::exit(1);
    }
}

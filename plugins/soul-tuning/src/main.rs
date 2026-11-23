//! Standalone `plugin-soul-tuning` Waist-2 NDJSON `stdio` capability plugin binary.

#![forbid(unsafe_code)]

use std::env;
use std::io;

use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{BodyId, PlaceId};
use plugin_soul_tuning::{
    DEFAULT_BODY_ID, DEFAULT_PLACE, SoulTuningBody, SoulTuningConfig, resolve_state_dir_from_env,
};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut body_id = DEFAULT_BODY_ID.to_owned();
    let mut place = DEFAULT_PLACE.to_owned();

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--body-id" => {
                if let Some(val) = args.get(i + 1) {
                    body_id = val.clone();
                    i += 1;
                }
            }
            "--place" => {
                if let Some(val) = args.get(i + 1) {
                    place = val.clone();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let config = match SoulTuningConfig::load_or_init_from_env() {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("plugin-soul-tuning config error: {err}");
            std::process::exit(1);
        }
    };

    let state_dir = resolve_state_dir_from_env();
    let mut runtime = PersistentBodyRuntime::new(state_dir.clone());
    let mut body = SoulTuningBody::with_ids(
        BodyId::new(body_id),
        PlaceId::new(place),
        config,
        state_dir,
    );

    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = runtime.run_stdio(stdin.lock(), stdout.lock(), &mut body) {
        eprintln!("plugin-soul-tuning stdio error: {err}");
        std::process::exit(1);
    }
}

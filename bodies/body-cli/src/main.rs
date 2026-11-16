//! Standalone `body-cli` Waist-2 NDJSON `stdio` body binary.

#![forbid(unsafe_code)]

use std::env;
use std::io;

use body_cli::{
    CliBody, DEFAULT_BODY_ID, DEFAULT_BUBBLE_CAP, DEFAULT_OWNER_ACTOR, DEFAULT_TERMINAL_PLACE,
    StderrTerminalSink,
};
use monoagens_body_kit::{PersistentBodyRuntime, load_state_dir_from_env};
use monoagens_body_sdk::{ActorId, BodyId, PlaceId};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let mut actor = DEFAULT_OWNER_ACTOR.to_owned();
    let mut place = DEFAULT_TERMINAL_PLACE.to_owned();
    let mut bubble_cap = DEFAULT_BUBBLE_CAP;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--actor" => {
                if let Some(val) = args.get(i + 1) {
                    actor = val.clone();
                    i += 1;
                }
            }
            "--place" => {
                if let Some(val) = args.get(i + 1) {
                    place = val.clone();
                    i += 1;
                }
            }
            "--bubble-cap" => {
                if let Some(val) = args.get(i + 1)
                    && let Ok(parsed) = val.parse::<usize>()
                {
                    bubble_cap = parsed.max(1);
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let mut runtime = PersistentBodyRuntime::new(load_state_dir_from_env());
    let mut body = CliBody::new(
        BodyId::new(DEFAULT_BODY_ID),
        PlaceId::new(place),
        ActorId::new(actor),
        bubble_cap,
        StderrTerminalSink,
    );

    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = runtime.run_stdio(stdin.lock(), stdout.lock(), &mut body) {
        eprintln!("body-cli stdio error: {err}");
        std::process::exit(1);
    }
}

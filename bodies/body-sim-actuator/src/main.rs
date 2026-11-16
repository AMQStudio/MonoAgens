//! Standalone `body-sim-actuator` Waist-2 NDJSON `stdio` body binary.

#![forbid(unsafe_code)]

use std::io;

use body_sim_actuator::SimActuatorBody;
use monoagens_body_kit::{PersistentBodyRuntime, load_state_dir_from_env};

fn main() {
    let mut runtime = PersistentBodyRuntime::new(load_state_dir_from_env());
    let mut body = SimActuatorBody::default();
    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = runtime.run_stdio(stdin.lock(), stdout.lock(), &mut body) {
        eprintln!("body-sim-actuator stdio error: {err}");
        std::process::exit(1);
    }
}

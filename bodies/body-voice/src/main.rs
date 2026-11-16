//! Standalone `body-voice` Waist-2 NDJSON `stdio` body binary.

#![forbid(unsafe_code)]

use std::env;
use std::io;
use std::path::PathBuf;

use body_voice::VoiceBody;
use monoagens_body_kit::{PersistentBodyRuntime, load_state_dir_from_env};

fn main() {
    let silero_path = env::var_os("MONOAGENS_SILERO_VAD_PATH")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let mut runtime = PersistentBodyRuntime::new(load_state_dir_from_env());
    let mut body = VoiceBody::with_memory_sink(silero_path.as_deref());
    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = runtime.run_stdio(stdin.lock(), stdout.lock(), &mut body) {
        eprintln!("body-voice stdio error: {err}");
        std::process::exit(1);
    }
}

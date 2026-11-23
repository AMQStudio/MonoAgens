//! Standalone `plugin-pdf-reader` Waist-2 NDJSON `stdio` capability plugin binary.

#![forbid(unsafe_code)]

use std::io;

use monoagens_body_kit::PersistentBodyRuntime;
use plugin_pdf_reader::PdfReaderBody;

fn main() {
    let (mut body, state_dir) = match PdfReaderBody::from_env() {
        Ok(res) => res,
        Err(err) => {
            eprintln!("plugin-pdf-reader configuration error: {err}");
            std::process::exit(1);
        }
    };

    let mut runtime = PersistentBodyRuntime::new(state_dir);
    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(err) = runtime.run_stdio(stdin.lock(), stdout.lock(), &mut body) {
        eprintln!("plugin-pdf-reader stdio error: {err}");
        std::process::exit(1);
    }
}

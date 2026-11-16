//! Deterministic OpenRPC 1.4.1 and JSON Schema Draft 2020-12 generator binary.

#![forbid(unsafe_code)]

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let files = monoagens_protocol::schema::write_generated_artifacts(&out_dir)?;
    println!(
        "Generated {} Waist-2 schema artifacts under {}",
        files.len(),
        out_dir.display()
    );
    Ok(())
}

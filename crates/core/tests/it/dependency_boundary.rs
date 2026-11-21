use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

#[test]
fn monoagens_core_depends_only_on_protocol_and_serde() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_manifest = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("Cargo.toml");

    let cargo_bin = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(cargo_bin)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(&workspace_manifest)
        .output()
        .expect("run cargo metadata");

    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let metadata: Value =
        serde_json::from_slice(&output.stdout).expect("parse cargo metadata JSON");
    let packages = metadata["packages"]
        .as_array()
        .expect("metadata.packages must be an array");

    let core_pkg = packages
        .iter()
        .find(|pkg| pkg["name"].as_str() == Some("monoagens-core"))
        .expect("monoagens-core package not found in cargo metadata");

    let deps = core_pkg["dependencies"]
        .as_array()
        .expect("monoagens-core.dependencies must be an array");

    let allowed: BTreeSet<&str> = ["monoagens-protocol", "safetensors", "serde"]
        .into_iter()
        .collect();
    let mut forbidden = Vec::new();
    let mut observed = BTreeSet::new();

    for dep in deps {
        let kind = dep["kind"].as_str();
        // Only dev-dependencies are exempt from the runtime/build dependency boundary.
        if kind == Some("dev") {
            continue;
        }
        let name = dep["name"].as_str().expect("dependency name");
        observed.insert(name.to_owned());
        if !allowed.contains(name) {
            forbidden.push(format!("{name} (kind={kind:?})"));
        }
    }

    assert!(
        forbidden.is_empty(),
        "monoagens-core has forbidden non-dev dependencies: {forbidden:?} (allowed: {allowed:?})"
    );
    assert!(
        observed.contains("monoagens-protocol")
            && observed.contains("safetensors")
            && observed.contains("serde"),
        "expected monoagens-protocol, safetensors, and serde in monoagens-core dependencies, got: {observed:?}"
    );
}

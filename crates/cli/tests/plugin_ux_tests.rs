use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;
use tempfile::TempDir;

fn monoagens_bin() -> &'static str {
    env!("CARGO_BIN_EXE_monoagens")
}

fn write_commented_config(dir: &Path) -> std::path::PathBuf {
    let cfg_path = dir.join("monoagens.toml");
    let data_dir = dir.join("data");
    fs::create_dir_all(&data_dir).expect("mkdir data");
    let content = format!(
        "# Top-level operator comment\nversion = 1\ndata_dir = \"{}\"\n\n# Soul configuration\n[soul]\nname = \"MonoUxTest\"\nseed = 7\n\n[model]\nprovider = \"scripted\"\nmodel = \"scripted-test\"\ndaily_token_budget = 10000\n\n[valve]\nmode = \"enforcing\"\nmax_chain_wakes = 8\n",
        data_dir.display()
    );
    fs::write(&cfg_path, content).expect("write commented monoagens.toml");
    cfg_path
}

#[test]
fn cli_plugin_install_list_disable_enable_remove_preserves_comments_and_creates_backup() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = write_commented_config(tmp.path());
    let bak_path = tmp.path().join("monoagens.toml.bak");

    // 1. Install a custom plugin with explicit transport, kind, args, deps, and opt-deps.
    let install_custom = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "plugin",
            "install",
            "custom-tool",
            "--command",
            "/usr/bin/env",
            "--kind",
            "capability",
            "--arg",
            "python3",
            "--arg",
            "tool.py",
            "--dep",
            "rbac",
            "--opt-dep",
            "workspace",
        ])
        .output()
        .expect("run plugin install custom-tool");
    assert!(
        install_custom.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&install_custom.stderr)
    );
    assert!(bak_path.is_file(), "expected backup at {}", bak_path.display());

    let bak_raw = fs::read_to_string(&bak_path).expect("read .bak");
    assert!(bak_raw.contains("# Top-level operator comment"));
    assert!(!bak_raw.contains("[plugins.custom-tool]"));

    let after_custom = fs::read_to_string(&cfg_path).expect("read config after custom install");
    assert!(after_custom.contains("# Top-level operator comment"));
    assert!(after_custom.contains("# Soul configuration"));
    assert!(after_custom.contains("[plugins.custom-tool]"));

    // 2. Install multiple built-in presets in a single command.
    let install_builtins = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "plugin",
            "install",
            "telegram",
            "workspace",
        ])
        .output()
        .expect("run plugin install telegram workspace");
    assert!(
        install_builtins.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&install_builtins.stderr)
    );

    // 3. List plugins in JSON mode and inspect configured + catalog entries.
    let list_out = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "--json", "plugin", "list"])
        .output()
        .expect("run plugin list --json");
    assert!(
        list_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&list_out.stderr)
    );
    let list_json: Value = serde_json::from_slice(&list_out.stdout).expect("valid plugin list json");
    let configured = list_json["configured"].as_array().expect("configured array");
    assert_eq!(configured.len(), 3);

    let find_cfg = |id: &str| -> &Value {
        configured
            .iter()
            .find(|p| p["id"] == id)
            .unwrap_or_else(|| panic!("missing '{id}' in {configured:?}"))
    };
    let custom = find_cfg("custom-tool");
    assert_eq!(custom["kind"], "capability");
    assert_eq!(custom["enabled"], true);
    assert_eq!(custom["command"], "/usr/bin/env");
    assert_eq!(custom["args"], serde_json::json!(["python3", "tool.py"]));
    assert_eq!(custom["dependencies"], serde_json::json!(["rbac"]));
    assert_eq!(custom["optional_dependencies"], serde_json::json!(["workspace"]));

    let tg = find_cfg("telegram");
    assert_eq!(tg["kind"], "adapter");
    assert_eq!(tg["enabled"], true);
    assert_eq!(
        tg["args"],
        serde_json::json!(["--token-env", "TELEGRAM_BOT_TOKEN"])
    );
    assert!(
        tg["command"]
            .as_str()
            .is_some_and(|c| c.contains("body-telegram")),
        "expected body-telegram command, got {:?}",
        tg["command"]
    );

    let ws = find_cfg("workspace");
    assert_eq!(ws["kind"], "capability");
    assert_eq!(ws["optional_dependencies"], serde_json::json!(["rbac"]));

    let catalog = list_json["catalog"].as_array().expect("catalog array");
    assert!(catalog.len() >= 12);
    let tg_cat = catalog.iter().find(|c| c["id"] == "telegram").unwrap();
    assert_eq!(tg_cat["installed"], true);

    // 4. Disable, then re-enable `custom-tool`.
    let disable_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "plugin",
            "disable",
            "custom-tool",
        ])
        .output()
        .expect("run plugin disable");
    assert!(
        disable_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&disable_out.stderr)
    );
    let disabled_raw = fs::read_to_string(&cfg_path).expect("read config after disable");
    assert!(disabled_raw.contains("# Top-level operator comment"));
    assert!(disabled_raw.contains("enabled = false"));

    let enable_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "plugin",
            "enable",
            "custom-tool",
        ])
        .output()
        .expect("run plugin enable");
    assert!(enable_out.status.success());
    let enabled_list = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "--json", "plugin", "list"])
        .output()
        .expect("run plugin list after enable");
    let enabled_json: Value = serde_json::from_slice(&enabled_list.stdout).unwrap();
    let custom_after = enabled_json["configured"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "custom-tool")
        .unwrap();
    assert_eq!(custom_after["enabled"], true);

    // 5. Remove `custom-tool` and verify comments + sibling plugins remain intact.
    let remove_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "plugin",
            "remove",
            "custom-tool",
        ])
        .output()
        .expect("run plugin remove");
    assert!(remove_out.status.success());
    let final_raw = fs::read_to_string(&cfg_path).expect("read final config");
    assert!(final_raw.contains("# Top-level operator comment"));
    assert!(final_raw.contains("# Soul configuration"));
    assert!(!final_raw.contains("[plugins.custom-tool]"));
    assert!(final_raw.contains("[plugins.telegram]"));
    assert!(final_raw.contains("[plugins.workspace]"));

    // 6. Verify missing plugin errors cleanly on enable/disable/remove and unknown install.
    for args in [
        vec!["plugin", "enable", "does-not-exist"],
        vec!["plugin", "disable", "does-not-exist"],
        vec!["plugin", "remove", "does-not-exist"],
        vec!["plugin", "install", "unknown-custom-plugin"],
    ] {
        let mut full_args = vec!["--config", cfg_path.to_str().unwrap()];
        full_args.extend(args);
        let res = Command::new(monoagens_bin())
            .args(&full_args)
            .output()
            .expect("run failing plugin cmd");
        assert!(
            !res.status.success(),
            "expected failure for {full_args:?}"
        );
    }
}

#[test]
fn cli_plugin_install_bootstraps_default_config_when_missing() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("fresh/monoagens.toml");

    let out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "plugin",
            "install",
            "web-search",
        ])
        .output()
        .expect("install web-search into missing config");

    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(cfg_path.is_file());
    let raw = fs::read_to_string(&cfg_path).expect("read bootstrapped config");
    assert!(raw.contains("version = 1"));
    assert!(raw.contains("[plugins.web-search]"));
    assert!(raw.contains("kind = \"capability\""));
}

#[test]
fn cli_secret_set_list_unset_never_prints_values_and_enforces_0600_unix() {
    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = tmp.path().join("monoagens.toml");
    let secrets_path = tmp.path().join("secrets.toml");

    let secret_a = "sk-top-secret-value-alpha-111";
    let secret_b = "tg-env-secret-value-beta-222";
    let secret_c = "tavily-stdin-secret-gamma-333";

    // 1. Set top-level secret via `--value`.
    let set_a = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "secret",
            "set",
            "model_api_key",
            "--value",
            secret_a,
        ])
        .output()
        .expect("secret set --value");
    assert!(
        set_a.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&set_a.stderr)
    );

    // 2. Set `[env]` secret via `--from-env`.
    let set_b = Command::new(monoagens_bin())
        .env("SRC_TG_TOKEN", secret_b)
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "secret",
            "set",
            "TELEGRAM_BOT_TOKEN",
            "--from-env",
            "SRC_TG_TOKEN",
            "--env",
        ])
        .output()
        .expect("secret set --from-env --env");
    assert!(
        set_b.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&set_b.stderr)
    );

    // 3. Set `[env]` secret via `--stdin`.
    let mut child = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "secret",
            "set",
            "TAVILY_API_KEY",
            "--stdin",
            "--env",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn secret set --stdin");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{secret_c}").expect("write stdin secret");
    }
    let set_c = child.wait_with_output().expect("wait secret set --stdin");
    assert!(
        set_c.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&set_c.stderr)
    );

    // Verify 0600 permissions on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&secrets_path)
            .expect("secrets.toml metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "expected 0600 permissions on secrets.toml, got {mode:o}");
    }

    // 4. List in both human and JSON modes, asserting values NEVER appear.
    let list_plain = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "secret", "list"])
        .output()
        .expect("secret list");
    assert!(list_plain.status.success());

    let list_json_out = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "secret",
            "list",
        ])
        .output()
        .expect("secret list --json");
    assert!(list_json_out.status.success());

    for output in [&set_a, &set_b, &set_c, &list_plain, &list_json_out] {
        let out_s = String::from_utf8_lossy(&output.stdout);
        let err_s = String::from_utf8_lossy(&output.stderr);
        for val in [secret_a, secret_b, secret_c] {
            assert!(
                !out_s.contains(val) && !err_s.contains(val),
                "secret value '{val}' leaked in CLI output!\nstdout={out_s}\nstderr={err_s}"
            );
        }
    }

    let list_json: Value = serde_json::from_slice(&list_json_out.stdout).expect("secret list json");
    assert_eq!(list_json["keys"], serde_json::json!(["model_api_key"]));
    assert_eq!(
        list_json["env_keys"],
        serde_json::json!(["TELEGRAM_BOT_TOKEN", "TAVILY_API_KEY"])
    );

    // 5. Unset secrets and verify removal.
    let unset_c = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "secret",
            "unset",
            "TAVILY_API_KEY",
            "--env",
        ])
        .output()
        .expect("secret unset --env");
    assert!(unset_c.status.success());

    let unset_a = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "secret",
            "unset",
            "model_api_key",
        ])
        .output()
        .expect("secret unset top-level");
    assert!(unset_a.status.success());

    let list_after = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "--json",
            "secret",
            "list",
        ])
        .output()
        .expect("secret list after unset");
    let after_json: Value = serde_json::from_slice(&list_after.stdout).unwrap();
    assert_eq!(after_json["keys"], serde_json::json!([]));
    assert_eq!(
        after_json["env_keys"],
        serde_json::json!(["TELEGRAM_BOT_TOKEN"])
    );

    // Unsetting missing key fails cleanly.
    let unset_missing = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "secret",
            "unset",
            "nonexistent",
        ])
        .output()
        .expect("secret unset missing");
    assert!(!unset_missing.status.success());
}

#[cfg(unix)]
#[test]
fn cli_pair_invokes_configured_plugin_with_state_env_and_args() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = TempDir::new().expect("tempdir");
    let cfg_path = write_commented_config(tmp.path());
    let data_dir = tmp.path().join("data");
    let pair_log = tmp.path().join("pair_invocation.json");

    // Create an executable wrapper script simulating a pairing binary (e.g. body-whatsapp).
    let mock_bin = tmp.path().join("mock-pair-bin");
    let script_src = format!(
        r#"#!/usr/bin/env python3
import json
import os
import sys

record = {{
    "argv": sys.argv[1:],
    "plugin_config": os.environ.get("MONOAGENS_PLUGIN_CONFIG"),
    "plugin_state_dir": os.environ.get("MONOAGENS_PLUGIN_STATE_DIR"),
    "body_state_dir": os.environ.get("MONOAGENS_BODY_STATE_DIR"),
    "pair_env_secret": os.environ.get("PAIR_TEST_SECRET"),
    "state_dir_exists": os.path.isdir(os.environ.get("MONOAGENS_PLUGIN_STATE_DIR", "")),
}}
with open({log_path:?}, "w", encoding="utf-8") as f:
    json.dump(record, f)

print("MOCK-QR-RENDERED-OK")
if "--exit-code" in sys.argv:
    idx = sys.argv.index("--exit-code")
    sys.exit(int(sys.argv[idx + 1]))
"#,
        log_path = pair_log.display().to_string(),
    );
    fs::write(&mock_bin, script_src).expect("write mock-pair-bin");
    fs::set_permissions(&mock_bin, fs::Permissions::from_mode(0o755))
        .expect("chmod +x mock-pair-bin");

    // Install the mock plugin into monoagens.toml and add a [env] secret.
    let install = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "plugin",
            "install",
            "mock-wa",
            "--command",
            mock_bin.to_str().unwrap(),
        ])
        .output()
        .expect("install mock-wa");
    assert!(install.status.success());

    let set_secret = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "secret",
            "set",
            "PAIR_TEST_SECRET",
            "--value",
            "pair-secret-val",
            "--env",
        ])
        .output()
        .expect("set PAIR_TEST_SECRET");
    assert!(set_secret.status.success());

    // 1. Run `monoagens pair mock-wa --phone +15551234567`.
    let pair_out = Command::new(monoagens_bin())
        .env_remove("PAIR_TEST_SECRET")
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "pair",
            "mock-wa",
            "--phone",
            "+15551234567",
        ])
        .output()
        .expect("run monoagens pair");
    assert!(
        pair_out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&pair_out.stderr)
    );
    let stdout = String::from_utf8_lossy(&pair_out.stdout);
    assert!(
        stdout.contains("MOCK-QR-RENDERED-OK"),
        "expected inherited stdout from mock binary, got: {stdout}"
    );

    let expected_state_dir = data_dir.join("plugins").join("mock-wa");
    let expected_db = expected_state_dir.join("session.db");
    let expected_cfg = tmp.path().join("plugins").join("mock-wa.toml");

    let recorded: Value =
        serde_json::from_str(&fs::read_to_string(&pair_log).expect("read pair log"))
            .expect("valid pair log json");
    assert_eq!(
        recorded["argv"],
        serde_json::json!([
            "pair",
            "--db",
            expected_db.display().to_string(),
            "--phone",
            "+15551234567"
        ])
    );
    assert_eq!(
        recorded["plugin_config"],
        serde_json::json!(expected_cfg.display().to_string())
    );
    assert_eq!(
        recorded["plugin_state_dir"],
        serde_json::json!(expected_state_dir.display().to_string())
    );
    assert_eq!(
        recorded["body_state_dir"],
        serde_json::json!(expected_state_dir.display().to_string())
    );
    assert_eq!(recorded["pair_env_secret"], serde_json::json!("pair-secret-val"));
    assert_eq!(recorded["state_dir_exists"], serde_json::json!(true));

    // 2. Verify `--db` override and non-zero exit code propagation.
    let custom_db = tmp.path().join("custom.db");
    let pair_custom = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "pair",
            "mock-wa",
            "--db",
            custom_db.to_str().unwrap(),
            "--exit-code",
            "7",
        ])
        .output()
        .expect("run monoagens pair with --db and --exit-code");
    assert_eq!(pair_custom.status.code(), Some(7));

    let recorded2: Value =
        serde_json::from_str(&fs::read_to_string(&pair_log).expect("read pair log 2"))
            .expect("valid pair log json 2");
    assert_eq!(
        recorded2["argv"],
        serde_json::json!([
            "pair",
            "--db",
            custom_db.display().to_string(),
            "--exit-code",
            "7"
        ])
    );

    // 3. Unknown plugin and unix_socket plugin fail cleanly on `pair`.
    let pair_unknown = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "pair", "unknown-plugin"])
        .output()
        .expect("pair unknown plugin");
    assert!(!pair_unknown.status.success());

    let sock_path = tmp.path().join("mock.sock");
    let install_sock = Command::new(monoagens_bin())
        .args([
            "--config",
            cfg_path.to_str().unwrap(),
            "plugin",
            "install",
            "sock-plugin",
            "--socket",
            sock_path.to_str().unwrap(),
        ])
        .output()
        .expect("install unix_socket plugin");
    assert!(install_sock.status.success());

    let pair_sock = Command::new(monoagens_bin())
        .args(["--config", cfg_path.to_str().unwrap(), "pair", "sock-plugin"])
        .output()
        .expect("pair unix_socket plugin");
    assert!(!pair_sock.status.success());
}

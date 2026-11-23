//! Integration tests for `plugin-soul-tuning`.

#![forbid(unsafe_code)]

use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    ActionExecuteParams, ActionExecuteResult, ActionId, ActorId, Audience, BodyDiscoverParams,
    BodyDiscoverResult, BodyHandler, CapabilityId, OpaquePayload, PlaceId, ReceiptOutcome, RpcId,
    Waist2ErrorCode, Waist2Headers, Waist2Method, Waist2Request, Waist2Response,
};
use plugin_soul_tuning::{
    ACTIVE_MODE_FILENAME, CAP_GET_MODE, CAP_LIST_MODES, CAP_SET_MODE, DEFAULT_PLACE,
    ENV_PLUGIN_CONFIG, ENV_PLUGIN_STATE_DIR, SoulTuningBody, SoulTuningConfig,
};
use serde_json::{Value, json};
use tempfile::tempdir;

fn exec_params(
    action_id: &str,
    cap: &str,
    place: &str,
    payload: &[u8],
) -> ActionExecuteParams {
    ActionExecuteParams {
        headers: Waist2Headers::new(format!("key-{action_id}")),
        action_id: ActionId::new(action_id),
        capability_id: CapabilityId::new(cap),
        places: vec![PlaceId::new(place)],
        audience: Audience::new([ActorId::new("per_owner")]),
        payload: OpaquePayload::from_bytes(payload),
        timeout_ms: Some(5_000),
        h_act: None,
    }
}

#[test]
fn discover_returns_sorted_capabilities_with_hashes_and_identity_changing_flag() {
    let mut body = SoulTuningBody::new(SoulTuningConfig::default(), None);
    let discover = body
        .discover(BodyDiscoverParams {
            headers: Waist2Headers::new("disc-1"),
            body_id: None,
        })
        .expect("discover succeeds");

    assert_eq!(discover.body_id.as_str(), "plugin-soul-tuning");
    assert_eq!(discover.places, vec![PlaceId::new(DEFAULT_PLACE)]);
    assert_eq!(discover.capabilities.len(), 3);

    let names: Vec<&str> = discover
        .capabilities
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    assert_eq!(names, vec![CAP_GET_MODE, CAP_LIST_MODES, CAP_SET_MODE]);

    for cap in &discover.capabilities {
        assert!(
            cap.capability_schema_hash.as_str().starts_with("sha256:"),
            "missing schema hash for {}",
            cap.id
        );
        assert_eq!(cap.places, vec![PlaceId::new(DEFAULT_PLACE)]);
    }

    assert!(!discover.capabilities[0].flags.identity_changing);
    assert!(!discover.capabilities[1].flags.identity_changing);
    assert!(discover.capabilities[2].flags.identity_changing);
}

#[test]
fn set_mode_switches_and_persists_across_restart() {
    let dir = tempdir().expect("tempdir");
    let state_dir = dir.path().to_path_buf();

    {
        let mut body = SoulTuningBody::new(SoulTuningConfig::default(), Some(state_dir.clone()));
        assert_eq!(body.active_mode(), "daily");

        // list_modes before switch
        let list_res = body
            .execute(exec_params("act-list", CAP_LIST_MODES, DEFAULT_PLACE, b"{}"))
            .expect("list_modes succeeds");
        assert!(list_res.accepted);
        let list_detail: Value = serde_json::from_str(
            list_res
                .receipt
                .expect("receipt")
                .detail
                .as_deref()
                .expect("detail"),
        )
        .expect("json");
        assert_eq!(list_detail["active_mode"], "daily");
        assert_eq!(list_detail["modes"].as_array().expect("array").len(), 2);

        // set_mode daily -> professional
        let set_res = body
            .execute(exec_params(
                "act-set",
                CAP_SET_MODE,
                DEFAULT_PLACE,
                br#"{"mode":"professional"}"#,
            ))
            .expect("set_mode succeeds");
        assert!(set_res.accepted);
        let receipt = set_res.receipt.expect("receipt");
        assert_eq!(receipt.outcome, ReceiptOutcome::Done);
        let detail: Value =
            serde_json::from_str(receipt.detail.as_deref().expect("detail")).expect("json");
        assert_eq!(detail["previous_mode"], "daily");
        assert_eq!(detail["active_mode"], "professional");
        assert!((detail["talkativeness_offset"].as_f64().expect("f64") - (-0.05)).abs() < 1e-6);
    }

    assert!(state_dir.join(ACTIVE_MODE_FILENAME).exists());

    // Simulate process restart with same state_dir
    let mut restarted = SoulTuningBody::new(SoulTuningConfig::default(), Some(state_dir));
    assert_eq!(restarted.active_mode(), "professional");

    let get_res = restarted
        .execute(exec_params("act-get", CAP_GET_MODE, DEFAULT_PLACE, b"{}"))
        .expect("get_mode succeeds");
    assert!(get_res.accepted);
    let get_detail: Value = serde_json::from_str(
        get_res
            .receipt
            .expect("receipt")
            .detail
            .as_deref()
            .expect("detail"),
    )
    .expect("json");
    assert_eq!(get_detail["active_mode"], "professional");
    assert_eq!(
        get_detail["available_modes"],
        json!(["daily", "professional"])
    );
}

#[test]
fn unknown_mode_is_refused_without_mutating_active_mode() {
    let dir = tempdir().expect("tempdir");
    let state_dir = dir.path().to_path_buf();
    let mut body = SoulTuningBody::new(SoulTuningConfig::default(), Some(state_dir.clone()));

    let res = body
        .execute(exec_params(
            "act-bad-mode",
            CAP_SET_MODE,
            DEFAULT_PLACE,
            br#"{"mode":"nonexistent"}"#,
        ))
        .expect("returns refused result");
    assert!(!res.accepted);
    let receipt = res.receipt.expect("receipt");
    assert_eq!(receipt.outcome, ReceiptOutcome::Refused);
    assert_eq!(receipt.reason_code.as_deref(), Some("ERR_UNKNOWN_MODE"));
    assert_eq!(body.active_mode(), "daily");
    assert!(!state_dir.join(ACTIVE_MODE_FILENAME).exists());
}

#[test]
fn unknown_capability_returns_err_unknown_capability() {
    let mut body = SoulTuningBody::new(SoulTuningConfig::default(), None);
    let err = body
        .execute(exec_params(
            "act-unknown-cap",
            "soul_tuning.nonexistent",
            DEFAULT_PLACE,
            b"{}",
        ))
        .unwrap_err();
    assert_eq!(err.code, Waist2ErrorCode::UnknownCapability.code());
}

#[test]
fn custom_modes_and_clamped_offsets_load_via_monoagens_plugin_config() {
    let dir = tempdir().expect("tempdir");
    let cfg_path = dir.path().join("nested").join("soul-tuning.toml");
    let state_dir = dir.path().join("state");

    // 1. Missing file -> auto-creates commented soul-tuning.toml and returns defaults.
    let initial = SoulTuningConfig::load_or_init_from_path(&cfg_path).expect("auto-init succeeds");
    assert_eq!(initial.default_mode, "daily");
    assert!(cfg_path.exists());
    let generated_text = fs::read_to_string(&cfg_path).expect("read generated toml");
    assert!(generated_text.contains("talkativeness_offset"));

    // 2. Write custom mode with out-of-range offset -> clamped to [-0.2, 0.2] on reload via binary.
    let custom_toml = r#"
version = 1
default_mode = "daily"

[modes.terse]
rule = "Be extremely brief."
talkativeness_offset = -0.9
examples = [["person", "hi"], ["you", "Hi."]]

[modes.chatty]
rule = "Be very warm and expansive."
talkativeness_offset = 1.5
examples = [["person", "hi"], ["you", "Hey there! Wonderful to hear from you."]]
"#;
    fs::write(&cfg_path, custom_toml).expect("write custom toml");

    let reloaded = SoulTuningConfig::load_or_init_from_path(&cfg_path).expect("reload succeeds");
    assert!((reloaded.modes["terse"].talkativeness_offset - (-0.2)).abs() < 1e-6);
    assert!((reloaded.modes["chatty"].talkativeness_offset - 0.2).abs() < 1e-6);
    assert!(reloaded.modes.contains_key("daily"));
    assert!(reloaded.modes.contains_key("professional"));

    // 3. Verify subprocess honors MONOAGENS_PLUGIN_CONFIG & MONOAGENS_PLUGIN_STATE_DIR.
    let auto_cfg_path = dir.path().join("auto").join("soul-tuning.toml");
    let mut child = Command::new(env!("CARGO_BIN_EXE_plugin-soul-tuning"))
        .env(ENV_PLUGIN_CONFIG, &auto_cfg_path)
        .env(ENV_PLUGIN_STATE_DIR, &state_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn plugin-soul-tuning");

    let req_line = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::ActionExecute,
        exec_params(
            "act-sub-set",
            CAP_SET_MODE,
            DEFAULT_PLACE,
            br#"{"mode":"professional"}"#,
        ),
    ))
    .expect("serialize");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{req_line}").expect("write stdin");
    }
    let output = child.wait_with_output().expect("wait output");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(auto_cfg_path.exists());
    assert!(state_dir.join(ACTIVE_MODE_FILENAME).exists());

    // 4. Reload subprocess with custom config path and verify persisted mode + clamped custom mode.
    let mut child2 = Command::new(env!("CARGO_BIN_EXE_plugin-soul-tuning"))
        .env(ENV_PLUGIN_CONFIG, &cfg_path)
        .env(ENV_PLUGIN_STATE_DIR, &state_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn plugin-soul-tuning #2");
    let list_line = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(2)),
        Waist2Method::ActionExecute,
        exec_params("act-sub-list", CAP_LIST_MODES, DEFAULT_PLACE, b"{}"),
    ))
    .expect("serialize");
    {
        let stdin = child2.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{list_line}").expect("write stdin");
    }
    let out2 = child2.wait_with_output().expect("wait output #2");
    assert!(out2.status.success());
    let stdout_str = String::from_utf8_lossy(&out2.stdout);
    let resp: Waist2Response<ActionExecuteResult> =
        serde_json::from_str(stdout_str.trim()).expect("parse ndjson response");
    let detail: Value = serde_json::from_str(
        resp.result
            .expect("res")
            .receipt
            .expect("rcpt")
            .detail
            .as_deref()
            .expect("detail"),
    )
    .expect("detail json");
    assert_eq!(detail["active_mode"], "professional");
    let modes = detail["modes"].as_array().expect("modes array");
    let terse = modes
        .iter()
        .find(|m| m["name"] == "terse")
        .expect("terse mode present");
    assert!((terse["talkativeness_offset"].as_f64().expect("f64") - (-0.2)).abs() < 1e-6);

    // 5. Unset env var inside subprocess -> succeeds without writing files.
    let missing_path = dir.path().join("should-not-exist.toml");
    let mut child3 = Command::new(env!("CARGO_BIN_EXE_plugin-soul-tuning"))
        .env_remove(ENV_PLUGIN_CONFIG)
        .env_remove(ENV_PLUGIN_STATE_DIR)
        .env_remove("MONOAGENS_BODY_STATE_DIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn plugin-soul-tuning #3");
    let get_line = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(3)),
        Waist2Method::ActionExecute,
        exec_params("act-sub-get", CAP_GET_MODE, DEFAULT_PLACE, b"{}"),
    ))
    .expect("serialize");
    {
        let stdin = child3.stdin.as_mut().expect("stdin");
        writeln!(stdin, "{get_line}").expect("write stdin");
    }
    let out3 = child3.wait_with_output().expect("wait output #3");
    assert!(out3.status.success());
    assert!(!missing_path.exists());
}

#[test]
fn runtime_ndjson_dispatch_roundtrip() {
    let dir = tempdir().expect("tempdir");
    let mut rt = PersistentBodyRuntime::new(Some(dir.path().to_path_buf()));
    let mut body = SoulTuningBody::new(SoulTuningConfig::default(), Some(dir.path().to_path_buf()));

    let disc_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(1)),
        Waist2Method::BodyDiscover,
        BodyDiscoverParams {
            headers: Waist2Headers::new("k-disc"),
            body_id: None,
        },
    ))
    .expect("serialize");
    let disc_line = rt.dispatch_line(&disc_req, &mut body).expect("response");
    let disc_resp: Waist2Response<BodyDiscoverResult> =
        serde_json::from_str(&disc_line).expect("parse");
    assert_eq!(disc_resp.result.expect("res").capabilities.len(), 3);

    let set_req = serde_json::to_string(&Waist2Request::new(
        Some(RpcId::Int(2)),
        Waist2Method::ActionExecute,
        exec_params(
            "act-rt-set",
            CAP_SET_MODE,
            DEFAULT_PLACE,
            br#"{"mode":"professional"}"#,
        ),
    ))
    .expect("serialize");
    let set_line = rt.dispatch_line(&set_req, &mut body).expect("response");
    let set_resp: Waist2Response<ActionExecuteResult> =
        serde_json::from_str(&set_line).expect("parse");
    assert!(set_resp.result.expect("res").accepted);
}

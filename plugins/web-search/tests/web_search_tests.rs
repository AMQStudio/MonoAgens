#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr};
use std::process::{Command, Stdio};

use monoagens_body_kit::PersistentBodyRuntime;
use monoagens_body_sdk::{
    ActionExecuteParams, ActionExecuteResult, ActionId, ActorId, ApprovalResolveParams,
    ApprovalResolveResult, Audience, BodyDiscoverParams, BodyDiscoverResult, BodyId,
    CURRENT_PROTOCOL_VERSION, CapabilityId, HAct, OpaquePayload, PlaceId, ReceiptOutcome,
    StateQueryParams, StateQueryResult, StateQueryStatus, Waist2ErrorCode, Waist2Headers,
    Waist2Method, Waist2Request, Waist2Response, sort_capabilities,
};
use serde_json::json;
use tempfile::TempDir;

use plugin_web_search::config::{
    ENV_BRAVE_API_KEY, ENV_PLUGIN_CONFIG, ENV_PLUGIN_STATE_DIR, ENV_TAVILY_API_KEY, MemoryEnv,
    WebSearchConfig,
};
use plugin_web_search::fetch::FetchOutput;
use plugin_web_search::providers::SearchOutput;
use plugin_web_search::ssrf::MockDnsResolver;
use plugin_web_search::transport::{HttpResponse, MockWebTransport};
use plugin_web_search::{DEFAULT_BODY_ID, DEFAULT_PLACE, WebSearchBody};

fn make_test_body(
    config: WebSearchConfig,
    transport: MockWebTransport,
    resolver: MockDnsResolver,
    env: MemoryEnv,
) -> WebSearchBody<MockWebTransport, MockDnsResolver, MemoryEnv> {
    WebSearchBody::new(
        BodyId::new(DEFAULT_BODY_ID),
        PlaceId::new(DEFAULT_PLACE),
        config,
        transport,
        resolver,
        env,
    )
}

fn dispatch_rpc<P: serde::Serialize, R: serde::de::DeserializeOwned>(
    runtime: &mut PersistentBodyRuntime,
    handler: &mut dyn monoagens_body_sdk::BodyHandler,
    id: i64,
    method: Waist2Method,
    params: P,
) -> Waist2Response<R> {
    let req = Waist2Request::new(Some(monoagens_body_sdk::RpcId::Int(id)), method, params);
    let line = serde_json::to_string(&req).unwrap();
    let out = runtime.dispatch_line(&line, handler).unwrap();
    serde_json::from_str(&out).unwrap()
}

fn exec_params(
    key: &str,
    action_id: &str,
    capability_id: &str,
    payload: &[u8],
) -> ActionExecuteParams {
    ActionExecuteParams {
        headers: Waist2Headers::new(key),
        action_id: ActionId::new(action_id),
        capability_id: CapabilityId::new(capability_id),
        places: vec![PlaceId::new(DEFAULT_PLACE)],
        audience: Audience::new([ActorId::new("per_owner")]),
        payload: OpaquePayload::from_bytes(payload),
        timeout_ms: Some(5_000),
        h_act: None,
    }
}

#[test]
fn ssrf_blocks_private_loopback_metadata_cgnat_ipv6_ports_and_schemes_without_http_calls() {
    let transport = MockWebTransport::new();
    let resolver = MockDnsResolver::new();
    let env = MemoryEnv::new();
    let mut body = make_test_body(WebSearchConfig::default(), transport, resolver, env);
    let mut runtime = PersistentBodyRuntime::new(None);

    let blocked_targets = [
        "http://127.0.0.1/",
        "http://169.254.169.254/latest/meta-data/",
        "http://10.0.0.1/internal",
        "http://192.168.1.1/router",
        "http://100.64.0.1/cgnat",
        "http://[::1]/",
        "http://[::ffff:127.0.0.1]/",
        "http://93.184.216.34:22/",
        "file:///etc/passwd",
    ];

    for (idx, url) in blocked_targets.iter().enumerate() {
        let payload = json!({ "url": url }).to_string();
        let resp: Waist2Response<ActionExecuteResult> = dispatch_rpc(
            &mut runtime,
            &mut body,
            idx as i64 + 1,
            Waist2Method::ActionExecute,
            exec_params(
                &format!("ssrf-{idx}"),
                &format!("act-ssrf-{idx}"),
                "web.fetch",
                payload.as_bytes(),
            ),
        );
        assert!(resp.error.is_none(), "unexpected RPC error for {url}");
        let res = resp.result.unwrap();
        assert!(!res.accepted, "expected accepted=false for {url}");
        let rcpt = res.receipt.unwrap();
        assert_eq!(rcpt.outcome, ReceiptOutcome::Refused);
        assert_eq!(rcpt.reason_code.as_deref(), Some("ERR_SSRF_BLOCKED"));
    }

    assert!(
        body.transport().recorded_requests().is_empty(),
        "SSRF-blocked URLs must never issue outbound HTTP requests"
    );
}

#[test]
fn redirect_to_ssrf_target_is_refused_and_bounded_redirects_work() {
    let transport = MockWebTransport::new();
    transport.push_route(
        "https://public.example.com/evil-redirect",
        Ok(HttpResponse {
            status: 302,
            headers: BTreeMap::from([(
                "location".to_owned(),
                "http://169.254.169.254/latest/meta-data/".to_owned(),
            )]),
            body: Vec::new(),
        }),
    );
    transport.push_route(
        "https://public.example.com/start",
        Ok(HttpResponse {
            status: 301,
            headers: BTreeMap::from([("location".to_owned(), "/final".to_owned())]),
            body: Vec::new(),
        }),
    );
    transport.push_route(
        "https://public.example.com/final",
        Ok(HttpResponse {
            status: 200,
            headers: BTreeMap::from([("content-type".to_owned(), "text/html; charset=utf-8".to_owned())]),
            body: b"<html><head><title>Final Page</title></head><body><script>bad()</script><style>h1{}</style><p>Hello &amp; Welcome</p></body></html>".to_vec(),
        }),
    );

    let mut resolver = MockDnsResolver::new();
    resolver.insert(
        "public.example.com",
        vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))],
    );
    let mut body = make_test_body(
        WebSearchConfig::default(),
        transport,
        resolver,
        MemoryEnv::new(),
    );
    let mut runtime = PersistentBodyRuntime::new(None);

    // 1. Redirect targeting metadata IP must be refused before second request.
    let resp: Waist2Response<ActionExecuteResult> = dispatch_rpc(
        &mut runtime,
        &mut body,
        1,
        Waist2Method::ActionExecute,
        exec_params(
            "redir-ssrf",
            "act-redir-ssrf",
            "web.fetch",
            br#"{"url":"https://public.example.com/evil-redirect"}"#,
        ),
    );
    let res = resp.result.unwrap();
    assert!(!res.accepted);
    let rcpt = res.receipt.unwrap();
    assert_eq!(rcpt.outcome, ReceiptOutcome::Refused);
    assert_eq!(rcpt.reason_code.as_deref(), Some("ERR_SSRF_BLOCKED"));
    assert_eq!(body.transport().recorded_requests().len(), 1);

    // 2. Valid public relative redirect completes and extracts clean HTML text.
    let resp2: Waist2Response<ActionExecuteResult> = dispatch_rpc(
        &mut runtime,
        &mut body,
        2,
        Waist2Method::ActionExecute,
        exec_params(
            "redir-ok",
            "act-redir-ok",
            "web.fetch",
            br#"{"url":"https://public.example.com/start","max_chars":200}"#,
        ),
    );
    let res2 = resp2.result.unwrap();
    assert!(res2.accepted);
    let rcpt2 = res2.receipt.unwrap();
    assert_eq!(rcpt2.outcome, ReceiptOutcome::Done);
    let output: FetchOutput = serde_json::from_str(rcpt2.detail.as_deref().unwrap()).unwrap();
    assert_eq!(output.url, "https://public.example.com/final");
    assert_eq!(output.status, 200);
    assert_eq!(output.title, "Final Page");
    assert_eq!(output.text, "Hello & Welcome");
    assert!(!output.truncated);
}

#[test]
fn provider_chain_falls_back_through_searxng_tavily_brave_to_duckduckgo() {
    let transport = MockWebTransport::new();
    // Tavily fails with 500; Brave has no key; DuckDuckGo returns HTML lite results.
    transport.push_route(
        "https://api.tavily.com/search",
        Ok(HttpResponse {
            status: 500,
            headers: BTreeMap::new(),
            body: b"internal error".to_vec(),
        }),
    );
    transport.push_route(
        "https://html.duckduckgo.com/html/",
        Ok(HttpResponse {
            status: 200,
            headers: BTreeMap::from([("content-type".to_owned(), "text/html".to_owned())]),
            body: br#"
            <html><body>
              <div class="result">
                <a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Frust-lang.org%2F">Rust &amp; Programming</a>
                <a class="result__snippet">Systems <b>language</b> for safety.</a>
              </div>
              <div class="result">
                <a class="result__a" href="https://docs.rs/">Docs.rs</a>
                <a class="result__snippet">Rust crate documentation.</a>
              </div>
            </body></html>"#
                .to_vec(),
        }),
    );

    let mut env = MemoryEnv::new();
    env.insert(ENV_TAVILY_API_KEY, "tvly-secret-do-not-leak");

    let mut body = make_test_body(
        WebSearchConfig::default(),
        transport,
        MockDnsResolver::new(),
        env,
    );
    let mut runtime = PersistentBodyRuntime::new(None);

    let resp: Waist2Response<ActionExecuteResult> = dispatch_rpc(
        &mut runtime,
        &mut body,
        1,
        Waist2Method::ActionExecute,
        exec_params(
            "search-fallback",
            "act-search-1",
            "web.search",
            br#"{"query":"rust programming","max_results":2}"#,
        ),
    );

    let res = resp.result.unwrap();
    assert!(res.accepted);
    let rcpt = res.receipt.unwrap();
    assert_eq!(rcpt.outcome, ReceiptOutcome::Done);
    let detail = rcpt.detail.as_deref().unwrap();
    assert!(!detail.contains("tvly-secret-do-not-leak"));

    let out: SearchOutput = serde_json::from_str(detail).unwrap();
    assert_eq!(out.provider, "duckduckgo");
    assert_eq!(out.query, "rust programming");
    assert_eq!(out.results.len(), 2);
    assert_eq!(out.results[0].title, "Rust & Programming");
    assert_eq!(out.results[0].url, "https://rust-lang.org/");
    assert_eq!(out.results[0].snippet, "Systems language for safety.");
    assert_eq!(out.results[1].url, "https://docs.rs/");
}

#[test]
fn provider_chain_uses_searxng_tavily_and_brave_when_configured() {
    // 1. SearXNG wins first when searxng_url is set.
    {
        let transport = MockWebTransport::new();
        transport.push_route(
            "http://127.0.0.1:8888/search",
            Ok(HttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: br#"{"results":[{"title":"SX","url":"https://sx.example","content":"SearXNG hit"}]}"#.to_vec(),
            }),
        );
        let cfg = WebSearchConfig {
            searxng_url: "http://127.0.0.1:8888".to_owned(),
            ..WebSearchConfig::default()
        };
        let mut body = make_test_body(cfg, transport, MockDnsResolver::new(), MemoryEnv::new());
        let mut runtime = PersistentBodyRuntime::new(None);
        let resp: Waist2Response<ActionExecuteResult> = dispatch_rpc(
            &mut runtime,
            &mut body,
            1,
            Waist2Method::ActionExecute,
            exec_params("sx", "act-sx", "web.search", br#"{"query":"monoagens"}"#),
        );
        let out: SearchOutput =
            serde_json::from_str(resp.result.unwrap().receipt.unwrap().detail.as_deref().unwrap())
                .unwrap();
        assert_eq!(out.provider, "searxng");
        assert_eq!(out.results[0].url, "https://sx.example");
    }

    // 2. Brave wins when SearXNG/Tavily absent and BRAVE_API_KEY is set.
    {
        let transport = MockWebTransport::new();
        transport.push_route(
            "https://api.search.brave.com/res/v1/web/search",
            Ok(HttpResponse {
                status: 200,
                headers: BTreeMap::new(),
                body: br#"{"web":{"results":[{"title":"Brave Hit","url":"https://brave.example","description":"Brave desc"}]}}"#.to_vec(),
            }),
        );
        let mut env = MemoryEnv::new();
        env.insert(ENV_BRAVE_API_KEY, "BSA-secret-key");
        let mut body = make_test_body(
            WebSearchConfig::default(),
            transport,
            MockDnsResolver::new(),
            env,
        );
        let mut runtime = PersistentBodyRuntime::new(None);
        let resp: Waist2Response<ActionExecuteResult> = dispatch_rpc(
            &mut runtime,
            &mut body,
            2,
            Waist2Method::ActionExecute,
            exec_params("brave", "act-brave", "web.search", br#"{"query":"brave test"}"#),
        );
        let out: SearchOutput =
            serde_json::from_str(resp.result.unwrap().receipt.unwrap().detail.as_deref().unwrap())
                .unwrap();
        assert_eq!(out.provider, "brave");
        assert_eq!(out.results[0].snippet, "Brave desc");
        let reqs = body.transport().recorded_requests();
        assert_eq!(
            reqs[0].headers.get("X-Subscription-Token").map(String::as_str),
            Some("BSA-secret-key")
        );
    }
}

#[test]
fn waist2_discover_unknown_capability_and_held_approval_roundtrip() {
    let mut body = make_test_body(
        WebSearchConfig::default(),
        MockWebTransport::new(),
        MockDnsResolver::new(),
        MemoryEnv::new(),
    );
    let mut runtime = PersistentBodyRuntime::new(None);

    // 1. body/discover
    let disc: Waist2Response<BodyDiscoverResult> = dispatch_rpc(
        &mut runtime,
        &mut body,
        1,
        Waist2Method::BodyDiscover,
        BodyDiscoverParams {
            headers: Waist2Headers::new("disc-1"),
            body_id: None,
        },
    );
    let disc_res = disc.result.unwrap();
    assert_eq!(disc_res.body_id.as_str(), DEFAULT_BODY_ID);
    assert_eq!(disc_res.protocol_version, CURRENT_PROTOCOL_VERSION);
    assert_eq!(disc_res.places, vec![PlaceId::new(DEFAULT_PLACE)]);
    let mut sorted = disc_res.capabilities.clone();
    sort_capabilities(&mut sorted);
    assert_eq!(disc_res.capabilities, sorted);
    assert_eq!(disc_res.capabilities.len(), 2);
    assert!(
        disc_res
            .capabilities
            .iter()
            .all(|c| !c.capability_schema_hash.as_str().is_empty())
    );

    // 2. -32001 ERR_UNKNOWN_CAPABILITY
    let unk: Waist2Response<ActionExecuteResult> = dispatch_rpc(
        &mut runtime,
        &mut body,
        2,
        Waist2Method::ActionExecute,
        exec_params("unk-1", "act-unk", "web.nonexistent", b"{}"),
    );
    assert_eq!(
        unk.error.unwrap().code,
        Waist2ErrorCode::UnknownCapability.code()
    );

    // 3. B-MUST-05 held + approval/resolve
    let mut hold_req = exec_params("hold-1", "act-hold-1", "web.fetch", b"hold");
    hold_req.h_act = Some(HAct::new("blake3:held-test"));
    let hold_resp: Waist2Response<ActionExecuteResult> = dispatch_rpc(
        &mut runtime,
        &mut body,
        3,
        Waist2Method::ActionExecute,
        hold_req,
    );
    let hold_rcpt = hold_resp.result.unwrap().receipt.unwrap();
    assert_eq!(hold_rcpt.outcome, ReceiptOutcome::Held);

    let approve_resp: Waist2Response<ApprovalResolveResult> = dispatch_rpc(
        &mut runtime,
        &mut body,
        4,
        Waist2Method::ApprovalResolve,
        ApprovalResolveParams {
            headers: Waist2Headers::new("approve-1"),
            action_id: ActionId::new("act-hold-1"),
            h_act: HAct::new("blake3:held-test"),
            state_epoch: 1,
            approved: true,
            resolver: ActorId::new("per_owner"),
            resolved_at_ms: 100.into(),
            signature: None,
            out_of_band_channel: false,
        },
    );
    assert!(approve_resp.result.unwrap().applied);

    let sq: Waist2Response<StateQueryResult> = dispatch_rpc(
        &mut runtime,
        &mut body,
        5,
        Waist2Method::StateQuery,
        StateQueryParams {
            headers: Waist2Headers::new("sq-1"),
            action_id: ActionId::new("act-hold-1"),
        },
    );
    assert!(matches!(
        sq.result.unwrap().status,
        StateQueryStatus::Terminal { receipt } if receipt.outcome == ReceiptOutcome::Done
    ));
}

#[test]
fn binary_auto_initializes_plugin_config_and_prefers_plugin_state_dir() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("cfg/web-search.toml");
    let plugin_state_dir = tmp.path().join("plugin-state");
    let body_state_dir = tmp.path().join("body-state");

    let bin = env!("CARGO_BIN_EXE_plugin-web-search");

    // 1. Launch binary with MONOAGENS_PLUGIN_CONFIG & MONOAGENS_PLUGIN_STATE_DIR set.
    let mut child = Command::new(bin)
        .env(ENV_PLUGIN_CONFIG, &config_path)
        .env(ENV_PLUGIN_STATE_DIR, &plugin_state_dir)
        .env("MONOAGENS_BODY_STATE_DIR", &body_state_dir)
        .env(ENV_TAVILY_API_KEY, "secret-env-key-should-not-be-written")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    {
        let stdin = child.stdin.as_mut().unwrap();
        let req = Waist2Request::new(
            Some(monoagens_body_sdk::RpcId::Int(1)),
            Waist2Method::BodyDiscover,
            BodyDiscoverParams {
                headers: Waist2Headers::new("bin-disc-1"),
                body_id: None,
            },
        );
        writeln!(stdin, "{}", serde_json::to_string(&req).unwrap()).unwrap();
        stdin.flush().unwrap();
    }
    drop(child.stdin.take());

    let mut stdout_reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    stdout_reader.read_line(&mut line).unwrap();
    let resp: Waist2Response<BodyDiscoverResult> = serde_json::from_str(&line).unwrap();
    assert_eq!(resp.result.unwrap().body_id.as_str(), DEFAULT_BODY_ID);

    let status = child.wait().unwrap();
    assert!(status.success());

    // Config file auto-generated and excludes env API keys.
    assert!(config_path.exists());
    let cfg_contents = fs::read_to_string(&config_path).unwrap();
    assert!(cfg_contents.contains("version = 1"));
    assert!(!cfg_contents.contains("secret-env-key-should-not-be-written"));

    // State persisted under MONOAGENS_PLUGIN_STATE_DIR, NOT MONOAGENS_BODY_STATE_DIR.
    assert!(plugin_state_dir.join("waist2_body_state.json").exists());
    assert!(!body_state_dir.join("waist2_body_state.json").exists());

    // 2. Edit config on disk and verify next launch reloads cleanly without overwriting.
    let updated_cfg = "version = 1\ntimeout_secs = 42\nallow_ports = [80, 443]\n";
    fs::write(&config_path, updated_cfg).unwrap();
    let reloaded = WebSearchConfig::load_or_init_from_path(&config_path).unwrap();
    assert_eq!(reloaded.timeout_secs, 42);
    assert_eq!(fs::read_to_string(&config_path).unwrap(), updated_cfg);
}

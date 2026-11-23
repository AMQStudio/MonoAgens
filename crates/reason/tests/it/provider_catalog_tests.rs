//! Anthropic dialect, discovery and error-classification tests against a local fake server.

use monoagens_core::ModelPurpose;
use monoagens_protocol::EventId;
use monoagens_reason::model::catalog;
use monoagens_reason::model::discovery::{
    DiscoveryError, list_models, list_models_cached, read_cache,
};
use monoagens_reason::{
    MessagesModelConfig, MessagesModelProvider, ModelProvider, ModelRequest, OpenAiModelConfig,
    OpenAiModelProvider, ProviderFailureClass, SecretString,
};
use serde_json::{Value, json};

use crate::fake_http::{FakeServer, Reply, closed_port_url};

const ACTS_JSON: &str = r#"{\"acts\":[]}"#;

fn request() -> ModelRequest {
    ModelRequest::text(
        EventId::new(1),
        ModelPurpose::Reason,
        "test-model",
        "be brief",
        "hello",
    )
}

fn openai_ok() -> Reply {
    Reply::json(
        200,
        &format!(
            r#"{{"model":"m","choices":[{{"message":{{"content":"{ACTS_JSON}"}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10}}}}"#
        ),
    )
}

fn anthropic_ok() -> Reply {
    Reply::json(
        200,
        &format!(
            r#"{{"id":"msg_1","type":"message","role":"assistant","model":"claude-x","content":[{{"type":"text","text":"{ACTS_JSON}"}}],"stop_reason":"end_turn","usage":{{"input_tokens":7,"output_tokens":3}}}}"#
        ),
    )
}

fn anthropic_provider(srv: &FakeServer, key: &str) -> MessagesModelProvider {
    let entry = catalog::find("anthropic").unwrap();
    let mut cfg = MessagesModelConfig::new(
        entry.chat_endpoint(Some(&format!("{}/v1", srv.url()))),
        Some(SecretString::new(key)),
    );
    cfg.key_header = entry.key_header.clone().expect("key_header");
    cfg.extra_headers = entry
        .extra_headers
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    MessagesModelProvider::new(cfg).expect("provider")
}

#[test]
fn anthropic_request_shape_and_headers() {
    let srv = FakeServer::start(|_| anthropic_ok());
    let provider = anthropic_provider(&srv, "sk-ant-test");
    provider.invoke(&request()).expect("invoke");
    let got = srv.captured();
    assert_eq!(got.len(), 1);
    let c = &got[0];
    assert_eq!((c.method.as_str(), c.path()), ("POST", "/v1/messages"));
    assert_eq!(c.header("x-api-key"), Some("sk-ant-test"));
    assert_eq!(c.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(c.header("authorization"), None);
    let body: Value = serde_json::from_str(&c.body).unwrap();
    assert_eq!(body["system"], "be brief");
    assert!(body["max_tokens"].as_u64().unwrap() > 0);
    assert_eq!(body["model"], "test-model");
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"][0]["text"], "hello");
}

#[test]
fn anthropic_and_openai_share_decode_and_usage_mapping() {
    let a_srv = FakeServer::start(|_| anthropic_ok());
    let o_srv = FakeServer::start(|_| openai_ok());
    let a = anthropic_provider(&a_srv, "k").invoke(&request()).expect("anthropic");
    let o = OpenAiModelProvider::new(OpenAiModelConfig::new(
        format!("{}/v1/chat/completions", o_srv.url()),
        None,
    ))
    .unwrap()
    .invoke(&request())
    .expect("openai");
    assert_eq!(a.output, o.output);
    assert_eq!(a.raw_text, o.raw_text);
    let (au, ou) = (a.record.usage.unwrap(), o.record.usage.unwrap());
    assert_eq!(
        (au.input_tokens, au.output_tokens, au.total_tokens),
        (ou.input_tokens, ou.output_tokens, ou.total_tokens)
    );
    assert_eq!(au.input_tokens, 7);
}

#[test]
fn anthropic_error_does_not_leak_key() {
    let srv = FakeServer::start(|_| Reply::json(401, r#"{"error":"bad key sk-ant-leaky"}"#));
    let err = anthropic_provider(&srv, "sk-ant-leaky")
        .invoke(&request())
        .expect_err("401");
    assert_eq!(err.class, ProviderFailureClass::Authentication);
    assert!(!err.detail.contains("sk-ant-leaky"), "{}", err.detail);
}

/// Records the behaviour of the existing OpenAI path before any change (task 2.3).
#[test]
fn openai_error_classification_records_current_behaviour() {
    for (status, class) in [
        (401, ProviderFailureClass::Authentication),
        (403, ProviderFailureClass::Authentication),
        (429, ProviderFailureClass::RateLimit),
        (500, ProviderFailureClass::Server),
        (503, ProviderFailureClass::Server),
        (504, ProviderFailureClass::Timeout),
    ] {
        let srv = FakeServer::start(move |_| Reply::json(status, r#"{"error":"nope"}"#));
        let p = OpenAiModelProvider::new(OpenAiModelConfig::new(
            format!("{}/v1/chat/completions", srv.url()),
            None,
        ))
        .unwrap();
        let err = p.invoke(&request()).expect_err("failure");
        assert_eq!(err.class, class, "status {status}");
        assert_eq!(err.status_code, Some(status));
        assert!(err.detail.contains("nope"), "provider text kept: {}", err.detail);
    }
}

#[test]
fn rate_limit_429_is_classified_for_both_dialects() {
    let mk = || {
        FakeServer::start(|_| {
            let mut r = Reply::json(429, r#"{"error":{"message":"slow down"}}"#);
            r.headers.push(("Retry-After".to_owned(), "7".to_owned()));
            r
        })
    };
    let srv = mk();
    let a = anthropic_provider(&srv, "k").invoke(&request()).expect_err("429");
    assert_eq!(a.class, ProviderFailureClass::RateLimit);
    assert_eq!(a.status_code, Some(429));
    assert!(a.detail.contains("slow down"));

    let srv = mk();
    let o = OpenAiModelProvider::new(OpenAiModelConfig::new(
        format!("{}/v1/chat/completions", srv.url()),
        None,
    ))
    .unwrap()
    .invoke(&request())
    .expect_err("429");
    assert_eq!(o.class, ProviderFailureClass::RateLimit);
    assert!(o.detail.contains("slow down"));
}

#[test]
fn timeout_is_classified_for_both_dialects() {
    let slow = || {
        FakeServer::start(|_| {
            let mut r = anthropic_ok();
            r.delay_ms = 1_500;
            r
        })
    };
    let mut req = request();
    req.timeout_ms = 200;
    let srv = slow();
    let a = anthropic_provider(&srv, "k").invoke(&req).expect_err("timeout");
    assert_eq!(a.class, ProviderFailureClass::Timeout);
    let srv = slow();
    let o = OpenAiModelProvider::new(OpenAiModelConfig::new(
        format!("{}/v1/chat/completions", srv.url()),
        None,
    ))
    .unwrap()
    .invoke(&req)
    .expect_err("timeout");
    assert_eq!(o.class, ProviderFailureClass::Timeout);
}

fn models_body(ids: &[&str]) -> String {
    json!({"object":"list","data": ids.iter().map(|i| json!({"id": i})).collect::<Vec<_>>()})
        .to_string()
}

#[test]
fn discovery_openai_models_keyless_and_bearer() {
    let srv = FakeServer::start(|_| Reply::json(200, &models_body(&["a", "b"])));
    let ollama = catalog::find("ollama").unwrap();
    let base = format!("{}/v1", srv.url());
    let got = list_models(ollama, None, Some(&base)).expect("list");
    assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    let c = &srv.captured()[0];
    assert_eq!((c.method.as_str(), c.path()), ("GET", "/v1/models"));
    assert_eq!(c.header("authorization"), None);

    let openai = catalog::find("openai").unwrap();
    list_models(openai, Some(&SecretString::new("sk-x")), Some(&base)).expect("list");
    assert_eq!(srv.captured()[1].header("authorization"), Some("Bearer sk-x"));
}

#[test]
fn discovery_openrouter_pricing_and_context() {
    let body = r#"{"data":[{"id":"openai/gpt-4","name":"GPT-4","context_length":8192,
        "pricing":{"prompt":"0.00003","completion":"0.00006"}},{"id":"free/x"}]}"#;
    let srv = FakeServer::start(move |_| Reply::json(200, body));
    let entry = catalog::find("openrouter").unwrap();
    let got = list_models(entry, None, Some(&format!("{}/api/v1", srv.url()))).unwrap();
    assert_eq!(got[0].context_length, Some(8192));
    assert!((got[0].price_in_per_mtok.unwrap() - 30.0).abs() < 1e-6);
    assert!((got[0].price_out_per_mtok.unwrap() - 60.0).abs() < 1e-6);
    assert_eq!(got[1].price_in_per_mtok, None);
}

#[test]
fn discovery_anthropic_models_headers_and_paging() {
    let srv = FakeServer::start(|c| {
        if c.target.contains("after_id=m1") {
            Reply::json(200, r#"{"data":[{"id":"m2","display_name":"Two","type":"model"}],"has_more":false,"last_id":"m2"}"#)
        } else {
            Reply::json(200, r#"{"data":[{"id":"m1","display_name":"One","type":"model"}],"has_more":true,"last_id":"m1"}"#)
        }
    });
    let entry = catalog::find("anthropic").unwrap();
    let got = list_models(
        entry,
        Some(&SecretString::new("sk-ant")),
        Some(&format!("{}/v1", srv.url())),
    )
    .unwrap();
    assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["m1", "m2"]);
    assert_eq!(got[0].display_name.as_deref(), Some("One"));
    let c = &srv.captured()[0];
    assert_eq!(c.path(), "/v1/models");
    assert_eq!(c.header("x-api-key"), Some("sk-ant"));
    assert_eq!(c.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(
        list_models(entry, None, Some(&srv.url())).unwrap_err(),
        DiscoveryError::MissingKey
    );
}

#[test]
fn discovery_gemini_native_strips_models_prefix_and_keeps_key_out_of_errors() {
    let srv = FakeServer::start(|c| {
        if c.target.contains("key=gem-key") {
            Reply::json(
                200,
                r#"{"models":[{"name":"models/gemini-2.5-flash","displayName":"Flash","inputTokenLimit":1048576,"supportedGenerationMethods":["generateContent"]},
                {"name":"models/embed-1","supportedGenerationMethods":["embedContent"]}]}"#,
            )
        } else {
            Reply::json(400, r#"{"error":"bad key"}"#)
        }
    });
    let entry = catalog::find("gemini").unwrap();
    let base = format!("{}/v1beta/openai", srv.url());
    let got = list_models(entry, Some(&SecretString::new("gem-key")), Some(&base)).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id, "gemini-2.5-flash");
    assert_eq!(got[0].context_length, Some(1_048_576));
    assert_eq!(srv.captured()[0].path(), "/v1beta/models");

    let err = list_models(entry, Some(&SecretString::new("wrong-key")), Some(&base)).unwrap_err();
    assert!(matches!(err, DiscoveryError::Http { status: 400, .. }));
    let dead = closed_port_url();
    let err = list_models(
        entry,
        Some(&SecretString::new("secret-in-url")),
        Some(&format!("{dead}/v1beta/openai")),
    )
    .unwrap_err();
    assert!(!err.to_string().contains("secret-in-url"), "{err}");
}

#[test]
fn discovery_cache_written_then_served_stale_when_offline() {
    let dir = tempfile::tempdir().unwrap();
    let srv = FakeServer::start(|_| Reply::json(200, &models_body(&["m1", "m2"])));
    let entry = catalog::find("ollama").unwrap();
    let base = format!("{}/v1", srv.url());
    let (fresh, err) = list_models_cached(entry, None, Some(&base), dir.path());
    assert!(err.is_none() && !fresh.stale);
    assert!(fresh.fetched_at_ms > 0);
    assert_eq!(read_cache(dir.path(), "ollama").unwrap().models.len(), 2);

    let offline = format!("{}/v1", closed_port_url());
    let (stale, err) = list_models_cached(entry, None, Some(&offline), dir.path());
    assert!(matches!(err, Some(DiscoveryError::Network(_))), "{err:?}");
    assert!(stale.stale);
    assert_eq!(stale.models, fresh.models);
    assert_eq!(stale.fetched_at_ms, fresh.fetched_at_ms);

    let empty_dir = tempfile::tempdir().unwrap();
    let (none, err) = list_models_cached(entry, None, Some(&offline), empty_dir.path());
    assert!(err.is_some() && none.models.is_empty() && none.stale);
}

#[test]
fn discovery_http_error_and_unsupported() {
    let srv = FakeServer::start(|_| Reply::json(401, r#"{"error":"no"}"#));
    let entry = catalog::find("openai").unwrap();
    let err = list_models(entry, Some(&SecretString::new("k")), Some(&srv.url())).unwrap_err();
    assert!(matches!(err, DiscoveryError::Http { status: 401, .. }));
    let mut none = entry.clone();
    none.discovery = monoagens_reason::model::DiscoveryMode::None;
    assert_eq!(list_models(&none, None, None).unwrap_err(), DiscoveryError::Unsupported);
}

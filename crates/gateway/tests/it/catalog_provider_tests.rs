//! `model-provider-catalog` config and provider wiring (fake local server, no network).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::thread;

use monoagens_core::ModelPurpose;
use monoagens_gateway::{
    GatewayConfig, ModelProviderKind, build_provider_from_config, parse_config_str,
};
use monoagens_protocol::EventId;
use monoagens_reason::{ModelProvider, ModelRequest, ProviderSecrets, SecretString};

type Seen = Arc<Mutex<Vec<(String, Vec<(String, String)>, String)>>>;

/// Serves `n` requests with `reply`, recording `(request line, lower-cased headers, body)`.
fn serve(n: usize, reply: &'static str) -> (String, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    let seen: Seen = Arc::default();
    let seen_bg = Arc::clone(&seen);
    thread::spawn(move || {
        for _ in 0..n {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut line = String::new();
            reader.read_line(&mut line).expect("line");
            let mut headers = Vec::new();
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).expect("hdr");
                let h = h.trim_end().to_owned();
                if h.is_empty() {
                    break;
                }
                if let Some((k, v)) = h.split_once(':') {
                    let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_owned());
                    if k == "content-length" {
                        len = v.parse().unwrap_or(0);
                    }
                    headers.push((k, v));
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).expect("body");
            seen_bg.lock().expect("lock").push((
                line.trim().to_owned(),
                headers,
                String::from_utf8_lossy(&body).into_owned(),
            ));
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    (format!("http://{addr}"), seen)
}

const OPENAI_REPLY: &str = r#"{"model":"m","choices":[{"message":{"content":"{\"acts\":[]}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
const MESSAGES_REPLY: &str = r#"{"model":"m","content":[{"type":"text","text":"{\"acts\":[]}"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#;

fn cfg(toml: &str) -> GatewayConfig {
    parse_config_str(toml, Path::new("monoagens.toml"), Path::new(".")).expect("config")
}

fn invoke(provider: &dyn ModelProvider) {
    let req = ModelRequest::text(EventId::new(1), ModelPurpose::Reason, "m", "sys", "usr");
    provider.invoke(&req).expect("invoke");
}

fn header<'a>(h: &'a [(String, String)], name: &str) -> Option<&'a str> {
    h.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

#[test]
fn legacy_openai_compatible_config_behaves_as_before() {
    let c = cfg("version = 1\n[model]\nprovider = \"openai_compatible\"\nmodel = \"m\"\n");
    assert_eq!(c.model.provider, ModelProviderKind::OpenAiCompatible);
    assert_eq!(c.model.catalog_provider, None);
    assert_eq!(c.model.provider_base_url, None);

    let (base, seen) = serve(1, OPENAI_REPLY);
    let secrets = ProviderSecrets {
        model_endpoint: Some(format!("{base}/v1/chat/completions")),
        model_api_key: Some(SecretString::new("legacy-key")),
        ..ProviderSecrets::default()
    };
    let provider = build_provider_from_config(&c, &secrets).expect("provider");
    assert_eq!(provider.provider_name(), "openai_compatible");
    invoke(provider.as_ref());
    let seen = seen.lock().unwrap();
    assert!(seen[0].0.starts_with("POST /v1/chat/completions"), "{}", seen[0].0);
    assert_eq!(header(&seen[0].1, "authorization"), Some("Bearer legacy-key"));
}

#[test]
fn catalog_id_config_parses_with_base_url_option_and_rejects_unknown() {
    let c = cfg(
        "version = 1\n[model]\nprovider = \"groq\"\nmodel = \"m\"\n[model.provider_options]\nbase_url = \"http://127.0.0.1:1/v1\"\n",
    );
    assert_eq!(c.model.provider, ModelProviderKind::OpenAiCompatible);
    assert_eq!(c.model.catalog_provider.as_deref(), Some("groq"));
    assert_eq!(c.model.provider_base_url.as_deref(), Some("http://127.0.0.1:1/v1"));

    let err = parse_config_str(
        "version = 1\n[model]\nprovider = \"no-such-provider\"\n",
        Path::new("monoagens.toml"),
        Path::new("."),
    )
    .expect_err("unknown provider");
    assert!(err.to_string().contains("model.provider"), "{err}");
}

#[test]
fn keyless_catalog_provider_sends_no_authorization_and_honours_base_url() {
    let (base, seen) = serve(1, OPENAI_REPLY);
    let c = cfg(&format!(
        "version = 1\n[model]\nprovider = \"ollama\"\nmodel = \"m\"\n[model.provider_options]\nbase_url = \"{base}/v1\"\n"
    ));
    // Even a stray legacy key must not be sent to a keyless provider.
    let secrets = ProviderSecrets {
        model_api_key: Some(SecretString::new("should-not-be-sent")),
        ..ProviderSecrets::default()
    };
    let provider = build_provider_from_config(&c, &secrets).expect("provider");
    invoke(provider.as_ref());
    let seen = seen.lock().unwrap();
    assert!(seen[0].0.starts_with("POST /v1/chat/completions"), "{}", seen[0].0);
    assert_eq!(header(&seen[0].1, "authorization"), None);
    assert!(!seen[0].2.contains("should-not-be-sent"));
}

#[test]
fn catalog_provider_uses_per_provider_key_from_secrets_toml() {
    let (base, seen) = serve(1, OPENAI_REPLY);
    let c = cfg(&format!(
        "version = 1\n[model]\nprovider = \"groq\"\nmodel = \"m\"\n[model.provider_options]\nbase_url = \"{base}/v1\"\n"
    ));
    let secrets = monoagens_gateway::parse_secrets_toml_str(
        "model_api_key = \"legacy\"\n[providers.groq]\napi_key = \"per-provider\"\n",
        Path::new("secrets.toml"),
    )
    .expect("secrets");
    let provider = build_provider_from_config(&c, &secrets).expect("provider");
    invoke(provider.as_ref());
    let seen = seen.lock().unwrap();
    assert_eq!(header(&seen[0].1, "authorization"), Some("Bearer per-provider"));
    assert!(!format!("{secrets:?}").contains("per-provider"));
}

#[test]
fn messages_dialect_catalog_provider_uses_x_api_key() {
    let (base, seen) = serve(1, MESSAGES_REPLY);
    let c = cfg(&format!(
        "version = 1\n[model]\nprovider = \"anthropic\"\nmodel = \"m\"\n[model.provider_options]\nbase_url = \"{base}/v1\"\n"
    ));
    let secrets = monoagens_gateway::parse_secrets_toml_str(
        "[providers.anthropic]\napi_key = \"k-ant\"\n",
        Path::new("secrets.toml"),
    )
    .expect("secrets");
    let provider = build_provider_from_config(&c, &secrets).expect("provider");
    assert_eq!(provider.provider_name(), "messages_api");
    invoke(provider.as_ref());
    let seen = seen.lock().unwrap();
    assert!(seen[0].0.starts_with("POST /v1/messages"), "{}", seen[0].0);
    assert_eq!(header(&seen[0].1, "x-api-key"), Some("k-ant"));
    assert_eq!(header(&seen[0].1, "authorization"), None);
}

#[test]
fn custom_provider_without_base_url_is_a_clear_error() {
    let c = cfg("version = 1\n[model]\nprovider = \"custom\"\nmodel = \"m\"\n");
    let err = build_provider_from_config(&c, &ProviderSecrets::default())
        .err()
        .expect("error");
    assert!(err.to_string().contains("base_url"), "{err}");
}

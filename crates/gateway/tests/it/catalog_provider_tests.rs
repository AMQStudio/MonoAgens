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
fn legacy_openai_compatible_kind_has_no_runtime_provider() {
    let c = cfg("version = 1\n[model]\nprovider = \"openai_compatible\"\nmodel = \"m\"\n");
    let secrets = ProviderSecrets {
        model_endpoint: Some("http://127.0.0.1:1/v1/chat/completions".to_owned()),
        model_api_key: Some(SecretString::new("legacy-key")),
        ..ProviderSecrets::default()
    };
    let err = build_provider_from_config(&c, &secrets).err().expect("no legacy provider");
    assert!(err.to_string().contains("/provider"), "{err}");
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
fn named_custom_provider_uses_its_own_base_url_and_only_its_own_key() {
    let (base, seen) = serve(1, OPENAI_REPLY);
    let c = cfg(&format!(
        "version = 1\n[providers.custom.my-gw]\nbase_url = \"{base}/v1\"\ndialect = \"openai_chat\"\n[model]\nprovider = \"my-gw\"\nmodel = \"m\"\n"
    ));
    assert_eq!(c.model.catalog_provider.as_deref(), Some("my-gw"));
    let secrets = monoagens_gateway::parse_secrets_toml_str(
        "model_api_key = \"legacy\"\n[providers.my-gw]\napi_key = \"gw-key\"\n[providers.groq]\napi_key = \"groq-key\"\n",
        Path::new("secrets.toml"),
    )
    .expect("secrets");
    let provider = build_provider_from_config(&c, &secrets).expect("provider");
    invoke(provider.as_ref());
    let seen = seen.lock().unwrap();
    assert!(seen[0].0.starts_with("POST /v1/chat/completions"), "{}", seen[0].0);
    assert_eq!(header(&seen[0].1, "authorization"), Some("Bearer gw-key"));
    assert!(!seen[0].2.contains("groq-key"));
}

#[test]
fn legacy_key_is_never_resolved_for_a_catalog_provider() {
    let secrets = ProviderSecrets {
        model_api_key: Some(SecretString::new("legacy-key")),
        ..ProviderSecrets::default()
    };
    for id in ["openai", "anthropic", "groq", "opencode-go"] {
        let entry = monoagens_reason::model::catalog::find(id).expect("entry");
        assert!(secrets.api_key_for(entry, |_| None).is_none(), "{id}");
    }
}

#[test]
fn legacy_opencode_go_config_is_migrated_once_with_backups() {
    let dir = tempfile::tempdir().expect("tmp");
    let cfg_path = dir.path().join("monoagens.toml");
    let sec_path = dir.path().join("secrets.toml");
    let cfg_text = "version = 1\n[model]\nprovider = \"openai_compatible\"\nmodel = \"deepseek-v4.1-flash\"\n";
    let sec_text = "model_endpoint = \"https://opencode.ai/inference/go/openai/v1/chat/completions\"\nmodel_api_key = \"oc_sk_test_key\"\n";
    std::fs::write(&cfg_path, cfg_text).unwrap();
    std::fs::write(&sec_path, sec_text).unwrap();

    let env = monoagens_gateway::load_or_init_gateway_env(Some(&cfg_path), |_| None).expect("load");
    assert_eq!(env.config.model.catalog_provider.as_deref(), Some("opencode-go"));
    assert_eq!(env.config.model.model, "deepseek-v4.1-flash");
    let sec_after = std::fs::read_to_string(&sec_path).unwrap();
    assert!(!sec_after.contains("model_endpoint") && !sec_after.contains("model_api_key"), "{sec_after}");
    assert!(sec_after.contains("[providers.opencode-go]") && sec_after.contains("oc_sk_test_key"));
    assert_eq!(std::fs::read_to_string(dir.path().join("monoagens.toml.bak")).unwrap(), cfg_text);
    assert_eq!(std::fs::read_to_string(dir.path().join("secrets.toml.bak")).unwrap(), sec_text);
    let entry = monoagens_reason::model::catalog::find("opencode-go").unwrap();
    assert_eq!(
        entry.chat_endpoint(None),
        "https://opencode.ai/inference/go/openai/v1/chat/completions"
    );
    assert_eq!(
        env.secrets.api_key_for(entry, |_| None).unwrap().expose(),
        "oc_sk_test_key"
    );

    // Second load: nothing left to migrate, files unchanged.
    let cfg_after = std::fs::read_to_string(&cfg_path).unwrap();
    monoagens_gateway::load_or_init_gateway_env(Some(&cfg_path), |_| None).expect("reload");
    assert_eq!(std::fs::read_to_string(&cfg_path).unwrap(), cfg_after);
    assert_eq!(std::fs::read_to_string(&sec_path).unwrap(), sec_after);
}

#[test]
fn unknown_legacy_endpoint_becomes_a_host_named_custom_provider() {
    let dir = tempfile::tempdir().expect("tmp");
    let cfg_path = dir.path().join("monoagens.toml");
    let sec_path = dir.path().join("secrets.toml");
    std::fs::write(&cfg_path, "version = 1\n[model]\nprovider = \"openai_compatible\"\nmodel = \"m\"\n").unwrap();
    std::fs::write(
        &sec_path,
        "model_endpoint = \"https://gw.example.com/v1/chat/completions\"\nmodel_api_key = \"k-1\"\n",
    )
    .unwrap();
    let env = monoagens_gateway::load_or_init_gateway_env(Some(&cfg_path), |_| None).expect("load");
    assert_eq!(env.config.model.catalog_provider.as_deref(), Some("gw-example-com"));
    assert_eq!(env.config.custom_providers.len(), 1);
    assert_eq!(env.config.custom_providers[0].base_url, "https://gw.example.com/v1");
    let entry = monoagens_reason::model::catalog::find("gw-example-com").unwrap();
    assert_eq!(entry.chat_endpoint(None), "https://gw.example.com/v1/chat/completions");
    assert_eq!(env.secrets.api_key_for(entry, |_| None).unwrap().expose(), "k-1");
}

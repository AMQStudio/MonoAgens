//! Catalog data tests (provider names live here and in `catalog.toml` only).

use monoagens_reason::model::catalog::{self, Dialect, DiscoveryMode};

#[test]
fn catalog_lists_every_provider_with_dialect_and_base_url() {
    let got: Vec<(&str, Dialect, &str)> = catalog::entries()
        .iter()
        .map(|e| (e.id.as_str(), e.dialect, e.base_url.as_str()))
        .collect();
    let oa = Dialect::OpenAiChat;
    let expected = [
        ("openai", oa, "https://api.openai.com/v1"),
        ("anthropic", Dialect::MessagesApi, "https://api.anthropic.com/v1"),
        ("gemini", oa, "https://generativelanguage.googleapis.com/v1beta/openai"),
        ("openrouter", oa, "https://openrouter.ai/api/v1"),
        ("groq", oa, "https://api.groq.com/openai/v1"),
        ("mistral", oa, "https://api.mistral.ai/v1"),
        ("xai", oa, "https://api.x.ai/v1"),
        ("deepseek", oa, "https://api.deepseek.com/v1"),
        ("together", oa, "https://api.together.xyz/v1"),
        ("fireworks", oa, "https://api.fireworks.ai/inference/v1"),
        ("cerebras", oa, "https://api.cerebras.ai/v1"),
        ("deepinfra", oa, "https://api.deepinfra.com/v1/openai"),
        ("nvidia", oa, "https://integrate.api.nvidia.com/v1"),
        ("moonshot", oa, "https://api.moonshot.ai/v1"),
        ("zai", oa, "https://api.z.ai/api/paas/v4"),
        ("minimax", oa, "https://api.minimax.io/v1"),
        ("dashscope", oa, "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"),
        ("huggingface", oa, "https://router.huggingface.co/v1"),
        ("sambanova", oa, "https://api.sambanova.ai/v1"),
        ("novita", oa, "https://api.novita.ai/v3/openai"),
        ("opencode-go", oa, "https://opencode.ai/inference/go/openai/v1"),
        ("opencode-zen", oa, "https://opencode.ai/inference/openai/v1"),
        ("ollama", oa, "http://127.0.0.1:11434/v1"),
        ("lm-studio", oa, "http://127.0.0.1:1234/v1"),
        ("llama-cpp", oa, "http://127.0.0.1:8080/v1"),
        ("vllm", oa, "http://127.0.0.1:8000/v1"),
    ];
    assert_eq!(got, expected);
}

#[test]
fn keyless_entries_env_names_and_data_driven_constants() {
    for id in ["ollama", "lm-studio", "llama-cpp", "vllm"] {
        let e = catalog::find(id).expect(id);
        assert!(e.keyless && e.api_key_env.is_empty(), "{id}");
    }
    let go = catalog::find("opencode-go").unwrap();
    assert_eq!(go.api_key_env, ["OPENCODE_API_KEY"]);
    assert_eq!(go.default_model, "deepseek-v4.1-flash");
    let g = catalog::find("gemini").unwrap();
    assert_eq!(g.discovery, DiscoveryMode::NativeModelsListQueryKey);
    assert_eq!(g.model_id_prefix.as_deref(), Some("models/"));
    assert!(g.discovery_url.is_some());
    let a = catalog::find("anthropic").unwrap();
    assert_eq!(a.discovery, DiscoveryMode::ModelsListHeaderKey);
    assert_eq!(a.key_header.as_deref(), Some("x-api-key"));
    assert_eq!(a.extra_headers.get("anthropic-version").map(String::as_str), Some("2023-06-01"));
    assert!(catalog::find("nope").is_none());
}

#[test]
fn endpoint_derivation_and_override() {
    let e = catalog::find("opencode-go").unwrap();
    assert_eq!(
        e.chat_endpoint(None),
        "https://opencode.ai/inference/go/openai/v1/chat/completions"
    );
    assert_eq!(
        e.chat_endpoint(Some("http://127.0.0.1:9/v1/")),
        "http://127.0.0.1:9/v1/chat/completions"
    );
    assert_eq!(
        catalog::find("anthropic").unwrap().chat_endpoint(None),
        "https://api.anthropic.com/v1/messages"
    );
}

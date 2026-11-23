# Specification — `model-provider-catalog`

## ADDED Requirements

### Requirement: Data-Driven Provider Catalog
MonoAgens SHALL ship its model providers as an embedded data catalog where each entry gives id, label, wire dialect, base URL, API-key environment names, keyless flag and discovery mode, covering the 27 providers in `design.md` D6 plus a `custom` entry.

#### Scenario: Catalog lists the D6 providers
- **WHEN** the catalog is loaded in a test
- **THEN** it contains every provider id in the D6 table, each with a dialect and a base URL (except `custom`).

### Requirement: Per-Provider Keys with Fixed Lookup Order
MonoAgens SHALL resolve a provider's key from `secrets.toml [providers.<id>]`, then the catalog's environment names, then the legacy `model_api_key`, and SHALL never write a key to the log.

#### Scenario: Lookup order
- **WHEN** `secrets.toml` has a key for `groq` and `GROQ_API_KEY` is also set to a different value
- **THEN** the `secrets.toml` key is used.

### Requirement: Legacy Configuration Keeps Working
A configuration with `provider = "openai_compatible"` and `secrets.toml` entries `model_endpoint` and `model_api_key` SHALL boot with the same endpoint and key as before this change.

#### Scenario: Legacy config boots
- **WHEN** the gateway boots from a legacy configuration in a test
- **THEN** the provider's chat URL and Authorization header equal the pre-change values.

### Requirement: Keyless Local Providers
A catalog entry marked keyless SHALL send chat requests without an Authorization header.

#### Scenario: Ollama without a key
- **WHEN** the user selects `ollama` with no key and a turn is sent to the fake provider server on its port override
- **THEN** the request path is `/v1/chat/completions` and the request has no Authorization header.

### Requirement: Parameterised OpenAI Chat Dialect
The `openai_chat` dialect SHALL take its base URL, headers and output-limit parameter name (`max_tokens` or `max_completion_tokens`) from the catalog entry and the user's overrides.

#### Scenario: Base URL override
- **WHEN** `[model.provider_options] base_url` is set for `openrouter`
- **THEN** chat requests go to `{base_url}/chat/completions`.

### Requirement: Native Anthropic Messages Dialect
The `anthropic_messages` dialect SHALL send `POST {base}/messages` with `x-api-key` and `anthropic-version: 2023-06-01`, a top-level system prompt and a required `max_tokens`, and SHALL map response text and `input_tokens`/`output_tokens` into the same model result as the OpenAI dialect.

#### Scenario: Same decoded output across dialects
- **WHEN** equivalent scripted response bodies are decoded through each dialect's response mapping
- **THEN** both yield the same model output and the same token usage.

### Requirement: Model Discovery with Cache
For providers with discovery MonoAgens SHALL list models (`GET {base}/models`, Anthropic's models endpoint, or Gemini's native list with the `models/` prefix removed) within a 10 s timeout, cache the result per provider with its fetch time, return the cache immediately and refresh in the background.

#### Scenario: Gemini prefix stripped
- **WHEN** the fake server returns Gemini-style ids `models/gemini-x`
- **THEN** discovery returns `gemini-x`.

#### Scenario: Discovery fails offline
- **WHEN** a refresh fails and a cached list exists
- **THEN** the cached list is returned with its age, the log has a `discovery` error for that provider, and an error toast points to `/log`.

### Requirement: Live Provider Rebuild
Changing provider, model, key or base URL through `config.apply` SHALL rebuild the main and titler providers in the running gateway without a restart.

#### Scenario: Switch model without restart
- **WHEN** `config.apply` changes `[model].model` while the TUI is running against the fake provider
- **THEN** the next turn's request body carries the new model id.

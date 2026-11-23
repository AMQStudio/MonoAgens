# Specification — `provider-config-cleanup`

## ADDED Requirements

### Requirement: Legacy Endpoint Configuration Is Removed by Migration
The `openai_compatible` provider kind and the `model_endpoint` / `model_api_key` secrets SHALL no longer be used at runtime. On load, a legacy configuration SHALL be migrated once, with a `.bak` of both files: if the endpoint matches a catalog provider's base URL, `[model] provider` becomes that catalog id and the key moves to `[providers.<id>] api_key`; otherwise a named custom provider is created from the endpoint (name derived from the host) and the owner is told to rename it.

#### Scenario: OpenCode Go legacy config
- **WHEN** the config has `provider = "openai_compatible"` and the secrets have `model_endpoint = https://opencode.ai/inference/go/openai/v1/chat/completions` and a `model_api_key`
- **THEN** after load `[model] provider = "opencode-go"`, the key is in `[providers.opencode-go] api_key`, `model_endpoint` and `model_api_key` are gone, both `.bak` files exist, and the next request goes to the same endpoint with the same key.

### Requirement: Custom Endpoints Are Named Providers
A provider that is not in the catalog SHALL be added as a named custom provider with a unique name, base URL, dialect (`openai_chat` or `messages_api`) and optional key, created from `/provider` → "Add custom provider"; it then behaves like any catalog provider (discovery, `/model`, keys scoped to it). The unnamed `custom` catalog entry SHALL be removed.

#### Scenario: Adding a custom provider
- **WHEN** the owner adds custom provider `my-gateway` with base URL `https://gw.example/v1` and a key
- **THEN** it appears in `/provider` and `/model` as `my-gateway`, its key is stored only for it, and discovery calls only `https://gw.example/v1/models`.

### Requirement: Model Picker Shows Only Logged-In Providers' Models
`/model` SHALL list models only from logged-in providers (own key set, or a local keyless provider the owner explicitly enabled); other providers SHALL appear below a separator as not logged in, with no discovery request, and selecting one SHALL open its key entry. No discovery request SHALL ever be made for a provider that is not logged in.

#### Scenario: One logged-in provider
- **WHEN** only `opencode-go` has a key and the owner opens `/model`
- **THEN** the model list contains only `opencode-go` models, every other provider is listed as not logged in, and exactly one discovery request (to `opencode-go`) is made.

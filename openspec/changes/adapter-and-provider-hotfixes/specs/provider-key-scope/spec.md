# Specification — `provider-key-scope`

## ADDED Requirements

### Requirement: Keys Are Only Sent to the Provider They Belong To
A provider SHALL receive only its own key: from `secrets.toml [providers.<id>]` or that catalog entry's environment names. The legacy `model_api_key` SHALL be used only for the legacy `openai_compatible` provider and for a catalog provider whose resolved base URL equals the host of the configured legacy `model_endpoint`; it SHALL never be sent to any other provider.

#### Scenario: Legacy key is not sprayed
- **WHEN** only `model_api_key` and `model_endpoint = https://opencode.ai/inference/go/openai/v1/chat/completions` are set and discovery is requested for `openai`
- **THEN** no request carrying that key is made to `api.openai.com`, and `openai` shows as "no key" in `/provider`.

### Requirement: Discovery Runs Only On Demand for Configured Providers
Model discovery SHALL NOT run at startup for every catalog provider. It SHALL run for the active provider, and for another provider only when the owner opens it in `/model` or `/provider` and that provider has its own key or is keyless; keyless local providers that are unreachable SHALL show "not running" without a warning in the log.

#### Scenario: Quiet startup
- **WHEN** the TUI starts with one configured provider
- **THEN** at most one discovery request is made, to that provider, and the log has no discovery warnings for other providers.

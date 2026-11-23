# Design — `model-provider-catalog`

Audit: [../tui-foundation/audit.md](../tui-foundation/audit.md).

## Context (facts checked on 2026-11-23)
- `ModelProvider::invoke` is blocking (`crates/reason/src/model/mod.rs:312`).
- Config: `crates/gateway/src/config.rs` (`ModelConfig` L172, `ModelProviderKind` L37). Secrets:
  `crates/reason/src/secrets.rs` (`ProviderSecrets`).
- The user's live provider is OpenCode Go: chat endpoint
  `https://opencode.ai/inference/go/openai/v1/chat/completions`; `GET .../v1/models` with
  `Authorization: Bearer <key>` returns an OpenAI list (`{"data":[{"id":...}]}`, 30 models). Rule: models URL =
  chat endpoint with the trailing `/chat/completions` replaced by `/models`.
- Titler output budget is `model.output_limits.classify` (user config: 1024).

## D6. Provider catalog and model discovery
Reference: oh-my-pi splits a provider into a catalog entry (id, default model, env names, base URL, wire API,
discovery) and an auth entry; dispatch keys on the wire API, not on the provider id. We do the same, smaller.
- `crates/reason/src/model/catalog.rs` + embedded `catalog.toml` (data, not code). Entry:
  `id, label, dialect, base_url, api_key_env: [..], keyless: bool, discovery: openai_models|anthropic_models|gemini_native|none, default_model, docs_url, seed_models: [..]`.
- Dialects: `openai_chat` (existing `openai.rs`, parameterised by base URL, headers, `output_limit_param`)
  and `anthropic_messages` (new: `POST {base}/messages`, headers `x-api-key`, `anthropic-version: 2023-06-01`,
  `max_tokens` required, system as top-level field; map usage `input_tokens`/`output_tokens`).
- Presets. Every base URL below was probed with an unauthenticated `GET {base}/models` on 2026-11-23
  (401/403 = endpoint exists, 200 = public list, 404 = no list):

  | id | dialect | base_url | key env | discovery | probe |
  |---|---|---|---|---|---|
  | openai | openai_chat | https://api.openai.com/v1 | OPENAI_API_KEY | openai_models | 401 |
  | anthropic | anthropic_messages | https://api.anthropic.com/v1 | ANTHROPIC_API_KEY | anthropic_models | 401 |
  | gemini | openai_chat | https://generativelanguage.googleapis.com/v1beta/openai | GEMINI_API_KEY | gemini_native | 404 on /openai/models -> list via `GET https://generativelanguage.googleapis.com/v1beta/models?key=...` |
  | openrouter | openai_chat | https://openrouter.ai/api/v1 | OPENROUTER_API_KEY | openai_models | 200 |
  | groq | openai_chat | https://api.groq.com/openai/v1 | GROQ_API_KEY | openai_models | 401 |
  | mistral | openai_chat | https://api.mistral.ai/v1 | MISTRAL_API_KEY | openai_models | 401 |
  | xai | openai_chat | https://api.x.ai/v1 | XAI_API_KEY | openai_models | 401 |
  | deepseek | openai_chat | https://api.deepseek.com/v1 | DEEPSEEK_API_KEY | openai_models | 401 |
  | together | openai_chat | https://api.together.xyz/v1 | TOGETHER_API_KEY | openai_models | 401 |
  | fireworks | openai_chat | https://api.fireworks.ai/inference/v1 | FIREWORKS_API_KEY | openai_models | 401 |
  | cerebras | openai_chat | https://api.cerebras.ai/v1 | CEREBRAS_API_KEY | openai_models | 403 |
  | deepinfra | openai_chat | https://api.deepinfra.com/v1/openai | DEEPINFRA_API_KEY | openai_models | 200 |
  | nvidia | openai_chat | https://integrate.api.nvidia.com/v1 | NVIDIA_API_KEY | openai_models | 200 |
  | moonshot | openai_chat | https://api.moonshot.ai/v1 | MOONSHOT_API_KEY | openai_models | 401 |
  | zai | openai_chat | https://api.z.ai/api/paas/v4 | ZAI_API_KEY | openai_models | 401 |
  | minimax | openai_chat | https://api.minimax.io/v1 | MINIMAX_API_KEY | openai_models | 401 |
  | dashscope | openai_chat | https://dashscope-intl.aliyuncs.com/compatible-mode/v1 | DASHSCOPE_API_KEY | openai_models | 401 |
  | huggingface | openai_chat | https://router.huggingface.co/v1 | HF_TOKEN | openai_models | 200 |
  | sambanova | openai_chat | https://api.sambanova.ai/v1 | SAMBANOVA_API_KEY | openai_models | 200 |
  | novita | openai_chat | https://api.novita.ai/v3/openai | NOVITA_API_KEY | openai_models | 200 |
  | opencode-go | openai_chat | https://opencode.ai/inference/go/openai/v1 | OPENCODE_API_KEY | openai_models | 200 (verified with key: 30 models) |
  | opencode-zen | openai_chat | https://opencode.ai/inference/openai/v1 | OPENCODE_API_KEY | openai_models | 200 |
  | ollama | openai_chat | http://127.0.0.1:11434/v1 | (keyless) | openai_models | local |
  | lm-studio | openai_chat | http://127.0.0.1:1234/v1 | (keyless) | openai_models | local |
  | llama-cpp | openai_chat | http://127.0.0.1:8080/v1 | (keyless) | openai_models | local |
  | vllm | openai_chat | http://127.0.0.1:8000/v1 | (keyless) | openai_models | local |
  | custom | openai_chat | (user) | (user) | openai_models | - |

  `default_model` and `seed_models` are left empty in the catalog except where the user's own config gives
  one (`opencode-go` -> `deepseek-v4.1-flash`); the model list comes from discovery. Gemini's discovery result
  ids carry a `models/` prefix: strip it.
- Config (additive, back-compatible):
  ```toml
  [model]
  provider = "opencode-go"      # catalog id; "openai_compatible" + secrets model_endpoint keeps working
  model = "deepseek-v4.1-flash"
  # optional: base_url override for any provider
  [model.provider_options]
  base_url = "..."
  ```
  secrets.toml: `[providers.<id>] api_key = "..."` (0600, masked). Lookup order: `secrets.toml [providers.<id>]`
  -> env names from the catalog -> legacy `model_api_key`.
- Discovery: `crates/reason/src/model/discovery.rs`, blocking `list_models(entry, key) -> Result<Vec<ModelInfo>>`
  (10 s timeout), cached in `<data_dir>/cache/models/<provider>.json` with fetch time; the picker shows the cache
  instantly and refreshes in the background; `r` forces refresh; failure shows cached list + error toast + log
  (`discovery` source). Exposed to every client as admin method `models.discover` (proposed params
  `{"provider":"<id>","refresh":bool}`, result `{models:[ModelInfo], fetched_at, stale:bool}` - confirm in review).
  The Anthropic and Gemini listing response shapes are not probed beyond the status codes above: verify during
  implementation with context7 docs.
- Switching provider/model rebuilds the `Arc<dyn ModelProvider>` (and the titler's) live; no restart. The
  rebuild is the hook `config.apply` calls (foundation).

## D10 (part). Picker and login patterns taken from omp v19.1.0 (observed in tmux on 2026-11-23)
Match the patterns, not omp's code.
- **Model picker (`/model`, also from the Model tab)**: two panes. Left: providers, configured ones first with
  `●` and their model count, then a separator and unconfigured ones with `○`; a first row "All models". Right:
  search line, model list (`provider/model-id` plus whatever metadata discovery returned, such as context
  size), scrollbar, and a detail line for the focused model. Choosing a model on an unconfigured provider opens
  that provider's key page first. Footer lists the keys (`⏎ select · ↑/↓ · type to search · ⎋ close`).
- **Provider login (`/login`)**: a searchable provider list with `● key set` / `○` status per row; Enter opens
  the key entry (masked) and a connection test, then returns to where the user came from.
- Add `/model` and `/login` slash commands that open these directly. Their menu descriptions show live state
  (`/model` -> current model).
- The settings Providers and Model tabs that reuse these screens are completed in `tui-settings-complete`.

## D-5. Model per purpose
- Optional `[model.purposes.<purpose>] provider = "..." model = "..."` for purposes `classify`, `titler`,
  `consolidate` (the set of purposes comes from the existing recorded purposes of model calls - verify the
  names in `crates/reason` during implementation). Absent = use `[model]`. The titler uses `classify`'s model
  when set. Off by default.
- Each model call keeps its recorded purpose and cost; the model name is gateway-side configuration, never
  seen by the core.

## E5. Cost and usage
- `ModelInfo` carries optional `price_in_per_mtok` and `price_out_per_mtok`. OpenRouter returns prices in its
  list (stated in the audit); the exact field names and units: verify during implementation. Else user-entered
  per model in `[model.prices."<provider>/<model>"]` (proposed - confirm in review). If no price is known the
  status bar hides the cost instead of showing `$0.00`.
- `/usage`: tokens in/out per purpose for today and the budget left (from the existing daily budget); read from
  the same counters the valve budget uses - verify the source during implementation.

## E6. Provider errors
- Classify errors into `rate_limited` (HTTP 429, with `Retry-After` if present), `auth` (401/403), `timeout`,
  `other`. Show a notice with the provider's own message text and "see /log". Log under source `provider`.
  Current behaviour is unverified: write a test that records it first.

# lab-live-eval-hygiene Specification

## Purpose
TBD - created by archiving change lab-live-eval-hygiene. Update Purpose after archive.

## Requirements

### Requirement: When2Speak scorer normalizes SILENT speak payloads and records per-item outcomes
`When2SpeakScorer::score` SHALL treat a `pred.spoke == true` prediction whose trimmed uppercase `response_text` starts with `"SILENT"` or `">"` as `SILENT` (`predicted_speak = false`), and SHALL call `judge.record_outcome` for every scored prediction with `metric_name = "when2speak_turn"`, `score = 1.0` on match / `0.0` on mismatch, `passed = Some(hit)`, and `category = entry.category`.

#### Scenario: chat.speak("SILENT") is scored as SILENT and recorded
- **WHEN** `When2SpeakScorer::score` evaluates predictions where `pred.spoke == true` and `response_text` is `"SILENT"` (or `">"`) against `SILENT` and `SPEAK` gold entries
- **THEN** the `SILENT` gold item is counted as a true negative (`score = 1.0`, `passed = Some(true)`), the `SPEAK` gold item is counted as a false negative (`score = 0.0`, `passed = Some(false)`), and `ScorerDetailRecorder` contains both `when2speak_turn` outcomes

### Requirement: DeepSeek high-effort profile headroom and single EOF retry on truncated JSON output
`lab/profiles/deepseek-v4.1-flash-high.toml` SHALL configure `output_limits.reason = 8192`, and `drive_model_effects` in `crates/lab/src/datasets/framework.rs` SHALL retry `provider.invoke` at most once when an invocation fails with `ProviderFailureClass::InvalidOutput` containing `"EOF while parsing"` and `CostGuard` has remaining budget.

#### Scenario: Truncated JSON EOF recovers on single retry within budget
- **WHEN** `provider.invoke` fails once with `ProviderFailureClass::InvalidOutput` and detail containing `"EOF while parsing"` while `CostGuard` is not exhausted, and succeeds on the retry
- **THEN** the wake succeeds without recording a failed wake and without invoking the provider more than twice

### Requirement: Direct-answer ordering rule in `[ANCHOR]` output contract
`render_slot_0_anchor` in `crates/reason/src/render.rs` SHALL state in `output_contract` that when a user explicitly asks for a yes/no answer or a single option letter, that exact word or option letter is placed first at the start of `payload` before any explanation.

#### Scenario: Rendered `[ANCHOR]` includes direct-answer contract sentence
- **WHEN** `render_context_plan` renders a `ContextPlan`
- **THEN** the `[ANCHOR]` segment's `output_contract` includes the yes/no and single option letter placement instruction

### Requirement: Live benchmark runs auto-start EmbeddingGemma 2 across all suites
`run_benchmark_cmd` in `crates/lab/src/main.rs` SHALL enable `EmbeddingGemma 2` auto-start/configuration and set `require_semantic_embedder = true` whenever `--live` is enabled (including `--suite in_tree_only --only ...`), unless `--allow-no-embedder` or `--preflight-only` is passed, while leaving offline (`live == false`) `in_tree_only` runs unchanged.

#### Scenario: Live in_tree_only requires semantic embedder while offline in_tree_only does not
- **WHEN** benchmark embedder gating evaluates `live = true` with `suite = "in_tree_only"` (and neither `--allow-no-embedder` nor `--preflight-only`) vs `live = false` with `suite = "in_tree_only"`
- **THEN** semantic embedder requirement is `true` for the live run and `false` for the offline run

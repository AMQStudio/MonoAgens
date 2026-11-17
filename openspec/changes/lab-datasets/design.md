# Design

## Context

See `proposal.md` — Why. Current code, observed 2026-11-17:

- `crates/lab/src/benchmark.rs` holds `standard_benchmark_registry()`: currently 22 `BenchmarkDatasetSpec`
  rows (`tomi` optional, `tool_rag_scaling` in-tree). `verify_required_datasets_present` checks
  `is_dir()` only; `run_benchmark_suite` evaluates no dataset.
- `crates/protocol/src/facts.rs:256` (`VisibleScope::allows`) enforces `context.audience ⊆ item.audience`
  (when non-empty). Any probe whose audience contains an actor absent from ingested memory turns
  causes Core to hide those memories silently.
- `crates/lab/Cargo.toml` depends on `monoagens-core`, `monoagens-protocol`, `monoagens-reason`,
  and `monoagens-store` — it does **not** depend on `monoagens-gateway`. Semantic embedders
  (`EmbeddingGemma2Embedder`, `EmbeddingGemma300mEmbedder`, `OpenAiEmbedder`, plus test/fallback
  `HashEmbedder`, `NoneEmbedder`, `ScriptedEmbedder`) live in `crates/reason/src/embed/`. Note
  that local `EmbeddingGemma 2` weights are not shipped in the repo today.
- `crates/lab/src/driver.rs` and `guard.rs` enforce `CostGuard`. `crates/reason/src/model/openai.rs`
  provides the OpenAI-compatible HTTP client (`endpoint_url`, `store: false`, bearer secret).
- `lab/adapters/` is a Python project (`pyproject.toml`) for baseline frameworks and local scoring helpers.

Constraints: Lab default suites open no network and call no paid model (`core-and-lab` task 17.3).
`lab/scenarios/sealed/` is never read. Waist-2 (`monoagens.body.v1`) and Core privacy/cognitive rules
stay untouched. Only the owner authorizes `--live` runs and license clearances.

## Owner Design Decisions (2026-11-17 `/grill-me`)

1. **`unavailable` gaps kept visible:** `pis` (method on `pm_bench`), `proact` (unreleased), and
   `mpbench` (empty HF repo `pritamdash/mp-bench`) remain in manifest/scorecard with
   `availability = "unavailable"`. In-tree `VisibleScope` scenarios cover privacy/poisoning internally
   while noting the external Security corpus gap honestly.
2. **Corpus collision fixes:** Bind `ubuntu_irc` solely to `jkkummerfeld/irc-disentanglement`
   (`CC-BY-4.0`/`ISC`, derived `parent_speaker_p1`); split `dyntom` (`Apache-2.0`, `dyntom_acc`) out
   of `exploretom` (`exact_match_acc`), bringing `standard_benchmark_registry()` to **23 external
   entries** + 1 local `tool_rag_scaling`.
3. **Component embedder probes (`reason::Embedder`):** Rename from any "Waist-2/Gateway" wording to
   a component probe lane using `monoagens-reason`'s `Embedder` trait directly. Non-conversational
   `multilexnorm` (`ind` + `ind_eng`) and `indocollex` (`MIT`) are `not_mappable` for Core behavioral
   runs and execute as `embedder_probe` (`twin_retrieval_r1_256d`). Non-fixture runs reject
   `HashEmbedder`, `NoneEmbedder`, and `ScriptedEmbedder`, and emit `not_scored` if semantic weights/config
   are absent (no unconditional `"$0"` claim until local `EmbeddingGemma 2` weights are present).
4. **Honest `R-53` + `VisibleScope` subset rules (`lab_proctor` + `privacy_conflict`):** Standardize
   `+1,000 ms`/turn (`+60,000 ms`/stage for `dyntom`), `ActorId("prior_assistant")` for historical
   replies, `ActorId("stream_source")` at `+60,000 ms`/chunk for `memory_agent_bench`
   (`user`/`prior_assistant` on `LME(S*)`), Mode 1 dynamic `audience` on structured ToM, Mode 2
   `narrator` on prose ToM, AND enforce `probe.audience ⊆ evidence.audience` strictly without any
   `[soul]`-only loophole: use `[speaker_a, soul]` (`locomo`), `[user, soul]` (`longmemeval`, `stale`,
   `LME(S*)`), explicit `lab_proctor` observer in history + `[lab_proctor, soul]` probes (`molweni`,
   `ubuntu_irc`, `memory_agent_bench` non-LME, all ToM), and keep real channel privacy on
   `group_mem_bench` by flagging cross-channel items as `privacy_conflict` in `GoldBook` (scored
   separately for honest privacy refusal alongside raw benchmark accuracy), backed by mandatory
   per-dataset visibility tests.
5. **Machine-pinned manifest (`datasets pin`):** Add owner-run `monoagens-lab datasets pin [ids]` to
   resolve GitHub/HF commit SHAs, file sizes, and LFS hashes directly from APIs into
   `lab/datasets.manifest.toml`; Task 6.2 never hand-types or guesses SHAs/sizes.
6. **Aligned `R-52` metrics & `R-54` splits:** Align `benchmark.rs` metric names (`abstention_acc`,
   `implicit_policy_adaptation`, `false_alarm_rate`, `sdrt_link_f1`, `mhop_f1`), score `locomo` via
   official `evaluation.py` (retaining Cat 5), run `fantom` `All*` via `lab/adapters` local
   `SentenceTransformer`, lock default splits (`LongMemEval_S`, `CAPA` 20 test, `proactive_bench` 233
   test, `when2speak` `finetune_test_token` @ `S=1,000`, `pitch` 384 candidates, `fantom` `short`,
   `S=471` stratified on large sets), and require cluster-bootstrap CIs on clustered corpora.
7. **`--live` cost & judge control (zero Core changes):** Require scripted preflight cost/wake
   estimates + `--max-cost-usd` `CostGuard` caps before `--live`; load static memory/ToM histories
   through Core's single unmodified loop using Lab-side `ScriptedModelProvider` returning silent
   `think` (`$0` API cost, recording Gate wake counts and `history_reasoning_mode = "scripted_silent"`
   in `RunRecord`, with an optional `--live-full-history` override for future full-live experiments);
   and support configurable OpenAI-compatible judges (`gpt-4o`, `gpt-5`, `Gemini-3.1-flash-lite`) or
   force metric renaming on judge substitution.
8. **Scorecard hygiene & companion sync:** Strictly **1 row per dataset** (including `multilexnorm`/
   `indocollex`), aligned 6-value status set `{run, partial, not_scored, unavailable, not_mappable,
   unverified}`, explicit `contamination_risk` field, and aligned `core-and-lab/specs/lab/spec.md`.

## Goals / Non-Goals

**Goals:**
- Pinned, license-gated, checksum-verified external datasets with zero agent-guessed hashes.
- `VisibleScope`-verified `Converter -> (Vec<ExternalItem>, GoldBook)` and `reason::EmbedderProbe`
  lanes without modifying `monoagens-core` behavior or Waist-2 protocol contracts.
- Preflight-budgeted `--live` runs and strictly aligned official metrics with cluster CIs,
  contamination tags, and explicit gap reporting.

**Non-Goals:**
- Running external baseline harnesses (Mem0, Letta, ...) on external datasets in this change.
- Packaging or bundling local `EmbeddingGemma 2` weights inside the repo (owner supplies model path/provider).
- Altering `monoagens-core` cognitive rules or tuning `prior` from `ScenarioSet::External`.

## Decisions

### 1. One committed TOML manifest, synchronized with the aligned registry

`lab/datasets.manifest.toml` holds one table per external dataset: `id`, `domain`, `sources` (`kind`,
`location`, `revision`, `role` — supporting multi-source `group_mem_bench`), `license.spdx`,
`license.terms`, `license.status` (`pending` | `cleared` | `refused`), `availability` (`available` |
`unavailable` | `not_mappable`), `core_applicability`, `contamination_risk` (`very_low` | `low` |
`moderate` | `high` | `severe` + note), `reason`, `files` (path, sha256, size — unset as `pending_pin`
until owner runs `datasets pin` or `fetch --record-checksums`), `checksums_recorded` (`source` |
`owner` | `pending`, with date), `converter_version`, and `scorer_version`.

`standard_benchmark_registry()` holds **23 external datasets** + in-tree `tool_rag_scaling`, adding
`dyntom` separately from `exploretom` and binding `ubuntu_irc` solely to `jkkummerfeld/irc-disentanglement`.
Registry↔manifest id-set mismatch fails every manifest-reading command.

### 2. The owner clears licenses (`pending` / `cleared` / `refused`)

Run 3 recorded SPDX/terms, including `CC-BY-NC-4.0` datasets (`locomo`, `opentom`, `exploretom`, `tomi`)
and missing/ambiguous repo `LICENSE` files (`pm_bench`, `molweni`, `multilexnorm`, `fantom` data notice).
Only the owner sets `license.status`. Before clearing `CC-BY-NC-4.0` (especially `opentom`, which
forbids training and public playground logging) for `--live`, the owner verifies API privacy terms
(e.g., non-retention / `store: false` or local inference). `fetch` refuses non-`cleared` datasets.

### 3. Machine pinning (`datasets pin`) and atomic fetch (`datasets fetch`)

- `monoagens-lab datasets pin [ids]` (owner-run only): queries GitHub/Hugging Face APIs via blocking
  `reqwest` to resolve exact HEAD commit SHAs, file lists, byte sizes, and HF LFS SHA-256 pointers,
  writing them cleanly into `lab/datasets.manifest.toml` without downloading corpus data. Never
  hand-written by agents/workers.
- `monoagens-lab datasets fetch [ids]` (owner-run only): downloads cleared datasets into
  `lab/datasets/<path>/.partial/`, checks SHA-256 and size, extracts deterministically if needed,
  and renames atomically (with `--record-checksums` once for repos without upstream SHA-256).
- `lab/datasets/` is git-ignored. No other Lab command or default test opens network connections.

### 4. Behavioral Converters, `VisibleScope` Alignment, and `reason::EmbedderProbe`

Behavioral datasets implement `Converter -> (Vec<ExternalItem>, GoldBook)` and `Scorer` under
`crates/lab/src/datasets/` (`GoldBook` never crosses to `run_scenario`).

**Mandatory `VisibleScope` Probe-Subset Invariant (`crates/protocol/src/facts.rs:256`, `Absolute Line 2`):**
`VisibleScope::allows` requires `probe.audience ⊆ evidence.audience` (and `soul ∈ history.audience`
whenever the soul recalls history). Converters MUST satisfy this honestly without altering Core
privacy or using covert `[soul]`-only probe loopholes:
1. **`locomo`:** History has `[speaker_a, speaker_b, soul]`; probes come from `speaker_a` with
   `audience = [speaker_a, soul]`, so `{speaker_a, soul} ⊆ {speaker_a, speaker_b, soul}`.
2. **1-on-1 assistant histories (`longmemeval`, `stale`, `memory_agent_bench` `LME(S*)`):** History
   percepts set `audience = [user, prior_assistant, soul]` (`user` as user, past replies as
   `ActorId("prior_assistant")`). Question probe from `user` sets `audience = [user, soul]`, so
   `{user, soul} ⊆ {user, prior_assistant, soul}`.
3. **Observer / Proctor & ToM suites (`memory_agent_bench` non-LME `stream_source`, `molweni`,
   `ubuntu_irc`, Mode-1 ToM `fantom`/`hi_tom`/`exploretom`/`tomi`, Mode-2 prose ToM `opentom`/`dyntom`):**
   An explicit `ActorId("lab_proctor")` observer is included in every ingested turn's `audience`
   (`[..., lab_proctor, soul]`), and probes are sent by `lab_proctor` with `audience =
   [lab_proctor, soul]`. In Mode-1 ToM, dynamic room presence updates character occupants
   (`{occupants..., lab_proctor, soul}`), and Mode-2 prose ToM uses `actor = "narrator"`,
   `audience = [narrator, lab_proctor, soul]` — preserving `VisibleScope` isolation across places
   and unwitnessed secrets without inventing character exits or loosening `Absolute Line 2`.
4. **Real multi-channel privacy (`group_mem_bench`):** History in each `channel_k` preserves real
   channel membership (`audience = channel_k_speakers + [soul]`, no omniscient `lab_proctor`), and
   each question uses its real `User_<n>` asker (`audience = [asker, soul]`). Any item whose gold
   evidence spans another channel lacking `asker` MUST be tagged `privacy_conflict = true` in
   `GoldBook` and scored separately (`non_conflict_acc`, `privacy_conflict_refusal_rate`,
   `privacy_conflict_count`, alongside raw benchmark accuracy) because refusing cross-channel secrets
   to unauthorized askers is required Core behavior, not a recall failure.
5. **Mandatory test:** Every behavioral dataset fixture test MUST assert `scope.allows(...) == true`
   for all non-conflict gold evidence `EventId`s under that dataset's probe `VisibleScope`, and on
   `group_mem_bench` MUST additionally assert `scope.allows(...) == false` for `privacy_conflict`
   cross-channel evidence!

Additional `R-53` rules: monotonic `+1,000 ms`/turn virtual clock where per-turn wall-clock ms are
missing (`+60,000 ms`/stage on `dyntom`; `+60,000 ms`/chunk `ActorId("stream_source")` on
`memory_agent_bench` stripping `"Please memorize it..."`); cloned post-ingestion checkpoints for
multi-probe items.

**Component `EmbedderProbe` lane (`monoagens-reason::Embedder`):**
Non-conversational `multilexnorm` (`ind` + `ind_eng`) and `indocollex` implement `EmbedderProbe`
calling `monoagens-reason::Embedder` directly (zero `gateway` dependency, zero Waist-2 protocol
involvement, zero invented `Percept`s). Non-fixture runs MUST reject `HashEmbedder`, `NoneEmbedder`,
and `ScriptedEmbedder` with an explicit error, and mark the dataset `not_scored` (`embedder_unconfigured`)
when local semantic model weights (`EmbeddingGemma 2` or configured semantic embedder) are absent.

### 5. Aligned `R-52` Scoring & Configurable Multi-Provider Judges

Scorers align to `R-52` (Rust + parity fixtures or pinned `lab/adapters` Python scripts):
- **Zero-API-judge datasets:** `locomo` (official `evaluation.py` Porter-F1 + Cat-5 substring refusal,
  retaining Cat 5), `pm_bench` (`set_f1`, `false_alarm_rate`), `pitch`, `when2speak`, `ubuntu_irc`
  (`thread_f1`, derived `parent_speaker_p1`), `molweni` (`qa_f1`, `sdrt_link_f1`), `fantom`
  (`eval_fantom.py` local `SentenceTransformer` via `lab/adapters`), `opentom` (`Loc_c`/`Loc_f`/`Att`/`MHop`),
  `hi_tom`, `exploretom` (`exact_match_acc`), `dyntom` (`dyntom_acc`), `tomi` (`shortcut_perturbation_acc`),
  `proactive_bench` (`TGL` trigger parity + optional local RM), and `EmbedderProbe` (`twin_retrieval_r1_256d`).
- **LLM-judged datasets (`--live`):** `longmemeval` (`gpt-4o`, `overall_acc` + `abstention_acc`),
  `memory_agent_bench` `LME(S*)` (`gpt-4o`), `group_mem_bench` (`gpt-5`), `stale` (`Gemini-3.1-flash-lite`,
  `SR`/`PR`/`IPA`), `capa` (`GPT-4o` schema judge via `capa/judge.py`). Official judges are configured
  via OpenAI-compatible provider entries (explicit `endpoint_url`, `model`, bearer secret) so any
  model provider can serve behind `OpenAiModelProvider`. If the configured judge model differs from
  the benchmark's official judge id, the Lab MUST either refuse the official metric or automatically
  emit it under `<metric>_non_official_judge` alongside judge model & prompt version.

### 6. `ScenarioSet::External`, `R-54` Defaults & Cluster-Bootstrap CIs

`ScenarioSet::External` is rejected by all calibration paths. Default splits (`LongMemEval_S`, `CAPA`
20 test meetings, `proactive_bench` 233 test, `when2speak` `finetune_test_token.jsonl` @ `S=1,000`,
`pitch` 384 candidates, `fantom` `short`, `S=471` stratified on large sets) and cluster-bootstrap CIs
apply across all clustered sets (`locomo`, `memory_agent_bench`, `group_mem_bench`, `capa`, `pitch`,
`fantom`, `molweni` MRC, `hi_tom`, `dyntom`).

### 7. One-Row Scorecard, `--live` Preflight Cost Guard & Lab-Side Scripted History Loading

- **One row per dataset:** Every registered external dataset yields **exactly 1 row** in the
  scorecard (`lane ∈ {core_behavioral, embedder_probe}`), carrying `status ∈ {run, partial,
  not_scored, unavailable, not_mappable, unverified}` and `contamination_risk`. (`multilexnorm` and
  `indocollex` occupy 1 row each in `embedder_probe` with `core_applicability = "not_mappable"`.)
- **`--live` Cost Guard & Lab-Side Scripted History Loading:**
  - Before any `--live` external dataset starts, `monoagens-lab benchmark --live` runs a deterministic
    **scripted preflight simulation** over the selected sample to print estimated history percepts,
    simulated Gate wake count, probe calls, judge calls, and worst-case USD cost, requiring an
    explicit `--max-cost-usd` (`CostGuard`) budget cap that stops the run cleanly if exceeded.
  - Static history ingestion for memory and ToM QA datasets runs Core's **single unmodified loop**
    while the **Lab driver** supplies a `ScriptedModelProvider` returning silent `think` (zero
    outbound replies, `$0` API cost) on historical turns and logs simulated Gate wakes plus
    `history_reasoning_mode = "scripted_silent"` in `RunRecord`, switching to the live `ModelProvider`
    only on evaluation probes (or an optional future `--live-full-history` override). Interactive and
    timing benchmarks (`pitch`, `when2speak`, `capa`, `proactive_bench`, `pm_bench`) run normal
    evaluation at their decision points.

## Migration Plan

Additive after `core-and-lab`. Companion `core-and-lab/specs/lab/spec.md` aligns its `--suite standard`
scenarios concurrently. Rollback removes `datasets` subcommand and `lab/datasets.manifest.toml`.

## Open Questions

None.

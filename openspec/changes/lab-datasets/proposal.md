# Proposal

## Why

`core-and-lab` requires `monoagens-lab benchmark --suite standard` to score MonoAgens on public
benchmarks for memory, prospective memory, timing, group chat, theory of mind, Indonesian
register and security. None of that can run today:

- `lab/datasets/` does not exist.
- `crates/lab/src/benchmark.rs:74-266` registers 22 external datasets (21 required, plus `tomi`
  as optional) by name and folder only. Nothing records where each one comes from, which
  version, under which license, or what its files should hash to.
- `verify_required_datasets_present` checks only that the folders exist (`path.is_dir()`).
- Even when every folder exists, `run_benchmark_suite` scores nothing. It returns the registry,
  Tool RAG scaling and the `Arm T` restart check, with no per-dataset result.
- Run 3 (`research/findings/2026-11-17-run-3.md`, `R-51`–`R-54`) verified that 3 registered IDs
  have no public dataset (`pis`, `proact`, `mpbench` — leaving zero external Security corpus until
  `mpbench` releases), `ubuntu_irc` and `exploretom` mixed metrics from separate corpora (`dyntom`
  vs `ExploreToM`), several registry metric names diverge from official protocols, and
  `multilexnorm`/`indocollex` are non-conversational register/lexicon probes rather than behavioral
  agent scenarios.

So the Lab's benchmark claims cannot be produced, reproduced or audited. This change supplies
what is missing: datasets that are machine-pinned and verified, honest conversion into MonoAgens
percepts satisfying `VisibleScope` subset rules or `reason` embedder probes, cost-guarded `--live`
execution, aligned registry metrics, and per-dataset scoring.

## What Changes

- Add a committed dataset manifest (`lab/datasets.manifest.toml`) recording per dataset: source(s),
  pinned revision, availability, contamination risk, license and redistribution terms, license
  status, file list with SHA-256 and size, and converter/probe and scorer versions (`R-51`).
  - Add `monoagens-lab datasets pin [ids]` (owner-run, querying GitHub/HF APIs directly) so commit
    SHAs and file sizes are never hand-typed or guessed by agents.
  - Mark unreleased datasets (`pis`, `proact`, `mpbench`) as `availability = "unavailable"` so they
    remain visible scorecard gaps without silent substitution.
- Align `crates/lab/src/benchmark.rs` (`standard_benchmark_registry()`) from 22 to **23 external
  entries** (plus in-tree `tool_rag_scaling`):
  - Split `dyntom` (`GAIR-NLP/DynToM`, `Apache-2.0`, `dyntom_acc`) out of `exploretom` (`exact_match_acc`).
  - Bind `ubuntu_irc` solely to `jkkummerfeld/irc-disentanglement` (`CC-BY-4.0`/`ISC`, `thread_f1` +
    non-official derived `parent_speaker_p1`).
  - Align metric names and coverage to official protocols (`longmemeval` `abstention_acc`, `stale`
    `implicit_policy_adaptation`, `pm_bench` `false_alarm_rate`, `molweni` `sdrt_link_f1`, `opentom`
    `mhop_f1`, and `multilexnorm`/`indocollex` `twin_retrieval_r1_256d`).
- Keep dataset files out of git, under a git-ignored `lab/datasets/`, filled only by an explicit
  owner-run `datasets fetch` command (`license.status = "cleared"` required).
- Make `benchmark --suite standard` refuse to run unless every required, available dataset verifies,
  and add `--only <ids>` (`partial` report).
- Add one converter and scorer per behavioral dataset (`Converter -> (Vec<ExternalItem>, GoldBook)`
  and `Scorer`) enforcing Run 3's `R-53` honest percept rules AND `VisibleScope::allows` subset
  compatibility (`probe.audience ⊆ evidence.audience`), verified by mandatory per-dataset visibility
  tests so privacy gates never silently hide benchmark answers.
- Route non-conversational register/lexicon datasets (`multilexnorm` `ind` + `ind_eng`, `indocollex`)
  to a dedicated **component embedder probe lane** using `monoagens-reason`'s `Embedder` trait
  directly (requiring real semantic weights — rejecting `hash`, `none`, and `scripted` outside tests
  — and reporting `not_scored` when local `EmbeddingGemma 2` weights are not installed), while keeping
  strictly **one scorecard row per dataset**.
- Guard `--live` execution with a **scripted preflight cost/wake estimate**, explicit `CostGuard`
  (`--max-cost-usd`) budget caps, Lab-side `ScriptedModelProvider` silent reasoning during static
  memory/ToM history loading (zero Core changes, `$0` history cost, logged in `RunRecord`), and
  configurable OpenAI-compatible judges (strictly renaming any metric if an official judge model
  `gpt-4o`/`gpt-5`/`Gemini-3.1-flash-lite` is substituted).
- Record `contamination_risk` per dataset, treat external datasets strictly as `ScenarioSet::External`
  (never tuning `prior`), and keep default tests 100% offline using tiny committed synthetic fixtures.

## Capabilities

### New Capabilities

- `benchmark-datasets`: the dataset manifest, machine pinning (`datasets pin`), the license gate,
  fetching outside git, checksum and version verification, and `monoagens-lab datasets` commands.
- `benchmark-evaluation`: converting verified datasets into honest `VisibleScope`-compatible
  percepts and gold answers (plus `reason` embedder probes for register suites), aligned scoring
  with cluster-aware intervals, `--live` cost/judge governance, the `external` set rule, and the
  one-row-per-dataset scorecard with contamination tags.

### Modified Capabilities

None in `openspec/specs/` (empty). Note: the companion delta `core-and-lab/specs/lab/spec.md` is
updated concurrently to keep `PIS`/`ProACT`/`MPBench` `unavailable` gaps, `DynToM`, and `reason`
embedder probes consistent across both open changes.

## Impact

- **Code**: `crates/lab` gains modules for manifest, API pinning (`datasets pin`), multi-source
  fetching/checksums, converters, `EmbedderProbe` (using `monoagens-reason::Embedder`, zero `gateway`
  dependency), scorers, cluster-bootstrap stats, preflight cost estimation, and the scorecard.
  `crates/lab/src/benchmark.rs` updates `standard_benchmark_registry()`,
  `verify_required_datasets_present`, and `run_benchmark_suite`; `crates/lab/src/main.rs` gains
  `datasets`; `lab/adapters/` hosts local Python helpers (`fantom` `SentenceTransformer`, `capa` judge,
  optional `proactive_bench` RM).
- **Files**: `lab/datasets.manifest.toml`, git-ignored `lab/datasets/`, committed fixtures under
  `crates/lab/tests/fixtures/datasets/`, `docs/lab-datasets.md`, updated `core-and-lab/specs/lab/spec.md`,
  and completed Run 3 findings at `research/findings/2026-11-17-run-3.md` + `run-3-notes/`.
- **Dependencies**: `sha2` (already in `lab`), blocking `reqwest` (already in workspace via `reason`)
  for `datasets pin`/`fetch` only, plus archive/Parquet parsing as required.
- **Network and cost**: only owner-run `datasets pin` and `datasets fetch` touch the network.
  Paid `--live` calls run only with explicit authorization, scripted preflight estimate, and
  `--max-cost-usd`. Note: local `EmbeddingGemma 2` weights must be supplied/downloaded separately to
  run local semantic embedder probes.
- **Order**: applies after `core-and-lab`. Groups 1–7 are unblocked by Run 3 and `/grill-me`
  rulings; Group 8 (`pin`, `cleared`/`refused`, `fetch`, `--live`) stays owner-only.

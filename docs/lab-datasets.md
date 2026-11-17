# MonoAgens Lab External Datasets Workflow

This guide documents the provenance-pinned, license-gated workflow for external benchmark datasets in `monoagens-lab`, aligned with [`openspec/changes/lab-datasets/`](../openspec/changes/lab-datasets/proposal.md) and [`research/findings/2026-11-17-run-3.md`](../research/findings/2026-11-17-run-3.md).

## 1. Overview & Two Execution Lanes

Every registered external dataset is defined in the committed template [`lab/datasets.manifest.example.toml`](../lab/datasets.manifest.example.toml) (where every `license.status` is `"pending"` and no owner-recorded checksums are published). On each developer/owner machine, `monoagens-lab datasets init` creates the local, git-ignored working manifest `lab/datasets.manifest.toml`. Neither `lab/datasets.manifest.toml` nor downloaded files under `lab/datasets/` are ever committed to git; back up `lab/datasets.manifest.toml` outside git once you clear licenses or record checksums. Datasets run in one of two lanes and emit **strictly one scorecard row per registered external dataset**:

1. **`core_behavioral` (`Converter -> Core -> Scorer`)**:
   - Converts verified raw corpus files into 6-fact `Percept`s (`ExternalItem`) and a compile-time-isolated `GoldBook`.
   - Enforces `VisibleScope::allows` (`probe.audience ⊆ evidence.audience`), monotonic virtual timestamps (`+1,000 ms`/turn, `+60,000 ms`/stage on `dyntom`, `+60,000 ms`/chunk on `memory_agent_bench`), `prior_assistant` for historical assistant replies, `lab_proctor` observers where needed, and `privacy_conflict` separation on `group_mem_bench`.
   - Static history loading runs Core's unmodified loop with Lab-side `ScriptedModelProvider` silent `think` (`$0` API cost, recorded as `history_reasoning_mode = "scripted_silent"` in `RunRecord`, with `--live-full-history` override for future full-live experiments).
2. **`embedder_probe` (`monoagens-reason::Embedder`)**:
   - Non-conversational Indonesian register/lexicon datasets (`multilexnorm`, `indocollex`) have `core_applicability = "not_mappable"` and execute directly against `monoagens_reason::Embedder` (`twin_retrieval_r1_256d`).
   - Rejects `HashEmbedder`, `NoneEmbedder`, and `ScriptedEmbedder` outside unit tests, and reports `status = "not_scored"` (`embedder_unconfigured`) when local `EmbeddingGemma 2` weights are absent.

### `unavailable` Scorecard Gaps

Three registered IDs have no public dataset release (`pis`, `proact`, `mpbench`) and remain explicitly marked `availability = "unavailable"` in the manifest. They stay visible in the scorecard with `status = "unavailable"` and are never silently substituted.

## 2. Step-by-Step Owner Workflow

### Step 1: Initialize the local manifest (`datasets init`)

```sh
monoagens-lab datasets init [--manifest lab/datasets.manifest.toml] [--template lab/datasets.manifest.example.toml] [--json]
```

Copies the committed template [`lab/datasets.manifest.example.toml`](../lab/datasets.manifest.example.toml) to the local, git-ignored `lab/datasets.manifest.toml`. Refuses non-zero without overwriting if `lab/datasets.manifest.toml` already exists (`--force` is not allowed). Every Lab command that needs the manifest fails non-zero and tells you to run `monoagens-lab datasets init` when the local file is absent.

> **Backup reminder:** `lab/datasets.manifest.toml` is git-ignored and is never committed. Back it up outside git after clearing licenses or recording owner checksums.

### Step 2: Inspect manifest & clear licenses locally (`datasets list`)

```sh
monoagens-lab datasets list [--manifest lab/datasets.manifest.toml] [--datasets-dir lab/datasets] [--json]
```

Displays each dataset's sources (including multi-source `group_mem_bench`), `license.spdx`, `license.status`, `availability`, `contamination_risk`, `verification_state`, `pending_pin` items, and missing owner checksums. Never opens a network connection.

Review terms and edit `license.status` locally in `lab/datasets.manifest.toml` (`pending` → `cleared` or `refused`):
- **`CC-BY-NC-4.0`** (`locomo`, `opentom`, `exploretom`, `tomi`): non-commercial only. `opentom` additionally forbids LLM training/fine-tuning and submitting items to public playgrounds that harvest inputs — verify non-retention (`store: false` or local inference) before clearing.
- **Missing repo `LICENSE` files** (`pm_bench`, `molweni`, `multilexnorm`): remain `pending` until the owner obtains author clearance or sets `refused`.

### Step 3: Pin upstream revisions and sizes (`datasets pin`, owner-run)

```sh
monoagens-lab datasets pin [ids...] [--manifest lab/datasets.manifest.toml] [--json]
```

Queries GitHub/Hugging Face APIs read-only to resolve commit SHAs, byte sizes, and available HF LFS SHA-256 pointers into `lab/datasets.manifest.toml` without downloading corpus files or changing `license.status`.

### Step 4: Fetch cleared datasets (`datasets fetch`, owner-run)

```sh
monoagens-lab datasets fetch [ids...] [--manifest lab/datasets.manifest.toml] [--datasets-dir lab/datasets] [--record-checksums] [--json]
```

- Refuses any dataset whose `license.status` is not `cleared`.
- Downloads into `lab/datasets/<relative_dir>/.partial/`, verifies size + SHA-256, and promotes atomically (cleaning `.partial/` on any interruption or mismatch).
- For upstream repositories without published SHA-256 manifests, pass `--record-checksums` once (trust on first use, recording hashes and fetch date); subsequent `--record-checksums` runs refuse and direct you to `datasets verify`.

### Step 5: Verify local files & converter cache (`datasets verify`)

```sh
monoagens-lab datasets verify [ids...] [--manifest lab/datasets.manifest.toml] [--datasets-dir lab/datasets] [--json]
```

Checks SHA-256 and byte size of every file, the extraction marker, and the cached `converter_version`. Exits non-zero on any mismatch (printing both expected and actual hashes) and opens zero network connections.

### Step 6: Run offline or preflight-gated `--live` benchmarks

```sh
# Offline deterministic / in-tree verification (never calls paid APIs)
monoagens-lab benchmark --suite in_tree_only --pareto --json

# Subset run (always marked partial, listing left-out datasets)
monoagens-lab benchmark --suite standard --only locomo,when2speak --out-dir lab/runs/pilot --json

# Live run (requires explicit --max-cost-usd cap; runs scripted preflight estimate first)
monoagens-lab benchmark --suite standard \
  --live --max-cost-usd 5.00 \
  --model-endpoint https://api.example.com/v1/chat/completions --model gpt-4o --model-api-key-env OPENAI_API_KEY \
  --judge-endpoint https://api.example.com/v1/chat/completions --judge-model gpt-4o --judge-api-key-env OPENAI_API_KEY \
  --judge-substitution rename \
  --out-dir lab/runs/live-run-1 --pareto --json
```

Notes:
- `--live` requires `--max-cost-usd <cap>` and always runs a deterministic scripted preflight simulation first (reporting `total_history_percepts`, `total_estimated_history_wakes`, `total_estimated_probe_calls`, `total_estimated_judge_calls`, and `total_worst_case_usd`), aborting before any paid call if worst-case spend exceeds the cap.
- Substituting a non-official judge model for `gpt-4o`, `gpt-5`, or `Gemini-3.1-flash-lite` either renames metrics to `<metric>_non_official_judge` (`--judge-substitution rename`) or fails (`--judge-substitution fail`).
- `ScenarioSet::External` is strictly rejected by all calibration paths (`external` results never tune the prior).
- Never read `lab/scenarios/sealed/` and never write to `lab/runs/reference/`.

## 3. Live provider options and the standard report (owner request `2026-11-17`)

`benchmark --live` accepts the OpenAI-compatible provider options below. Values are recorded in run records; header **names** are recorded, never values.

| Flag | Effect |
|---|---|
| `--reasoning-effort low\|medium\|high\|max` | Sent as `reasoning_effort` on every soul request. OpenCode Go's `deepseek-v4.1-flash` advertises `low`, `high`, `max`. |
| `--judge-reasoning-effort low\|medium\|high\|max` | Same, for every judge request. |
| `--extra-header NAME=ENV_VAR` (repeatable) | Reads the value from the environment and sends it on every soul/judge request (e.g. `--extra-header x-opencode-session=OPENCODE_SESSION_ID`). |
| `--embed-endpoint URL` | Uses an already-running EmbeddingGemma 2 sidecar instead of spawning the pinned one. |
| `--embed-model-path PATH` | Overrides the manifest-resolved embedder weights directory. |
| `--allow-no-embedder` | Offline-only escape hatch; a `--suite standard` run without a semantic embedder otherwise refuses to start. |
| `--preflight-only` | Runs conversion + scripted cost preflight and writes `preflight.json`, without any paid call. |
| `--pilot-items N` | Truncates the seeded R-54 sample to its first N items (pilot gate). |
| `--report` | Generates the standard report into `<out-dir>/report` after the run. |
| `--report-note TEXT` (repeatable) | Adds an operator note to the report's caveats section. |

OpenCode Go (official docs: <https://opencode.ai/docs/console/go> and <https://opencode.ai/docs/console/harnesses>):

- Base URL `https://opencode.ai/inference/go`, OpenAI Chat Completions path `/openai/v1/chat/completions`.
- Model ID `deepseek-v4.1-flash` (live list: `GET https://opencode.ai/inference/go/v1/models`).
- Go **requires** a stable `x-opencode-session` header per conversation and a client-specific `User-Agent` (the Lab sends `monoagens-lab/<version>`).
- Usage reports `opencode.list_cost` and `opencode.allowance` per response.

### Report generation

```sh
# after a run (or any time later)
monoagens-lab report --run lab/runs/<run-id> [--out <dir>] [--note "..."] [--no-python-charts] [--json]
```

Every value is derived from the persisted run records and `*.outcomes.json` files: `REPORT.md`, self-contained `index.html`, `tables/*.csv`, `charts/*`, `transcripts/<dataset>/<item_id>.{jsonl,md}` for failure examples, and the reproducibility block (commit, prior digest, manifest digest, provider host, efforts, judge substitution, seeds/splits, embedder, `history_reasoning_mode`).

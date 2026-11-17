# Design

## Context

See `proposal.md` for why. Current state that shapes the approach:

- `crates/lab/src/datasets/framework.rs` drives external items through the core. In live probe
  calls it replaces the core's rendered plan with `build_probe_soul_prompt` (whole history +
  question) and serializes `plan.sections` as JSON in other calls instead of using
  `monoagens_reason::render_context_plan_with` and `parse_thought_output`. History is loaded with
  `ScriptedModelProvider` (`scripted_silent`). The `lab-datasets` owner revision of 2026-11-18
  (Group 11) replaces both with the production render/parse path and live history.
- `crates/reason/src/model/openai.rs` supports `reasoning_effort`, extra headers, User-Agent,
  retries and telemetry, but reads cached tokens only from
  `usage.prompt_tokens_details.cached_tokens`; OpenCode reports them in
  `usage.opencode.tokens.cache_read`, with `opencode.tokens.input` excluding cache reads (Run 4,
  `R-58`). `lab-datasets` 11.4 fixes this.
- OpenCode Go, `deepseek-v4.1-flash` (Run 4, `R-58`): $0.15 / M uncached input, $0.003 / M cached
  read, $0.60 / M output (reasoning included in output); ×2 on Mon–Fri 01:00–04:00 and
  06:00–10:00 UTC; Go allowance $100 / month for this model, at most 20 % per 5 hours and 50 % per
  week (reset Monday 00:00 UTC); paid requests are blocked at a limit unless Extra Usage is on.
  `x-opencode-session` must be stable per conversation for routing and KV-cache reuse.
- Published protocols (Run 4, `R-55`; verbatim texts in
  `research/findings/run-4-notes/R55_verbatim_prompts_and_tables.md`): every published LoCoMo
  comparison scores categories 1–4 (1,540 questions) with the `ACCURACY_PROMPT` judge
  (`CORRECT`/`WRONG` JSON); official LoCoMo scores 1–4 with token F1 and category 5 with a refusal
  substring check; LongMemEval-S uses five task-specific yes/no judge prompts.
- Measured data sizes: LoCoMo 10 conversations, 5,882 turns, ~22 k tokens per conversation,
  1,986 questions (446 in category 5); LongMemEval-S 500 histories of ~517 turns (~112 k tokens).
- `lab/adapters/{mem0,letta,zep_graphiti,langgraph_langmem}` are skeletons (head-to-head only).
- Machine: 14 GiB RAM, 8 GB VRAM laptop; `/tmp` is tmpfs; one cargo command at a time.

## Goals / Non-Goals

**Goals:**
- Phase 0: a clean, cheap baseline of the full MonoAgens on LoCoMo and LongMemEval-S.
- One token ledger for every LLM call, ingest and answer alike, with cached tokens.
- Later, a fair head-to-head under the same rules.

**Non-Goals:**
- Showing or matching published scores, or running published answer/judge models (gpt-4o-mini,
  gpt-4o, gpt-5).
- Full-transcript or no-memory arms; other benchmark categories; BEAM, MemoryAgentBench, STALE or
  GroupMemBench.
- Tuning MonoAgens on these results in this change; how the benchmarks may be used for improving
  memory is a later owner decision.

## Decisions

### 1. One local metering proxy in front of OpenCode Go
MonoAgens, the judge and (later) every framework send LLM calls to a Lab-local OpenAI-compatible
proxy that forwards to OpenCode Go. The proxy sets the model, `reasoning_effort = "high"`, the
User-Agent and the per-(participant, conversation) `x-opencode-session`, and writes one ledger
line per call: participant, dataset, conversation, phase, uncached input, cached read, output,
reasoning tokens, latency, list cost, allowance cost, status. It reads `usage.opencode.tokens`
(falling back to `prompt_tokens_details.cached_tokens`). Header values and keys are never written.
*Why:* frameworks make internal calls the Lab cannot see (Run 4: Mem0 cannot set headers;
Graphiti's generic client does not send `reasoning_effort`); one proxy meters all identically and
enforces parity in one place. For phase 0 the proxy is used even though MonoAgens' own provider
could set the headers, so phase 0 and head-to-head share one ledger format.

### 2. Question wording and answer-format instruction
Each item is asked with its benchmark question plus one versioned answer-format instruction per
benchmark, taken from the published answer prompts (LoCoMo: answer briefly and turn relative time
references into absolute dates; LongMemEval-S: answer briefly and say so if the information is not
available). MonoAgens receives this as the text of the question percept, addressed to it, and
answers through its own rendered prompt and a dispatched reply; its prompt is never replaced.
Frameworks (later) place the same text in one shared answer template over their own recalled
context.

### 3. Judge
The judge is `deepseek-v4.1-flash`, effort `high`, through the proxy, running the published prompt
texts verbatim: LoCoMo `ACCURACY_PROMPT` with JSON verdict; LongMemEval-S's five task-specific
prompts (standard, temporal with off-by-one tolerance, knowledge-update, preference rubric,
abstention). Metrics are named `j_deepseek` (LoCoMo) and `acc_j_deepseek` (LongMemEval-S), with
the prompt version recorded. LoCoMo official token F1 and the category 5 refusal check are
computed locally without a model. An unparsable verdict is retried once, then recorded as a judge
failure, never as correct.

### 4. Phase 0 scheduling and cache
LoCoMo conversations and LongMemEval-S histories run one at a time per process, start to finish,
each with its own stable session id, so the core's static prompt prefix stays warm in the provider
cache. The runner resumes per conversation or history, warns before starting inside a peak-hour
window, backs off on rate limits, and stops cleanly when an allowance window is exhausted
(Extra Usage stays the owner's choice). Raw token counts are the efficiency metric; costs are
shown with and without cache.

### 5. Pilot
Phase 0 pilot = the first LoCoMo conversation by sorted sample id (all its questions) and 20
LongMemEval-S questions chosen by a seeded rule stratified by question type, which are also part of
the full run. It stops with measured tokens per call, cached share, silence rate, errors and the
projected cost of the rest.

### 6. Head-to-head (later, owner-gated)
The adapters gain `ingest`, `answer` and `reset`; frameworks use the proxy and, where possible,
EmbeddingGemma 2 through an OpenAI-compatible `/v1/embeddings` route (Mem0, LangMem and Graphiti
accept 256 dimensions; Letta unverified). Open owner choices from Run 4 are decided before that
work starts: Letta runtime (deprecated V1 Docker vs Node App Server vs gap), Graphiti database
(FalkorDB/falkordblite vs Neo4j), and each framework's ingestion unit (`R-57`). LongMemEval-S
head-to-head may use an owner-set seeded subset, identical across participants.

## Risks / Trade-offs

- [Our judge differs from published judges] → own metric names; no published numbers shown; the
  claim made is only about systems run under the same rules.
- [DeepSeek reasoning makes output larger than estimated] → measured in the pilot before the rest.
- [Go allowance windows pause the run] → resumable per conversation; peak-hour warning.
- [Full MonoAgens is slow on long histories (LongMemEval-S ~517 turns each)] → measured in the
  pilot; histories are independent and may run in a few parallel processes within rate limits.
- [A framework cannot use the proxy or shared embedder (head-to-head)] → gap or recorded parity
  deviation; never silently replaced.

## Migration Plan

Additive. Applies after `lab-datasets` Group 11. Rollback: the suite is opt-in
(`--suite memory-comparison`).

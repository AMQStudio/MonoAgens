# Tasks — `lab-memory-comparison`

Applies after `lab-datasets` Group 11 (full-MonoAgens rule, per-category suites, cached-token
telemetry). Groups 1–3 serve phase 0 and are done first; Group 5 (head-to-head) waits for the
owner's decision after memory has been improved. Machine rules for every worker: one cargo
command at a time, never build in `/tmp`, targeted tests while iterating. Default tests stay
offline; never read `lab/scenarios/sealed/`. Facts: `research/findings/2026-11-18-run-4.md` and
`run-4-notes/`.

## 1. Metering proxy

- [x] 1.1 Add a Lab-local OpenAI-compatible metering proxy (Design 1) forwarding to OpenCode Go: sets model, `reasoning_effort`, User-Agent and a per-(participant, conversation) `x-opencode-session` (value never logged), retries transient errors, backs off on rate limits, and writes one ledger line per call (participant, dataset, conversation, phase, uncached input, cached read from `usage.opencode.tokens.cache_read` with fallback to `prompt_tokens_details.cached_tokens`, output, reasoning, latency, list and allowance cost, status). Verify with a loopback mock provider test: headers and effort reach upstream, both usage shapes are parsed, ledger lines carry attribution, and no header value or key appears in any written file.
- [x] 1.2 Route MonoAgens' live provider and the judge through the proxy. Verify with a mock-provider run where MonoAgens and the judge make calls and the ledger totals equal the run record totals.

## 2. Phase 0 suite, judge and scoring

- [x] 2.1 Add `benchmark --suite memory-comparison --phase 0`: full MonoAgens only, LoCoMo all 10 conversations and LongMemEval-S all 500 questions, versioned question wording and answer-format instruction per benchmark (Design 2), one conversation/history at a time with its own session id, resumable, peak-hour warning, clean stop on allowance limits, `development_exposure` recorded per benchmark. Verify with a fixture run using the mock provider, including a resume after an interrupted conversation.
- [x] 2.2 Add the judge (Design 3): LoCoMo `ACCURACY_PROMPT` and the five LongMemEval-S prompts copied verbatim from `run-4-notes/R55_verbatim_prompts_and_tables.md`, prompt version recorded, one retry on an unparsable verdict then a recorded judge failure. Verify with mock-judge tests for each prompt type and for an unparsable verdict.
- [x] 2.3 Score: LoCoMo token F1 (official) and `judge_acc` per category, categories 1–4 headline, category 5 with the official refusal check; LongMemEval-S `judge_acc` per question type with abstention apart; silence rate; cluster-bootstrap 95 % intervals by conversation. Verify on fixtures with known answers, including a silent answer.
- [x] 2.4 Add the phase 0 pilot stop (Design 5): first LoCoMo conversation + 20 seeded stratified LongMemEval-S questions, then exit with measured tokens per call, cached share, silence rate, errors and projected cost of the rest. Verify with a fixture run that the pilot ids are a subset of the full-run ids and the run stops.

## 3. Memory report

- [x] 3.1 Add the memory-only report (spec "The memory report shows quality against token cost"): setup block, results per dataset and category, quality-vs-tokens chart, ingest/answer token split with cached tokens, calls, latency p50/p95, silence rate, list and allowance cost with and without cache, failure examples with transcripts, all from run records, and no published scores. Verify against a fixture run directory that every table value equals its record value.

## 4. Owner-gated live phase 0

Owner approval (2026-11-18): phase 0 is approved in full, one round, to run straight after Groups 1–3 — the pilot still runs first and the coordinator checks it for sanity (no override of MonoAgens' prompt, answers parse, cached tokens recorded, cost in line with the estimate) before continuing without a further owner stop. Deliverable: a complete memory report for MonoAgens alone (per-category scores with intervals, F1 and judge metrics, token and cost tables, charts, failure examples, transcripts), in the style of published memory-benchmark papers but without other systems' numbers.

- [ ] 4.1 Owner-approved live phase 0 pilot (DeepSeek v4.1 Flash, effort `high`, outside peak hours); deliver the pilot report and stop.
- [ ] 4.2 After owner approval: the rest of phase 0 with `--max-cost-usd`; deliver the phase 0 memory report.

## 5. Head-to-head (owner decision required before starting)

- [ ] 5.1 Owner decides the open Run 4 choices: Letta runtime, Graphiti database, ingestion unit per framework, LongMemEval-S item count. Record them in `design.md` Decision 6.
- [ ] 5.2 Extend the NDJSON adapter protocol with `ingest`, `answer` and `reset` in `lab/adapters/common.py` and `crates/lab/src/baseline.rs`, keeping virtual time, oracle stripping and sealed-mode checks. Verify with protocol conformance tests on a fake adapter.
- [ ] 5.3 Add an OpenAI-compatible `/v1/embeddings` route to the EmbeddingGemma 2 sidecar. Verify with a test that returns 256-dimension vectors.
- [ ] 5.4 Make the Mem0, Letta, Zep/Graphiti and LangMem adapters real per Run 4 `R-56`/`R-57` and the 5.1 choices: model via the proxy, shared embedder where accepted, per-conversation isolation, shared answer template. Verify each with an offline test against the mock provider and fake embedder.
- [ ] 5.5 Add `--phase head-to-head`: exactly the five participants, pilot of one LoCoMo conversation per participant then stop, gaps for a participant that cannot run, parity deviations listed. Verify with a fixture run including one adapter that fails init.
- [ ] 5.6 Owner-approved live head-to-head pilot, then the rest after approval; deliver the memory report.

## Workflow follow-up

- Archive after `lab-datasets` is archived.

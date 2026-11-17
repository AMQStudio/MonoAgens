# Research brief 4 for MonoAgens: memory-framework comparison protocol

You are the researcher for MonoAgens. Read `research/researcher-prompt-2.md` for the project and
the rules every finding must respect; they still hold. This round is narrow: how to compare the
full MonoAgens against dedicated memory frameworks on LoCoMo and LongMemEval-S, fairly and by the
published standard.

## Read first

Repository: https://github.com/AMQStudio/MonoAgens, branch `main`.

1. `openspec/changes/lab-memory-comparison/` — every `[pending R-nn]` there waits on this brief.
2. `research/findings/2026-11-17-run-3.md` (`R-52`, `R-54`) — official LoCoMo and LongMemEval
   metrics and splits already verified.
3. `lab/adapters/` — the current adapter skeletons for `mem0`, `letta`, `zep_graphiti` and
   `langgraph_langmem`, and their pinned package versions in `lab/adapters/pyproject.toml`.

Fixed by the owner (do not reopen): phase 0 runs only MonoAgens on LoCoMo under the published protocol (R-55 is needed first); in the later head-to-head phases the systems compared are full MonoAgens, Mem0, Letta,
Zep/Graphiti and LangMem only (no full-transcript or no-memory arm); every system uses the same
LLM (`deepseek-v4.1-flash` through OpenCode Go's OpenAI-compatible endpoint, reasoning effort
`high`) and the same judge; phase 1 is LoCoMo in full, phase 2 is LongMemEval-S.

## Rules for this brief

- Every fact needs a primary source: the framework's own repository, documentation, evaluation
  code or paper, with a link and a pinned revision or version.
- Say plainly when you cannot confirm something.
- Do not choose for the owner. Where a choice is needed, give the options and what each implies.

## Questions

**R-55 Standard comparison protocol.** For LoCoMo and LongMemEval-S as used to compare memory
frameworks in published evaluations (Mem0, Zep, Letta, LangMem or others):
- which LoCoMo categories are scored and how category 5 (adversarial) is treated;
- the answer prompt, retrieval budget (top-k or token budget) and any per-system settings;
- the LLM-as-judge metric ("J" or equivalent): exact judge prompt, verdict format, and the
  model used; plus any other reported metric (F1, BLEU-1, latency, tokens);
- the token and latency accounting each publication reports (ingest vs answer);
- the **answer model** (the LLM that writes the answers) and the memory/ingestion model each
  publication used, and whether that model is available through OpenCode Go;
- the **published scores** per system, metric and LoCoMo category, exactly as printed, with the
  table or figure they come from. The owner's phase 0 will show these beside MonoAgens' scores.

**R-56 Running each framework on an OpenAI-compatible endpoint.** For Mem0, Letta,
Zep/Graphiti and LangMem at a current stable version:
- how to point every internal LLM call at a custom OpenAI-compatible base URL with custom
  headers (`x-opencode-session`, `User-Agent`) and a `reasoning_effort` field, or how to route it
  through a local proxy that adds them;
- how to use a custom embedding model served locally through an OpenAI-compatible
  `/v1/embeddings` endpoint, and which embedding dimensions each accepts;
- the infrastructure each needs (server process, database, graph database) and whether it runs
  locally on a 14 GiB RAM / 8 GB VRAM laptop;
- the public API to ingest a conversation and to answer a question, and how to isolate one
  conversation's memory from another's.

**R-57 Ingestion granularity.** For each framework's own published LoCoMo or LongMemEval
evaluation: is the conversation ingested per message, per message pair, per session or another
unit, and with which timestamps? How many LLM calls does one ingested unit cost?

**R-58 OpenCode Go caching.** From OpenCode's official documentation for OpenCode Go and the
`deepseek-v4.1-flash` model:
- whether prompt caching applies, how cached input is priced against uncached input, and how the
  response reports cached tokens (for example `usage.prompt_tokens_details.cached_tokens`);
- what `x-opencode-session` does for routing or caching;
- the plan's usage limits (monthly allowance, 5-hour and weekly windows) and what happens when a
  limit is reached.

## Deliver

`research/findings/<date>-run-4.md` with one section per question, plus notes in
`research/findings/run-4-notes/` as needed.

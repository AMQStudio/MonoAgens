# ADR-0019: Grounding, Freshness, and Research — Know What Time It Is, Know What You Do Not Know, Never Guess Current Facts

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** The Soul must chat naturally, answer about recent events, and never invent current facts. A research contract already exists (`engine/turn/research.py`: trigger, per-turn `ResearchCard`, deterministic `verify` of citations, freshness, and claim support), but its trigger depends on the per-turn decision call that ADR-0012 retires and on per-language keyword triggers, and nothing tells the Soul where its own knowledge ends. This ADR makes grounding a mechanism that survives the overhaul.
- **Supersedes:** in part ADR-0009 §4 (search routing of 4.4 through the turn decision call)
- **Related:** ADR-0012 (envelope, taint), ADR-0016 (memory layers), ADR-0017 (scenarios), ADR-0004 (web search providers)
- **Implementation:** draft

---

## 1. Context

- `research.trigger_reason` combines a mode (`off`, `auto`, `always`), keyword triggers from the
  lexicon, and the `needs_search` hint of `turn/decision.py`.
- `research.verify` checks that citations `[Rn]` exist, that sources are fresh enough, and that
  claims are supported by quotes, without a model call. It is a sound deterministic verifier.
- The prompt states the current date, but not the boundary of the model's own knowledge, so the
  model may answer a time-sensitive question from stale training data.
- Facts learned from research are not stored with an "as of" date, so an old answer can be
  repeated as if current.

## 2. Decision drivers

- Current facts come from evidence in this turn or from dated memory, never from the model's
  prior.
- "I do not know yet, let me check" is a correct answer; a confident guess is not.
- Casual conversation stays natural and free of tool calls.
- Checks are deterministic; no LLM judge.

## 3. Options considered

1. **Keep the decision call as the search trigger.** Rejected: ADR-0012 removes it, and a model
   deciding whether it knows enough is the failure being fixed.
2. **Search on every turn.** Rejected: cost, latency, and noise for casual chat.
3. **Temporal self-knowledge, model-initiated search, and deterministic grounding checks
   (chosen).**

## 4. Decision

### 4.1 Temporal self-knowledge

The verified facts block (ADR-0012) always states: the current date, time, and timezone; the
model's knowledge boundary (from operator configuration of the model, never hardcoded); and the
rule that anything after that boundary, or anything that changes over time (news, prices,
schedules, scores, versions, office holders, weather), must come from a source in this turn or
from dated memory.

### 4.2 Search is a tool the model uses, verified by the harness

- `web_search` and `fetch` are always offered when configured. The model decides to search; the
  harness does not guess intent from keywords.
- `research.mode: always` and explicit user requests ("search", "check", a pasted link) still
  force research deterministically.
- Fetched content is `untrusted_content` and sets the taint of ADR-0012 §4.

### 4.3 Deterministic grounding check

Before sending, the harness checks the reply structurally, as `research.verify` does today:

- Every citation points to a source of this turn, and the source is within the freshness window
  for its type.
- Specific claims (numbers, dates, amounts, versions, proper names that are not in the
  conversation) must appear in this turn's sources, tool results, the conversation, or memory.
  An ungrounded specific triggers one re-ask with the list of ungrounded items; a second failure
  sends the reply with those items replaced by an explicit "not verified" statement.

This is a verifier over data, not a model judging a draft; it never sees rooms the audience may
not see.

### 4.4 Dated world memory

Facts taken from research are stored through `MemoryStore` with `as_of` and source; rendering
shows their age ("as of 3 days ago"); past a per-type freshness window they are presented as
possibly outdated and must be re-checked before being stated as current.

### 4.5 Staying informed (optional plugin)

Proactively following news on chosen topics is an optional plugin (periodic job, ADR-0014) that
writes dated world memories; it is not core and is disabled by default.

### 4.6 Casual conversation

Small talk needs no tools and no citations; the grounding check applies only to specific claims,
so a natural chat passes untouched.

## 5. Configuration

Generated into `config.yaml` with comments:

```yaml
engine:
  research:
    # How the Soul handles facts that change over time.
    mode: auto               # auto = the Soul searches when it needs current facts.
                             # always = search on every question. off = never search.
    freshness_hours:         # How old a source may be and still count as current, per type.
      news: 48
      prices: 24
      reference: 8760        # One year.
    grounding: true          # true = numbers, dates, and names in a reply must come from a source,
                             # the conversation, or memory; unverified items are marked as such.
llm:
  knowledge_boundary: ""    # Where the configured model's built-in knowledge ends, e.g. "2026-06".
                            # Set per model by the operator; empty = treat everything recent as unknown.
```

## 6. Metrics and gates

- Golden scenarios: a news question produces a search and cited, fresh sources; an invented
  number is caught by the grounding check; casual chat makes zero tool calls.
- Live suite: daily freshness run asking about current events, asserting structure (search
  happened, sources dated within the window, specifics grounded).

## 7. Acceptance criteria

- Asked about something after the knowledge boundary, the Soul searches or says it does not know
  yet; it never answers from its prior as if current.
- A reply never contains a number or date that appears in no source, tool result, conversation,
  or memory without being marked unverified.
- A remembered world fact older than its window is presented as possibly outdated.

## 8. Risks and mitigations

- **The model does not search when it should.** Mitigation: temporal self-knowledge in the facts
  block; the grounding check catches the resulting specifics; scenarios measure it.
- **Grounding check too strict for paraphrased numbers.** Mitigation: normalization of number and
  date formats; failures mark items as unverified rather than blocking the reply.
- **Search results are themselves wrong or hostile.** Mitigation: source types, citations, taint.

## 9. Interaction with other ADRs

- ADR-0012: facts block, taint, and retirement of the decision call.
- ADR-0016: dated world memory is a semantic-memory kind.
- ADR-0014: the optional news plugin uses a periodic-job hook.
- ADR-0017: freshness and grounding scenarios join the coverage matrix.

## 10. Code references

- `monoagens/engine/turn/research.py` (`trigger_reason`, `ResearchCard`, `verify`),
  `monoagens/engine/turn/research_flow.py`, `monoagens/engine/turn/facts.py`.
- `monoagens/engine/tools/web_search.py`, `monoagens/engine/tools/fetch.py`.
- `monoagens/engine/turn/decision.py` (`needs_search` hint).

*This ADR lives at `docs/v0.1.0/adr/engine/0019-grounding-freshness-and-research.md`.*

# ADR-0012: Message Envelope and Privacy by Construction — Label Once at Ingress, Render Speakers Explicitly, Filter Context by Audience

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** Speaker, addressee, privacy, and kind are not stored with a message. Command receipts are stored as the bot's own words, a reply to the bot renders as "replying to Mono", the privacy level is back-filled minutes later, and privacy plus refusal honesty are enforced by LLM reviewers after drafting. This ADR labels every message once, persists the labels, renders them explicitly, and moves privacy enforcement to context assembly.
- **Supersedes:** in part ADR-0003 §4 (the post-draft review chain of 4.2), ADR-0002 §4 (privacy level assigned after storage), ADR-0004 §4 (verbatim leak screening of 4.2)
- **Related:** ADR-0011 (charter), ADR-0013 (receipts and requirements), ADR-0002 (memory), ADR-0003 (turn lifecycle)
- **Implementation:** draft

---

## 1. Context

Observed failures traced to the missing envelope (audit `.audit/02-turn-pipeline.md`,
`.audit/03-memory-privacy.md`, `.audit/08-stores-llm-providers.md`):

- `messages` has `author`, `visibility`, and a free `data` JSON, but no speaker, addressee,
  kind, trust, or validity columns (`monoagens/engine/schema/base.py`).
- History renders `role=<author> vis=L<n>` (`engine/turn/context.py:83`); command receipts and
  notices are `role=bot`, so the model reported "I approved #5 and #8" for approvals the user
  ran.
- A quote of the bot's own message renders as `replying to Mono: "..."`
  (`engine/turn/context.py:552`, `:588`); the model misread "bro gaslighting", aimed at itself,
  as aimed at the user.
- Old failures ("I cannot access Google") remain in the transcript as the bot's own words and
  poison later turns; a claim judge (`engine/claims.py`) re-asks drafts to compensate.
- The privacy level is NULL at ingress and filled by a background worker; disclosure and
  cross-room checks run over drafts, and `auditor: false` disables disclosure review entirely.
- Fetched web, Drive, and tool content carries no trust label (prompt-injection surface).

## 2. Decision drivers

- The model must never guess who is speaking or to whom.
- What the model must not disclose, it must not see.
- Labels must be cheap and deterministic; a model call is optional and off by default.
- Labels must be explainable and recomputable when rules change.
- A stale failure must stop being evidence once its cause is gone.

## 3. Options considered

1. **Keep per-turn LLM labelling (`turn/decision.py`) and add more prompt rules.** Rejected:
   costs a call per turn, labels are lost, and rules in prompts are not enforcement.
2. **Label lazily at first use.** Rejected: different consumers would label differently.
3. **Label once at ingress, persist with provenance, consume everywhere (chosen).**

## 4. Decision

### 4.1 Envelope labels

Every stored message carries these labels, written once when the message is recorded:

| Label | Values | Primary source |
|---|---|---|
| `speaker` | `self`, `sender`, `other_person`, `system_receipt` | harness (deterministic) |
| `actor` | person id of whoever caused a receipt | harness |
| `addressed_to` | `self`, `person:<id>`, `group`, `unknown` | deterministic addressing score (§4.7); optional tie-breaker off by default |
| `reply_to_self` | boolean | transport reply target matched to an outgoing message |
| `kind` | `chat`, `question`, `command`, `tool_result`, `notice` | harness |
| `privacy` | `L0`..`L4` (`L5` never stored) | deterministic evidence pipeline (ADR-0002) |
| `trust` | `owner`, `member`, `untrusted_content` | role plus origin (web, Drive, MCP output) |
| `valid_until` | condition for receipts and failures, such as a config or capability version | harness |

Each label records provenance in one JSON column: source (`transport`, `rule`, `llm`), rule or
model version, and for `llm` the confidence. Transport evidence always outranks `llm`. A
background refinement may raise `privacy`, never lower it.

### 4.2 Schema

One migration adds columns to `messages` (names final in the implementation PR):

```
messages
  speaker          text
  actor            text
  addressed_to     text
  reply_to_self    boolean
  kind             text
  trust            text
  valid_until      jsonb
  label_meta       jsonb
```

`visibility` becomes the stored `privacy` level and is written at ingress. Existing rows are
labelled by a deterministic backfill; rows that cannot be labelled get `unknown`.

### 4.3 Rendering contract

The transcript renderer is the only producer of model-facing history and has one
implementation (today there are two):

- `self` lines are marked as the Soul's own words; `system_receipt` lines are marked as system
  facts with their `actor` ("Muqsith ran /approve #5: approved"), never as the Soul's words.
- `reply_to_self` renders as a reply to the Soul's own message, quoting it.
- `addressed_to=self` is stated on the line.
- Receipts whose `valid_until` no longer holds render as superseded, or are dropped.
- `untrusted_content` is fenced as data, never as instructions.
- A verified facts block (today `engine/turn/facts.py`) is the single source of current
  capability and membership truth.

### 4.4 Privacy by construction

- Context assembly receives the audience (direct or group, members, room) and selects only rows
  and memories the audience may see; cross-room material enters only as meta facts ("an archive
  exists; it may not be quoted here").
- `Agent.dispatch()` re-checks the audience of the outbound content with the same policy
  function (ADR-0011 §4).
- With both guards in place and covered by tests, these post-draft layers are retired one at a
  time: the claim judge (`engine/claims.py`), the cross-room verbatim detector, the disclosure
  auditor, and character polish (which moves to the expression plugin). `turn/decision.py`
  reads stored labels instead of recomputing them.

### 4.5 Untrusted content and taint

A fenced data block is a prompt convention, not enforcement. The turn therefore carries a taint
bit, set when it ingests `untrusted_content` (fetched pages, web search results, Drive or file
content from outside the workspace, MCP output):

- While tainted, a tool call with an external side effect (sending to a room other than the
  current one, `send_file`, `relay`, `write` or `sed` outside the scratch area, `shell`,
  `settings`, plugin write actions) raises an `Approval` from the current sender (ADR-0013); for
  a sender without the `approve` permission it is refused.
- Replying in the current room stays allowed: the content returns to whoever asked for it.
- The taint bit and the tool calls it gated are recorded in the turn trace.

### 4.6 Secrets in transcripts

`L5` secrets are rejected from memory today, but the raw text stays in `messages` and could
reach activity gists (ADR-0016). At ingress the same deterministic secret gate redacts secrets
in the stored text, keeps a redaction marker (kind, position, keyed hash), and never passes the
secret to labels, gists, or consolidation. The transport's own message is outside our control;
the Soul says so when it detects a secret in a shared room.

### 4.7 Addressing: is this message for me?

`addressed_to` in a group is not guessed by a model. The gateway reports standard transport
signals; the core scores them deterministically together with conversation state it alone
knows, and stores the score with its reasons in `label_meta`.

Signals reported by gateways (fixed vocabulary in `monoagens.sdk`): `dm`, `mentions` (person
ids, with `mentions_self`), `reply_to` (message id, author, `author_is_self`), plain occurrence
of the Soul's name. Conversation state added by the core: the Soul spoke to this sender in this
thread within the continuity window with nobody else in between; a pending requirement of this
sender (ADR-0013).

| Signal | Points to | Default weight |
|---|---|---|
| Direct chat | self | decisive |
| Mention of the Soul | self | +100 (the only signal that passes alone) |
| Reply to the Soul's message | self | +50 |
| Reply to the Soul's message while mentioning another person | that person | −80 (quoting the Soul to tag someone) |
| Mention of another person, not the Soul | that person | −80 |
| Reply to another person's message | that person | −50 |
| Continuation of the Soul's conversation with this sender | self | +40 |
| Pending requirement of this sender | self | +40 |
| Soul's name written as plain text | self | +15 |
| Later: interlocutor graph, the sender's recent messages are an exchange with another person | that person | −40 |
| Later: semantic continuity, similarity to the Soul's last reply in this thread (local embedding model, no LLM) | self | up to +30 |
| No signal | nobody | 0 |

The design is a cascade, the pattern that addressee-recognition research and multi-party
assistants converge on: decisive transport signals first, weaker signals combined into a score,
silence when unsure (a wrong answer costs more than a missed one). Learned addressee models
(for example MPC-BERT on Ubuntu IRC) are not reliable enough to decide alone and are trained on
other languages and platforms; their insight, modelling who talks to whom, enters as the
interlocutor-graph signal.

Version 1 ships the rows without the "Later" prefix, plus reasons (§4.8); rows and steps marked
"Later" are added only when golden scenarios or real corrections show version 1 is not enough.

Bands: score at or above `answer_at` answers; below `ignore_below` stays silent; the band in
between stays silent unless the operator enables the optional tie-breaker (one small model
call, off by default). Silent messages are still recorded for context. The room or role
`skepticism` level shifts both thresholds. Expression plugins cannot raise a score
(ADR-0011 §4).

### 4.8 Reasons and calibration

- **Every decision has a reason.** The scorer returns the decision, the score, and the list of
  signals with their points ("mention_self +100", "reply_to_self_tagging_other −80"). It is
  stored in `label_meta` and in the turn trace for answers and for silences alike.
- **The model is told why it is speaking.** The rendered turn states the reason ("you were
  mentioned by Tari", "Tari continues your conversation"), so the reply addresses the right
  person. The model does not decide whether to speak and is not asked to justify it afterwards;
  a model's after-the-fact justification is not evidence.
- **Reasons are tested.** Golden scenarios (ADR-0017) assert the decision and its reasons, not
  only the decision, so a right answer for a wrong reason fails.
- **Later: weights are calibrated, not guessed.** The defaults are starting values. Labelled examples
  come from golden scenarios and from correction signals in real use: someone says the message
  was not for the Soul, a Soul reply is deleted, or a person mentions the Soul right after it
  stayed silent. Weights are refitted with logistic regression over the same signals, so they
  stay a small, explainable table; precision of answering is the primary metric, recall the
  secondary one. Refitted weights are proposed as a config change, never applied silently.

## 5. Configuration

- `engine.privacy.auditor` and other review switches are removed with the layers they control.
- Addressing (§4), generated into `config.yaml` by `templates.py` with these comments:

```yaml
engine:
  addressing:
    # How the Soul decides whether a group message is meant for it.
    # Every message gets a score from transport signals (mention, reply, ...) and from the
    # conversation itself. Mention is the only signal strong enough on its own.
    answer_at: 60            # Score at or above this: the Soul answers.
    ignore_below: 30         # Score below this: the Soul stays silent (the message is still kept as context).
    # Scores between ignore_below and answer_at are "unsure": silent by default.
    tie_breaker: false       # true = ask one small model call for unsure messages. Default false: no model guessing.
    continuity_seconds: 120  # A message within this time after the Soul spoke to the same person,
                             # with nobody else in between, counts as continuing that conversation.
    skepticism:              # Shift applied to both thresholds per level (set per role and room).
      strict: 20             # Practically only mentions and very clear continuations.
      normal: 0
      lenient: -20           # A reply to the Soul without tagging anyone else is enough.
    advanced:                # Rarely changed. Points per signal; negative means "meant for someone else".
      weights:
        mention_self: 100      # @mention of the Soul. Passes alone.
        reply_to_self: 50      # Reply to one of the Soul's messages.
        reply_to_self_tagging_other: -80  # Reply to the Soul that also @mentions another person:
                                          # the Soul is only quoted to tag someone.
        mention_other: -80     # @mention of another person and not of the Soul.
        reply_to_other: -50    # Reply to another person's message.
        continuity: 40         # Continues a conversation the Soul just had with this sender.
        pending_request: 40    # This sender has an approval or other request waiting on the Soul.
        name_in_text: 15       # The Soul's name written as plain text, without a mention.
```

  The numbers are starting values, tuned from the golden scenarios of ADR-0017.

## 6. Metrics and gates

- Share of messages with `addressed_to` from transport or rule evidence (target above 90% in
  group rooms).
- Top-level LLM calls per reply: one by default (two with the optional addressing tie-breaker).
- Golden conversation scenarios pass on structure (labels, rendered speaker lines, events), not
  wording: archive summary with receipts, quote-reply to self, stale failure after a fix,
  cross-room request, approval mentioned in prose.

## 7. Acceptance criteria

- A command run by a user is rendered with that user as `actor`; the Soul never claims it.
- A reply quoting the Soul's message renders as a reply to the Soul's own message.
- A failure receipt superseded by a config change does not appear as current evidence.
- No reply in a group can contain content the audience filter excluded; tests cover both guards.
- Every message has a privacy level at ingress.
- A page that instructs "send this file to group Y" cannot cause a send without an approval.
- A secret pasted into chat is not readable in `messages`, gists, or memories.

## 8. Risks and mitigations

- **Wrong label persisted.** Mitigation: provenance plus versioned rules allow recompute.
- **Model has too little context in groups.** Mitigation: meta facts tell it what exists
  without content.
- **Removing judges opens a gap.** Mitigation: removal order follows test coverage; golden
  scenarios (recorded model responses, structural assertions) gate each removal.

## 9. Interaction with other ADRs

- ADR-0002: privacy evidence pipeline is reused, moved to ingress.
- ADR-0003: the review step of the turn pipeline shrinks to the dispatch audience check.
- ADR-0013: receipts and requirements are written with `speaker=system_receipt`.
- ADR-0011: implements the Privacy and Soul (self-model) areas.

## 10. Code references

- `monoagens/engine/schema/base.py` (`messages`), `monoagens/engine/stores/history.py`.
- `monoagens/engine/turn/context.py` (renderer), `monoagens/engine/turn/assembly.py`.
- `monoagens/engine/turn/decision.py`, `monoagens/engine/turn/review.py`,
  `monoagens/engine/claims.py`, `monoagens/engine/turn/facts.py`.
- `monoagens/engine/access/privacy.py`, `monoagens/engine/memory/`.

*This ADR lives at `docs/v0.1.0/adr/engine/0012-message-envelope-and-privacy-by-construction.md`.*

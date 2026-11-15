# Tasks

Stage A (groups 1–11) is the foundation. Stage B (groups 12–21) is wave 1. Each task names
the design decision (D), the finding (R, in `research/findings/2026-11-15-run-1.md`) or the
`RESEARCH.md` section (§) it implements. Start values for every parameter are in R-03 and in
the finding named; they go into `prior/dispositions.json`, never into code.

No task waits for research. Do not settle an open point (D19) from your own judgement: build
what the finding states, and leave the rest to research brief 2, whose results enter the next
change.

## 1. Toolchain and workspace

- [ ] 1.1 Install Rust stable with `rustup`, add `rust-toolchain.toml` pinning stable with `rustfmt` and `clippy`; verify `cargo --version` and `cargo clippy --version` succeed
- [x] 1.2 Initialize a git repository with a `.gitignore` for `target/`, soul directories and `lab/runs/` except `lab/runs/reference/`; verify `git status` lists `ARCHITECTURE.md`, `RESEARCH.md`, `AGENTS.md`, `CLAUDE.md`, `research/` and `openspec/` as untracked
- [ ] 1.3 Create the Cargo workspace with crates `protocol`, `core`, `store`, `reason`, `lab` and the module files of D2 as empty stubs, edition 2024, `#![forbid(unsafe_code)]`; verify `cargo build --workspace` and `cargo clippy --workspace -- -D warnings` pass
- [ ] 1.4 Add a test in `core` that reads `cargo metadata` and fails if `monoagens-core` depends on anything other than `monoagens-protocol` and `serde`; verify it passes, and fails when a dependency is added temporarily

## 2. Protocol

- [ ] 2.1 Define the opaque ids, audience, receipt with five outcomes, and the capability descriptor with flags, typical latency and effect declaration (channel, window, noise); verify unit tests construct each and reject a percept without an audience
- [ ] 2.2 Define the messages: percept with its reportable facts (addressees, named, reply-to, correlation id, length, negative flag, vector with embedder tag), action, receipt, body state, actor facts (reliability prior, teacher weight); verify a percept with none of the optional facts is accepted
- [ ] 2.3 Define the five thought acts with the fields of D9 (R-12) and the classification verdict, with validation; verify a test round-trips every protocol type through JSON for equality and that an output fitting no act is reported invalid with its reason
- [ ] 2.4 Implement version handling: unknown fields ignored, older versions accepted, a newer version rejected with an error naming both; verify one test per case
- [ ] 2.5 Write `crates/protocol/README.md` with one serialized example per message; verify each example is parsed by a doc test

## 3. Core: log, loop, doors and reproducibility

- [ ] 3.1 Define the event type and the kinds of D4 with format versions and per-kind upcasting; verify tests for an upcast and for a loud failure on a newer version
- [ ] 3.2 Define the `Store` and `Doors` ports, a no-op `Doors` and an in-memory `Store` (including naive text and vector candidates); verify append and ordered-read tests and a shared store conformance test module
- [ ] 3.3 Implement `Core::open` and the `step` skeleton: append first, inherit `origin_actor` along `caused_by`, return effects; verify a failed append yields an error and no effect, and that a model result cannot change the origin
- [ ] 3.4 Call the three doors at steps 3, 6 and 7 of the turn and record a `door_verdict` for each removal or refusal (D1, D13); verify with a test door that removes a context item, refuses a wake and refuses an action
- [ ] 3.5 Implement the seeded draw from (seed, event id) and take time only from `now`; verify identical event sequences for identical inputs and seed, and a changed draw for a changed seed
- [ ] 3.6 Implement the six books of D4 as projections rebuilt at `open`; verify state after processing equals state after reopening on the same store
- [ ] 3.7 Implement the registry with the 22 rows of D18 and `Switches`, so a row without claim or neutral value does not compile; verify a fixed event stream completes every turn with every row neutral
- [ ] 3.8 Create `prior/dispositions.json` with format and content versions and every parameter of R-03 and of the findings (start value, range, source) and its loader; verify a valid prior loads and an out-of-range value is rejected naming the parameter
- [ ] 3.9 Implement ticks and `next_due()`, appending a tick only when it fires something; verify idle ticks leave the log unchanged
- [ ] 3.10 Implement replay and its comparison of decisions, expectation transitions and recall rankings; verify a clean log reproduces and a tampered decision is reported with the first differing event
- [ ] 3.11 Add the payload-independence test (same facts, different payload bytes, identical decisions) and `evidence.rs` (beta counts with the discount of R-20, rule of three, 95% interval) with unit tests against known values; verify both pass
- [ ] 3.12 Implement `activation.rs` with Petrov's hybrid base level (R-04: k recent uses exact, the rest approximated, age floored at one minute); verify against the exact sum for small n and that B is monotonic in d on a grid

## 4. Core: base gate (§2–3, D6)

- [ ] 4.1 Implement `own_name`, `feeling_of_knowing` and `relevance` with neutral values when a fact is missing, and `info_gap` taking reference vectors as input; verify unit tests per signal including the missing-vector case
- [ ] 4.2 Implement closeness from exchanges and place familiarity from presence with `B_ctx` (R-04); verify co-presence alone gives zero closeness and familiarity falls after contact stops
- [ ] 4.3 Implement `social_cost` (R-02); verify another selected speaker, an already-answered event and a close sender
- [ ] 4.4 Implement salience with the weight maps of R-03 and habituation with gain κ (R-01); verify the repeated-source drop, recovery after a pause, and that κ = 0 leaves salience unchanged
- [ ] 4.5 Implement the threshold with its four terms and the maps of §3 and R-03; verify one test per term for its direction
- [ ] 4.6 Implement the go / note / silent draw and the circuit breaker for allocated turns and expectation events; verify a statistical test (share of go near P) and that commitments bypass the draw
- [ ] 4.7 Record each decision with all numbers and its reason; verify a test recomputes the outcome from the stored event alone
- [ ] 4.8 Start `docs/mechanisms.md` with rows 1–7 of the brain map: formula, finding or section, parameters, neutral value; verify every parameter named exists in `prior/dispositions.json`

## 5. Core: recall (§5, D7)

- [ ] 5.1 Implement recall as the union of vector and text candidates ranked by total activation, returning provenance, with `memory_used` recorded when an item enters a plan; verify "often used beats merely similar", recall without vectors, and a higher base level on the second recall
- [ ] 5.2 Implement importance as the maximum of `i_reason` and `i_commit` (R-05; `i_surprise` arrives in 16.2); verify a promise-linked event outranks an equally old, equally similar routine one
- [ ] 5.3 Add a long-gap test (one fact, thirty virtual days, several hundred distractors, a differently worded query with a matching vector); verify the fact is in the top results

## 6. Core: base expectations (§6, §17, §19, D8)

- [ ] 6.1 Implement the expectation record and book with kinds effect, response, promise and goal and conditions due time and reply-from; verify open, answer and expire with their events
- [ ] 6.2 Open effect and response predictions after an action, with expiry from the body's declared latency (R-08; learning arrives in 12.3), and run a wake caused by an expectation in that expectation's chain; verify an answering reply wakes as a commitment in the same chain and an expiry is a noticed absence
- [ ] 6.3 Implement agency (R-09): match by correlation id or own actor, and by channel, window and tolerance; verify an echo and an in-window sensor change are self-caused and not appraised, a person's reply is not self-caused, an unpredicted sensor change is appraised, and an unmet effect prediction is an error
- [ ] 6.4 Fire time conditions from ticks; at `open` mark promises with due < now as overdue and wake once per place (R-10); verify firing no earlier than due and the start-up behaviour
- [ ] 6.5 Implement the promise states, retry at `t + b·2^(i−1)` up to `n_max`, the wake to inform or renegotiate, and kept late (R-10); verify the four scenarios of spec `expectations`
- [ ] 6.6 Implement goals (R-07): opened by `expect`, attribution steps 1 and 2 with the breaker, expiry; verify a receipt in a busy place wakes as a commitment and an expired goal attributes nothing
- [ ] 6.7 Apply `expect`, `resolve` and `revise` and expose promise debt to the gate; verify revising a due time (the old time does not fire) and promise debt rising when promises are due

## 7. Core: thinking (D9)

- [ ] 7.1 Build the `ContextPlan` in the order of R-11 with M, N and k from the prior; verify the order, the bounds on a long history, and that the trigger and fired expectations are never dropped
- [ ] 7.2 Build the chain working set: the soul's own thoughts and acts in full, the last M observations in full, older ones as id placeholders (R-11); verify a receipt-triggered wake sees the earlier action and receipt, and a long chain shows placeholders
- [ ] 7.3 Pass the plan through the context door, emit `Effect::Model` with purpose reason and record `wake_started` with the audience of the trigger and of each item; verify with the test door
- [ ] 7.4 Apply a model result: record thoughts, pass each act through the action door, record the action, emit `Effect::Dispatch`, refuse unknown capabilities, and refuse a `revise` aimed at a log event, the identity, a credence count or a disposition (R-12); verify each of the five acts, a result with no act, and the refusals
- [ ] 7.5 Implement `memory.recall` as the core's own capability through the action door, with a receipt that continues the chain; verify the next wake's plan holds the recalled items and that a refusing door stops it
- [ ] 7.6 Record identity from the identity input, default it on first start, record changes with origin and place it first in every plan; verify the scenarios of spec `soul-identity`
- [ ] 7.7 Run a complete loop with no-op doors and a canned model result; verify "the doors apply no rule" from spec `cognitive-loop`, and that the `core` suite runs in under 5 seconds

## 8. Store

- [ ] 8.1 Implement the SQLite `Store` with `rusqlite` (`bundled`): events table, ordered reads, append in a transaction; verify the store conformance tests pass on an in-memory database
- [ ] 8.2 Add FTS5 text candidates keyed by the query event's id, vectors tagged with their embedder, in-memory brute-force cosine and vector release; verify a rare exact term, nearest-vector order, no comparison across embedder tags, and a released vector no longer matching
- [ ] 8.3 Add reopen tests: an appended event survives reopening a file, and a log with a newer event version fails at open; verify on a temporary file

## 9. Reason: rendering and providers (D14, spec `model-providers`)

- [ ] 9.1 Implement rendering of a `ContextPlan` (plan order, quoted-data delimiters, absolute times with relative wording computed against `<now>`, the labels of each remembered item); verify the same plan renders identically twice, two wakes share the identity-and-capabilities prefix, and an instruction-like payload appears only inside delimiters
- [ ] 9.2 Define the thought acts as tool definitions per purpose and parse provider output into protocol types, plain text becoming a thought; verify several acts in order, a malformed act reported with its field, and plain text
- [ ] 9.3 Implement the `scripted` model provider (outputs keyed by trigger and purpose, scripted latency); verify a full loop with no network access
- [ ] 9.4 Implement the embedders `none`, `hash` and `scripted`, the embedding of condition and expected-reply text from thought acts, and truncation to a configured dimension with re-normalization and an embedder tag of model, dimension and quantization (R-13); verify determinism of `hash`, concept-label vectors of `scripted`, unit length after truncation, and that a failed embed leaves the event without a vector and records the failure
- [ ] 9.5 Implement secrets from the environment and `secrets.toml`, refusing keys in scenario or soul files; verify the refusal and that no recorded failure contains key material
- [ ] 9.6 Implement the `openai-compatible` model and embedding providers with `ureq`, time limits, usage including cached tokens, and failure classes; verify against a local mock HTTP server: success, tool calls, authentication error, rate limit, timeout, invalid output
- [ ] 9.7 Accept `info_gap` reference vectors as an optional input of the soul with none shipped, and keep `w_info` at 0 in the prior (the reference set is research brief 2, A-14, and belongs to the next change); verify the gate runs with `info_gap` neutral and that supplied vectors reach the core as numbers

## 10. Lab foundation (D15, spec `lab`)

- [ ] 10.1 Define the TOML scenario format and parser (soul with actor facts, test bodies, timed steps with reportable facts and concept labels, scripted outputs, SPEAK / SILENT annotations, tagged checks); verify a valid file and errors naming the offending key
- [ ] 10.2 Implement the virtual clock and the discrete-event driver over the real core, store and `reason`; verify two events thirty days apart run without waiting and that events during a scripted model latency are appraised before its result
- [ ] 10.3 Implement test bodies with places, actors, capabilities with latency and effect declarations, scripted receipts and follow-up percepts; verify a chat body that echoes and a device body with a sensor percept
- [ ] 10.4 Implement the cost guard as a rule at the context door (run token limit, chain wake limit, both adjustable and removable); verify a self-waking chain is stopped and reported, and that with the limit removed it is not
- [ ] 10.5 Implement the checks of spec `lab` and the foundation metrics (wakes per event, calls and tokens by purpose, promises kept and kept late, recall hits at gaps, narrower-audience items in wider contexts); verify one passing and one failing scenario per check kind
- [ ] 10.6 Implement run records and `monoagens-lab run` (scenario or set, seed, machine-readable output, non-zero exit on a failed check); verify two scripted runs with one seed write identical logs
- [ ] 10.7 Implement `--ablate`, `compare` with at least five seeds per side and a confidence interval per metric, and `mechanisms` (R-01); verify ablating habituation changes the wake count beyond seed noise in a repeating-source scenario and a no-effect row is flagged
- [ ] 10.8 Implement `coverage` (every row of the brain map is named by at least one dev scenario, else non-zero exit) and sealed-set handling with `lab/scenarios/sealed/README.md`; verify an uncovered row is reported and a sealed run prints no scenario content
- [ ] 10.9 Implement `log`, `explain` and `replay`; verify the explanation prints every number of a silent decision and a recorded log replays with exit status zero
- [ ] 10.10 Implement `calibrate` (R-03): `fok_threshold`, `fok_scale`, `wake_gate_lo` and its bounds from the development set per embedder, recorded with the run and refused on other sets; verify the computed values on a small fixed set and the refusal
- [ ] 10.11 Implement `baseline` (R-11): a run with the context replaced by the full transcript, with an optional restart in mid-task, reporting success, tokens and cache share for both forms; verify on a two-step scripted scenario
- [ ] 10.12 Implement live mode (`--live`) with a pre-flight check of provider settings and separate reporting of structural and wording checks; verify a live run without settings stops before any scenario
- [ ] 10.13 Write `docs/lab.md` (scenario format, commands, sets, calibration, how to add a mechanism) and `docs/testing.md` (layers and budgets); verify the example scenario in `docs/lab.md` runs

## 11. Stage A: foundation proven end to end

- [ ] 11.1 Write dev scenario `recall-across-places`: a fact told in one place, asked in other words in another place thirty days later; verify scripted and that the context check finds the fact
- [ ] 11.2 Write dev scenarios `promise-kept`, `promise-after-downtime` and `promise-retried`: a reminder after two hours of silence; the same with the soul stopped across the due time; the same with a failed receipt first; verify kept, overdue then kept late, and retried then kept
- [ ] 11.3 Write dev scenario `group-silence`: three actors talk without addressing the soul, then one does; verify silent decisions with zero model calls, then one reply
- [ ] 11.4 Write dev scenario `ask-group-report-back` as a goal: a private request to ask a group and report back, with the answer arriving amid chatter; verify the answer is attributed to the goal, the private request is in the later context, and an action lands in the private place
- [ ] 11.5 Write dev scenario `device-door`: the soul pushes a door through a device body; once the sensor answers in its window, once it stays silent, once it changes with no act; verify self-caused, an effect error and an appraised external change
- [ ] 11.6 Write dev scenarios that name rows 1–7, 10, 12, 13 and 20 of the brain map for `coverage`, and two validation scenarios varying 11.1 and 11.3; verify `monoagens-lab coverage` exits zero for those rows
- [ ] 11.7 Add a `cargo test` integration that runs the dev and validation sets scripted and replays every log; verify it passes within the Lab budget
- [ ] 11.8 Run `calibrate`, then the stage-A dev set live, `compare` for the stage-A rows and `baseline` on 11.4; verify all structural checks pass and write `lab/runs/reference/stage-a.md` with wording pass rates, token cost, each row's metric differences with intervals, and the transcript comparison

## 12. Gate, wave 1 (§4, §15, D6, D8)

- [ ] 12.1 Implement patience as a `wait` expectation on unallocated go decisions; verify yielding when another actor replies in time, waking at expiry, and no wait on an allocated turn
- [ ] 12.2 Implement backlog chunking (R-14): the backlog test, the age-discounted maximum, the breaker for a commitment in the chunk, at most four chunks per wake; verify thirty stale events give one decision and six backlogged places are handled four then two
- [ ] 12.3 Implement timing learned per place (R-08): quantile estimates for patience, expiry and the silence window, shrunk toward the body's latency, recorded as `timing_updated`; verify a slow place lengthens expiry and a new place uses the declared latency
- [ ] 12.4 Document rows 8 and 22 in `docs/mechanisms.md` and write dev scenarios `yield-to-speaker` and `missed-backlog`; verify scripted passes and an ablation difference for both

## 13. Thinking, wave 1 (§4, §11, §14, D9)

- [ ] 13.1 Implement the speaking and certainty bars on unallocated turns using the `communicative` flag, recording withheld acts with both numbers; verify low motivation withholds and an allocated turn is not withheld
- [ ] 13.2 Implement report control (R-15): `P_rc` from the certainty bar, value and cost; up to `n_recall` recalls; then a plan that asks for a coarser grain; then withholding; verify the four scenarios of spec `reasoning`
- [ ] 13.3 Add the model's knowledge cutoff to the plan and the rendering, and the eight speaking criteria of §4 to the instructions for unallocated turns; verify the cutoff appears with `<now>` and the criteria appear only on unallocated turns
- [ ] 13.4 Document rows 9 and 17 and write dev scenarios `withheld-turn` and `not-sure-enough`; verify scripted passes and an ablation difference for each

## 14. Expectations, wave 1 (§6, §17, §20, D8)

- [ ] 14.1 Implement free-text conditions with the wake gate and step 3 of goal attribution: ignore below `wake_gate_lo`, otherwise one `classify` request recorded as a `verdict` with the similarity; verify an unrelated event causes no call, a negative verdict keeps the expectation open, a positive one fires it, and a related remark is attributed to its goal without bypassing the gate
- [ ] 14.2 Keep `wake_gate_hi` at 1 until the rule of three allows a lower value; verify every event above the lower band is classified while verdicts are too few, and a candidate is adopted only after 50 with none negative
- [ ] 14.3 Implement content misses (R-18): the miss value from the act vector and the optional expected-reply vector, its z-score against the place's unrepaired replies, and a wake with who replied, timing, length and distance in the plan; verify a short, fast, far reply is a miss and an ordinary reply is answered
- [ ] 14.4 Implement judged silence (R-19): a prediction for every `note` decision and a seeded fraction ρ of silent ones, the unanswered test, and the asked-again test with one classification; verify the three scenarios of spec `expectations`, including that replay picks the same silences
- [ ] 14.5 Document the wave-1 parts of row 12 and write dev scenarios `free-text-condition`, `did-not-land` and `silence-was-wrong`; verify scripted passes and an ablation difference for each

## 15. Beliefs (§7, §14, D10)

- [ ] 15.1 Implement basis (R-17): taken from `think`, checked by the core, `observed` downgraded to `inferred` when its links reach no witnessed percept; verify the three scenarios of spec `beliefs`
- [ ] 15.2 Implement validity through `belief_changed` (`valid_to`, `superseded_by`) from `revise`, ranking superseded items below current ones and marking them; verify the changed-address scenario of spec `memory-recall`
- [ ] 15.3 Implement credence (R-20): evidence only for the `used_ids` of the deciding think, the periodic relaxation toward the prior, and rebuild from the log; verify the four scenarios of spec `beliefs`
- [ ] 15.4 Implement reliability per (actor, domain) from actor facts or a uniform prior, with domains supplied from outside or by reasoning (R-21; clusters arrive in 16.6); verify an actor often wrong in one domain gives lower starting credence there only
- [ ] 15.5 Implement the recollection test (R-16) and put the labels of each remembered item into the plan (basis, recollected or familiar, verbatim or gist, credence, validity); verify the three scenarios of spec `memory-recall`
- [ ] 15.6 Document rows 14 and 16 with their four switches and write dev scenarios `told-vs-observed`, `stale-address`, `unreliable-source` and `familiar-not-recollected`; verify scripted passes and an ablation difference for each switch

## 16. Sleep (§12, §21, §14, D11)

- [ ] 16.1 Implement the trigger (R-22) and the progress events so that a run never delays appraisal and resumes after a stop; verify a message during sleep is appraised at once, a busy period starts no sleep, and no episode is consolidated twice after a restart
- [ ] 16.2 Implement event boundaries (R-23): surprise as the maximum of time error, content error and novelty, the spike rule, the fallback without vectors, `episode_closed`, and `i_surprise` for importance (R-05); verify an abrupt topic change splits, a long steady conversation does not, and the fallback on a change of participants
- [ ] 16.3 Emit one `consolidate` request per closed episode and record gists, lessons (when two or more episodes share a prediction error) and procedures (when a goal was reached), each linked to its source ids (R-22); verify the three scenarios of spec `sleep-consolidation` and that every source event is reachable and unchanged
- [ ] 16.4 Implement merging by place, participant Jaccard and time (R-24); verify two similar episodes with different people stay separate
- [ ] 16.5 Make the verbatim record prevail: when a gist's detail is absent from its source events, supersede the gist; verify the scenario of spec `beliefs`
- [ ] 16.6 Recompute the K domain centroids during sleep with a seeded method and record `domains_updated` (R-21); verify replay reproduces the same domains
- [ ] 16.7 Implement fading with `vector_released`; verify a faded event is not found by vector, is found by its words and is reachable from its gist
- [ ] 16.8 Document rows 11, 18 and 21 and write dev scenarios `sleep-supersedes-plan`, `two-similar-talks`, `lesson-from-two-errors` and `long-gap-after-sleep`; verify scripted passes and an ablation difference for each

## 17. Perception ledger (§18, D12)

- [ ] 17.1 Build per-actor cards from projections with the four fields and the size bound of R-25; verify every entry points to recorded events and the ledger rebuilds from the log
- [ ] 17.2 Flag candidate false beliefs when an item an actor witnessed is superseded outside its audience; verify the moved-object pair of scenarios in spec `perception-ledger`
- [ ] 17.3 Build the self card (what the soul said, promised and claimed toward those present, capabilities offered now from `body_state`); verify a removed capability disappears from it
- [ ] 17.4 Place one compact card per person present and the self card in the plan and render them, with second-order facts only for the soul's own claims (R-25); verify the flag reaches the context of the stale actor's wake
- [ ] 17.5 Document row 19 and write dev scenarios `false-belief` (chat and device variants) and `what-did-i-promise`; verify scripted passes and an ablation difference

## 18. Bounded learning (§8, §6, D12)

- [ ] 18.1 Derive the outcome of a decision from its resolved prediction (R-26), including repair, failed receipt, negative flag and missed silence; verify the three scenarios of spec `bounded-learning`
- [ ] 18.2 Implement weight learning by the delta rule, scaled by teacher weight and clamped; verify the direction of change, no change at rate zero, and no global change from an actor with teacher weight zero
- [ ] 18.3 Implement bounded updates for threshold terms: the 95% interval test, step and daily bounds, floor and ceiling, freeze and report; verify a handful of observations moves nothing, a strong day moves at most five percent, and a pinned value freezes with a report event
- [ ] 18.4 Implement per-place and per-source offsets learned from everyone's outcomes by the same rule; verify two places with opposite outcomes diverge while the global value stays
- [ ] 18.5 Implement band learning for `wake_gate_lo` with its margin and bounds; verify bounded downward steps after repeated positive verdicts near the band
- [ ] 18.6 Record every change as `learning_change` with its delta and implement `rollback` as reversing events; verify current values equal prior plus deltas and a rollback to seven days ago restores them
- [ ] 18.7 Document row 15 with its switches off by default and write dev scenario `two-places-opposite-norms` over a simulated month and `learning-with-rollback`; verify a run with switches off has no learning event and both scenarios pass scripted

## 19. Lab, wave 1

- [ ] 19.1 Implement the behaviour metrics from SPEAK / SILENT annotations (R-27): when-to-speak precision and recall, false speaks, missed replies, share of messages against 1/n, reply-delay distribution; verify the worked example of spec `lab` gives 0.8 and 0.8
- [ ] 19.2 Implement the memory and honesty metrics (detail asserted without recollection, superseded beliefs presented as current, false-belief answers, cache share); verify each on a small scenario built to trip it
- [ ] 19.3 Implement the soul directory (`soul.toml`, `log.db`) and continuing a soul across sessions; verify memories, open promises and learned values carry over
- [ ] 19.4 Implement `monoagens-lab talk` against the real clock with the model on a worker thread; verify by an integration test with the scripted provider that a typed line gets a reply and a due reminder appears unprompted
- [ ] 19.5 Extend `docs/lab.md` with annotations, metrics, soul directories and the probe; verify the commands in it run as written

## 20. Stage B: wave 1 proven end to end

- [ ] 20.1 Add validation scenarios that vary at least six wave-1 dev scenarios (different sizes, gaps, order); verify they pass scripted without changing any mechanism
- [ ] 20.2 Verify `monoagens-lab coverage` exits zero for all 22 rows and the scripted Lab layer stays within its budget
- [ ] 20.3 Check the claim of R-03 on the development set: with every knob at 0.5 the soul's message rate is between 0.5 and 2 times 1/n; verify the measured ratio is in `lab/runs/reference/stage-b.md`
- [ ] 20.4 Run the full dev and validation sets live with a real model and the reference embedder; verify all structural checks pass and commit the run record
- [ ] 20.5 Run `compare` for all 22 rows, rows 6, 11 and 22 first, and write `lab/runs/reference/stage-b.md`: metric differences with intervals per row, rows with no measured effect, wording pass rates, cost by purpose
- [ ] 20.6 Present to the owner the rows with no measured effect and the parameters whose start values looked wrong, each with the evidence; verify the list is in `lab/runs/reference/stage-b.md`

## 21. Research track and documents

- [ ] 21.1 Create `research/backlog.md` listing R-01 to R-40 with status after run 1 and the open notes of D19; verify it matches the status table of the findings
- [ ] 21.2 Correct `RESEARCH.md` as run 1 lists: terms (host → gateway, hooks → doors), links to sections and files that do not exist, "owner" in §7–8 replaced by per-actor numbers, §5 text candidates placed behind the store, §19 limited to effect predictions, the 1/n norm and the 88% cache figure marked as MonoAgens's own assumption and unsourced; verify no reference points to a missing file or section
- [ ] 21.3 Add or amend the sections of `RESEARCH.md` for the accepted findings (formula, parameters, neutral value, claim, sources, and what is inference); verify every finding R-01 to R-05 and R-07 to R-27 is referenced
- [ ] 21.4 Apply the citation corrections of R-28 to the reference list and mark the four unverified details; verify no "title to verify" note remains
- [ ] 21.5 Write the top-level `README.md`: what this is, that it restricts nothing but cost and is for the Lab only, install, first run, the probe, where research lives; verify every command in it runs as written, then update `AGENTS.md` if a command changed

## 22. Integration checks

- [ ] 22.1 Run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings` and `cargo test --workspace` from a clean checkout; verify all pass and the budgets of D16 hold
- [ ] 22.2 Benchmark the `feeling_of_knowing` probe and recall at 10⁵ events; verify the per-event appraisal time is recorded in `docs/testing.md`
- [ ] 22.3 Manual check with `monoagens-lab talk` and a real model: tell a fact, quit, restart, ask in other words, ask for a reminder in two minutes and wait, then read both with `explain`; verify recall after restart and an unprompted reminder
- [ ] 22.4 Run `openspec validate core-and-lab --strict`; verify it reports the change as valid

## Workflow follow-up

- Research brief 2 (`research/researcher-prompt-2.md`) goes to the researcher, who now has read access to the repository. Its findings enter the changes after this one.
- Decide the license before the first public release.
- The owner writes sealed scenarios and runs them after stage B.
- Archive this change after review, then plan: wave 2 (R-29 to R-31), the gateway with policy at the doors (R-33, reviving the parked `bootstrap-vertical-slice`), hard forgetting (R-34), and wave 3 once R-32 is answered.

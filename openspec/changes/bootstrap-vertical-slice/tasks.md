# Tasks

## 1. Toolchain and workspace

- [ ] 1.1 Install Rust stable with `rustup`, add `rust-toolchain.toml` pinning stable with `rustfmt` and `clippy`; verify `cargo --version` and `cargo clippy --version` succeed
- [ ] 1.2 Initialize a git repository with a `.gitignore` for `target/`, data directories and `lab/runs/`; verify `git status` shows `ARCHITECTURE.md`, `RESEARCH.md` and `openspec/` as untracked and nothing ignored by mistake
- [ ] 1.3 Create the Cargo workspace with empty crates `protocol`, `core`, `store`, `gateway`, `lab`, `cli` (design Decision 1), edition 2024, `#![forbid(unsafe_code)]` in each; verify `cargo build --workspace` and `cargo clippy --workspace -- -D warnings` pass
- [ ] 1.4 Add a test in `core` that reads `cargo metadata` and fails if `monoagens-core` depends on anything other than `monoagens-protocol` and `serde`; verify it passes, and fails when a dependency is added temporarily

## 2. Protocol

- [ ] 2.1 Define the six facts and opaque id types (actor, place, audience, capability descriptor with flags, receipt with five outcomes) with serde; verify unit tests construct each and reject a percept without an audience
- [ ] 2.2 Define boundary messages (percept, action, receipt, capability registration, lifecycle) and `ThoughtAct` (think, act, expect, resolve, revise) with a protocol version field; verify a test round-trips every message type through JSON and compares for equality
- [ ] 2.3 Implement version handling: unknown fields ignored, older versions accepted, newer version rejected with an error naming both versions; verify with one test per case
- [ ] 2.4 Write `crates/protocol/README.md` describing the six facts and the serialized form with one example per message; verify each example in it is parsed by a doc test

## 3. Core: log, loop and reproducibility

- [ ] 3.1 Define the event type with kinds, format version, time, facts, `origin_actor`, `chain`, `cause`, features and opaque payload (design Decision 3), plus per-kind upcasting; verify tests for upcast of an older version and a loud failure on a newer one
- [ ] 3.2 Define the `Store` and `Doors` ports and an in-memory `Store`; verify a test appends events and reads them back in sequence order
- [ ] 3.3 Implement `Core::open` and `Core::step` skeleton: append the input first, inherit `origin_actor` and `chain` from `cause`, return effects; verify tests that a failed append returns an error and no effect, and that inheritance cannot be overridden by a reasoning result
- [ ] 3.4 Implement the seeded generator (`hash(seed, event id)`) and take time only from the `now` argument; verify a test that the same inputs and seed give identical event sequences, and a different seed changes at least one draw
- [ ] 3.5 Implement projections rebuilt from the log at `open`; verify a test that state after processing equals state after dropping the core and reopening on the same store
- [ ] 3.6 Implement the mechanism registry and `Switches` (design Decision 14) so a row without claim or neutral value does not compile; verify a test that runs a fixed event stream with every mechanism neutral and completes every turn
- [ ] 3.7 Load and validate the prior (`prior/dispositions.json` with format and content versions, ranges per knob); verify tests for a valid prior and for an out-of-range value naming the knob
- [ ] 3.8 Implement tick handling and `next_due()`: a tick is appended only when it fires something; verify a test that idle ticks leave the log unchanged
- [ ] 3.9 Implement replay: reprocess a recorded log without effects and compare decisions; verify a test that a clean log reproduces and a tampered decision is reported with the first differing event
- [ ] 3.10 Add the payload-independence test (same features, different payload bytes, identical decisions); verify it passes and that the whole `core` suite runs in under 3 seconds

## 4. Core: appraisal gate

- [ ] 4.1 Implement the five signals from features and state with neutral values when a feature is missing (`RESEARCH.md` §2); verify unit tests per signal including the missing-vector case
- [ ] 4.2 Implement salience and habituation per (place, source) with decay computed from timestamps (`RESEARCH.md` §3); verify tests for the repeated-source drop and recovery after a pause
- [ ] 4.3 Implement the threshold with talk rate, budget used, promise debt and familiarity terms and the knob mappings from the prior; verify one test per term moving the threshold in the specified direction
- [ ] 4.4 Implement the go / note / silent draw and the circuit breaker for allocated turns, expectation events and active chains; verify a statistical test (share of go near P over many events) and tests that commitments bypass the draw
- [ ] 4.5 Record each decision with all its numbers and its reason; verify a test that recomputes the outcome from the stored event alone
- [ ] 4.6 Add a gate section to `crates/core/README.md` mapping each formula to its `RESEARCH.md` section and knob; verify every knob named there exists in `prior/dispositions.json`

## 5. Core: memory recall

- [ ] 5.1 Implement base-level activation (Petrov approximation) from `memory_used` events and age; verify tests that more uses raise it and age lowers it by the power law
- [ ] 5.2 Implement recall: union of vector and text candidates from the `Store`, ranked by total activation, returning provenance; verify tests for "often used beats merely similar" and for recall without vectors
- [ ] 5.3 Record a `memory_used` event when a memory enters a context plan; verify a test that a second recall of the same item sees a higher base level
- [ ] 5.4 Add a long-gap test: one fact, thirty virtual days and several hundred distractor events, a differently worded query with a matching vector; verify the fact is in the top results

## 6. Core: expectations

- [ ] 6.1 Implement the expectation record and projection (kinds promise and prediction, conditions `due_at` and `reply_from`); verify tests for open, answer, expire and their recorded events
- [ ] 6.2 Fire time conditions from ticks and at `open` for overdue items, recording lateness; verify tests for firing no earlier than due and for firing at start-up after downtime
- [ ] 6.3 Keep a fired promise open and refire with backoff after a failed wake; verify a test that a reasoning failure leaves the promise open and schedules the next fire
- [ ] 6.4 Open a prediction after every action and mark echoed deliveries as self-caused; verify tests that an answering percept wakes as a commitment, an expiry is recorded as a noticed absence, and an echo causes no appraisal
- [ ] 6.5 Apply `expect`, `resolve` and `revise` from reasoning results and expose promise debt to the gate; verify tests for revising a due time (old time does not fire) and for promise debt rising when promises are due

## 7. Core: context plan and thought acts

- [ ] 7.1 Build the `ContextPlan` in the fixed section order with bounds from the prior and the context audience (design Decision 8); verify tests for the order, for the bounds on a long history, and that the trigger and fired expectations are never dropped
- [ ] 7.2 Pass the plan through the context door and emit `Effect::Wake`; record `wake_started`; verify a test with a door that removes an item and one that refuses the wake
- [ ] 7.3 Apply a reasoning result: record thoughts, pass each act through the action door, record the action, emit `Effect::Dispatch`, refuse unknown capabilities with a receipt; verify tests for each of the five thought acts and for a result with no act
- [ ] 7.4 Include the chain working set in later wakes of the same chain; verify a test that a receipt-triggered wake's plan contains the earlier action and receipt
- [ ] 7.5 Run a complete loop with no-op doors and a canned reasoning result; verify the valve-free scenario from spec `cognitive-loop` passes in `core` tests

## 8. Store

- [ ] 8.1 Implement the SQLite `Store` with `rusqlite` (`bundled`): events table, ordered reads, append in a transaction; verify the `core` store conformance tests pass against it using an in-memory database
- [ ] 8.2 Add FTS5 text candidates keyed by the query event's id and in-memory brute-force cosine over stored vectors; verify tests for a rare exact term and for nearest-vector order
- [ ] 8.3 Add crash-safety and reopen tests: an appended event survives reopening the file, and a log with a newer event version fails at open; verify both pass on a temporary file

## 9. Gateway: configuration, persons and identity

- [ ] 9.1 Implement `monoagens.toml` loading and validation (version 1, sections per design Decision 11), default file creation, XDG paths, `--config` and `MONOAGENS_HOME`; verify tests for a valid file, a newer version, a parse error with location, and a secret-looking key
- [ ] 9.2 Implement secrets from environment variables and `secrets.toml`; verify a test that a provider error is recorded without any key material
- [ ] 9.3 Implement person mapping with stable ids for unmapped accounts and the owner; verify tests for two accounts mapping to one person and for the same unknown account getting the same id after a restart
- [ ] 9.4 Implement identity: default on first start, change detection at start-up recorded as an owner change, and the `identity.revise` capability that updates the file with a backup; verify tests for each and that changing the model records no identity change
- [ ] 9.5 Document the configuration in `docs/configuration.md` with a complete example; verify a test loads that example

## 10. Gateway: valve and doors

- [ ] 10.1 Implement the valve as pure functions with the two cost guards of design Decision 9 (daily budget with the fired-promise exception, chain brake) and per-rule switches; verify unit tests for the budget, brake, switched-off and same-facts-twice scenarios in spec `valve`, and a test that the valve refuses nothing because of actor, audience or capability
- [ ] 10.2 Implement `Doors` in the gateway with the valve first and last around other rules and a `door_verdict` event for each removal or refusal; verify a test where a later rule tries to widen and the last pass undoes it
- [ ] 10.3 Record `valve_state` at start-up and on change, and record on every `wake_started` the audience of the trigger and of each context item; verify tests that a run with the valve off says so in the log, and that every percept, thought, action and receipt in a recorded run carries `origin_actor` and an audience

## 11. Gateway: runtime, plugins and providers

- [ ] 11.1 Implement the runtime: one channel, one task owning the core, ticks driven by `next_due()`, model calls and actions as spawned tasks with timeouts; verify tests that three events are appraised during a slow model call and that a timeout becomes a failure event
- [ ] 11.2 Implement the built-in plugin interface: place and capability registration, routing by place, receipts, and a body failure leaving the loop running; verify tests for routing to the right body, no owner of a place, a hanging body and a crashing body
- [ ] 11.3 Implement prompt rendering from a `ContextPlan` (stable-to-volatile, quoted-data delimiters, absolute times, `<now>`); verify tests that two wakes share the identity-and-capabilities prefix and that an instruction-like payload appears only inside delimiters
- [ ] 11.4 Implement the `scripted` model provider and the `none`, `hash` and `scripted` embedding providers; verify a full loop test with no network access
- [ ] 11.5 Implement the `openai-compatible` model and embedding providers (tool calls parsed to `ThoughtAct`, plain text as a thought, usage recorded as `model_call`); verify tests against a local mock HTTP server for success, tool calls, an error status and a timeout
- [ ] 11.6 Attach features at the inbound door and continue without a vector when the embedder fails; verify a test for the failure path
- [ ] 11.7 Verify the `gateway` test suite runs in under 15 seconds and record the measured time in `docs/testing.md` together with the four test layers

## 12. Bodies and CLI

- [ ] 12.1 Implement the terminal body (one place, the local account, a speaking capability, echo of delivered actions) and `monoagens chat`; verify by an integration test with the scripted provider that a typed line produces a printed reply and both are in the log
- [ ] 12.2 Implement first-run behaviour of `monoagens chat` without configuration or model settings; verify a test for the created file, the printed message and the non-zero exit status
- [ ] 12.3 Implement `monoagens log` with filters and `--explain <event>` for decisions; verify a test that the explanation prints every number of a silent decision
- [ ] 12.4 Implement `monoagens replay`; verify a test that a recorded chat log replays cleanly with exit status zero
- [ ] 12.5 Add a machine-readable output option and error-stream handling to all reporting commands; verify a test parses the JSON output of `monoagens log`
- [ ] 12.6 Write the top-level `README.md`: what MonoAgens is, the safety note that this version restricts nothing but cost, install, first chat, and where the Lab and research live; verify every command in it runs as written

## 13. Lab

- [ ] 13.1 Define the TOML scenario format (persons, places, timed steps, concept labels, scripted outputs, tagged checks) and its parser; verify tests for a valid file and for errors that name the offending key
- [ ] 13.2 Implement the test body and the virtual clock, and the runner that builds the real gateway and core over an in-memory database; verify a test that two events thirty days apart run without real waiting
- [ ] 13.3 Implement the checks (action / no action in place within time, context contains / lacks item, expectation state, receipt outcome, payload text) and the standard metrics; verify tests with one passing and one failing scenario per check kind
- [ ] 13.4 Implement run records and `monoagens lab run` with scenario or set selection, seed and machine-readable output; verify a test that two scripted runs with the same seed write identical logs and that a failing check gives a non-zero exit status
- [ ] 13.5 Implement `--ablate`, `lab compare` and `monoagens lab mechanisms`; verify a test that ablating habituation changes the wake count in a repeating-source scenario and that a no-effect mechanism is flagged
- [ ] 13.6 Implement sealed-set handling (aggregates only, marked in the run record) and create `lab/scenarios/sealed/README.md` explaining who writes sealed scenarios and who must not read them; verify a test that a sealed run prints no scenario content
- [ ] 13.7 Implement live mode (`--live`), with a pre-flight check for provider settings and separate reporting of structural and wording checks; verify a test that a live run without credentials stops before any scenario with a message naming the settings
- [ ] 13.8 Document the scenario format and the Lab workflow in `docs/lab.md`; verify the example scenario in it runs

## 14. End-to-end scenarios

- [ ] 14.1 Write dev scenario `recall-across-bodies`: a fact told through one test body, asked in other words through a second body thirty days later by the same person; verify it passes scripted and the context check finds the fact
- [ ] 14.2 Write dev scenario `promise-kept`: a reminder promised for two hours later, then silence; verify scripted that no action occurs before the due time, one occurs after, and the promise is resolved
- [ ] 14.3 Write dev scenario `group-silence`: three sources talk in a group without addressing the soul, then one addresses it; verify recorded silent decisions with zero model calls, then one reply
- [ ] 14.4 Write dev scenario `ask-group-report-back`: a source asks in private for the soul to ask a group something and report back, a group member answers; verify the answer's wake is in the same chain, its context contains the private request, and an action lands in the private place
- [ ] 14.5 Write dev scenario `identity-by-conversation`: the owner, then another source, asks for a new name; verify two done receipts and two identity-change events, each naming the right origin, and the new name in the next context
- [ ] 14.6 Write dev scenario `budget-and-brake`: a spent budget refuses ordinary wakes but lets a due promise through; a self-waking chain is stopped at the limit; with each rule switched off neither refusal occurs; verify all checks pass scripted
- [ ] 14.7 Add the Lab metric "items from a narrower audience placed in a wider context" computed from recorded audiences, and write dev scenario `private-surfaces` (something told in private is asked about in a group); verify the metric is reported as non-zero for it, as the baseline for the later privacy rule
- [ ] 14.8 Write two validation scenarios that vary 14.1 and 14.3 (different gap and distractors; different group size and order); verify they pass scripted without changing any mechanism
- [ ] 14.9 Add `cargo test` integration that runs the dev set scripted and replays each resulting log; verify it passes in under 30 seconds
- [ ] 14.10 Run the dev set live against a configured OpenAI-compatible model and embedding endpoint; verify all structural checks pass, and commit the run record and a short note of wording pass rates and token cost to `lab/runs/reference/`
- [ ] 14.11 Run `lab compare` for every registered mechanism over the dev and validation sets; verify a report is written to `lab/runs/reference/ablation.md` listing each mechanism's metric differences and any flagged as having no measured effect

## 15. Research track and documents

- [ ] 15.1 Create `research/backlog.md` with the ten entries of design Decision 15, each with question, why it matters, the Lab measurement that would answer it and status; verify every entry names a scenario or metric
- [ ] 15.2 Reconcile `RESEARCH.md` with `ARCHITECTURE.md`: host → gateway, hooks → doors, fix links to sections and files that do not exist, without changing formulas or sources; verify no reference in `RESEARCH.md` points to a missing file or section
- [ ] 15.3 Create `prior/references.json` with reference sentences for `info_gap` in English and Indonesian; verify the gateway computes reference vectors from it at start-up in a test with the hash embedder
- [ ] 15.4 Check the 2026 preprint citations flagged in `RESEARCH.md` against arXiv and mark each as verified or corrected in the backlog; verify no "title to verify" note remains without a status

## 16. Integration checks

- [ ] 16.1 Run `cargo fmt --check`, `cargo clippy --workspace -- -D warnings` and `cargo test --workspace` from a clean checkout; verify all pass and the three time budgets hold
- [ ] 16.2 Manual check of `monoagens chat` with a real model: tell a fact, quit, restart, ask about it in other words, ask for a reminder in two minutes and wait; verify recall after restart and an unprompted reminder, then confirm both with `monoagens log --explain`
- [ ] 16.3 Run `openspec validate bootstrap-vertical-slice --strict`; verify it reports the change as valid

## Workflow follow-up

- Decide the license before the first public release (design Open Questions).
- Archive the change after review, then plan the next changes: the policy layer on top of the core (the two absolute lines, privacy by audience, roles and permissions) as rules in the existing doors, installed plugins over a process transport, the first chat body, and sleep and consolidation.

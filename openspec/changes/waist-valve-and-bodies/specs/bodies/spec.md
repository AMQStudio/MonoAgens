# Spec Delta

## Purpose

Specifies canonical reference bodies and ecosystem bridges implementing Waist 2
(`monoagens.body.v1`), including in-process `body-cli`, `body-mcp-bridge` (MCP `2026-07-28`),
Phase 2–3 A2A/ROS-2 bridges, Phase-3 `body-sim-actuator`, `< 50 ms` voice barge-in, and honest
multi-chunk delivery (`R-36`, `R-37`, `R-45`, `R-47`, `R-50`).

## ADDED Requirements

### Requirement: Canonical body-cli runs local terminal chat with honest feedback
The built-in `body-cli` SHALL register a terminal place, local account, and `chat.speak` capability,
communicate via `monoagens.body.v1` message semantics, and render chunked replies with `0 ms`
artificial delay (`d_art = 0 ms`) while displaying honest `1 s` `working_cue` and `10 s`
`progress_note` indicators.

#### Scenario: Terminal chat processes input, overdue promises, and honest progress cues
- **WHEN** the user sends messages or a promise matures while a long tool action runs past `10 s`
- **THEN** `body-cli` shows honest progress cues during execution, prints finalized text immediately with `0 ms` delay, and logs echoes accurately

### Requirement: External body-mcp-bridge adapts MCP 2026-07-28 natively
The `body-mcp-bridge` SHALL isolate MCP `2026-07-28` (and `2025-11-25` fallback) servers behind
virtual `mcp:<server_id>` places, mapping OAuth/elicitation `input_required` prompts to Waist-2
`held` receipts, bounded long-running task polling to single terminal receipts, and tool schema
updates to `capability_schema_hash` changes.

#### Scenario: MCP task polling and elicitation map cleanly to Waist-2 receipts
- **WHEN** a remote MCP server enters async task polling or returns an `input_required` prompt
- **THEN** `body-mcp-bridge` surfaces `held` or polls until completion and emits exactly one terminal `action/receipt`

### Requirement: A2A, ROS-2 bridges, and Phase-3 sim actuator obey Waist 2
The `plugin-a2a-bridge` (A2A `v1.0.1`), `plugin-ros2-bridge` (ROS 2 `Lyrical Luth`), and Phase-3
`body-sim-actuator` SHALL integrate exclusively via `monoagens.body.v1` methods and pass Body TCK
before production registration, keeping device credentials and RTOS control loops external to Core.

#### Scenario: Sim actuator exposes motion capabilities behind Waist 2 only
- **WHEN** `body-sim-actuator` registers continuous or physical capabilities in Phase 3
- **THEN** it receives `action/execute` and reports Option A cancellation or `done` receipts strictly via `monoagens.body.v1`

### Requirement: Embodied voice barge-in mutes playback within 50 ms on one receipt
Speech-capable bodies SHALL combine `Silero VAD` (`WebRTC VAD` fallback), acoustic echo
cancellation, and causal z-scored `f_modal` slots (zero emotion tags). Non-AEC speech with
`vad_prob >= 0.70` sustained for `>= 120 ms` SHALL mute local playback in `< 30 ms` (`< 50 ms`
total cutoff) and emit one `failed` receipt (`termination_reason = preempted_external`).

#### Scenario: Short noise click continues TTS while 200 ms speech preempts cleanly
- **WHEN** an `80 ms` backchannel click versus `200 ms` sustained user speech interrupts `speech.speak` at `42%` progress
- **THEN** the `80 ms` click continues playback while genuine barge-in mutes audio within `50 ms` emitting one receipt with `preempted_external` and `progress = 0.42`

### Requirement: Split chat bubbles correlate platform IDs and respect partial barge-in
Any chat body splitting text to platform cap `|c_m| <= L_cap` SHALL track every emitted platform
ID (`pid_1..pid_M`), halt unsent chunks on user barge-in, register all delivered IDs for `R-09`
self-echo and reply-to resolution, and emit one combined `action/receipt` reporting exact
`delivered_units`.

#### Scenario: Three-bubble output aborted cleanly after first chunk
- **WHEN** a user replies immediately after bubble `pid_1` of a three-bubble `say` action posts
- **THEN** chunks 2 and 3 are aborted and one `action/receipt` reports `delivered_units = 1` with `pid_1` registered for echo suppression

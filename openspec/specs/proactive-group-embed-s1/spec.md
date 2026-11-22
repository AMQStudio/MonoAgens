# proactive-group-embed-s1 Specification

## Purpose
TBD - created by archiving change proactive-group-embed-s1. Update Purpose after archive.

## Requirements

### Requirement: Live pre-queue EmbeddingGemma 2 enrichment in `monoagens chat`
`monoagens chat` SHALL enrich submitted `Input::Percept` inputs through `GatewayHandle::pre_queue_service()` whenever `[embed]` is enabled, auto-managing a local `embeddinggemma2_sidecar.py` process when `embed.backend = "embeddinggemma2_http"` specifies `model_path` without an external `endpoint_url`, and ensuring `embeddinggemma2_sidecar.py` applies `instruction_prefix` consistently with `TaskPrefix::Document`.

#### Scenario: Sidecar honors TaskPrefix instruction_prefix and CLI enriches percepts
- **WHEN** `monoagens chat` submits a percept with `[embed] backend = "embeddinggemma2_http"` enabled
- **THEN** the sidecar applies `instruction_prefix` (`"title: none | text: "`), `PreQueueService` attaches a `256d` `egemma2-256` `t_d` embedding to the percept, and `Core` computes non-zero semantic similarity (`max_visible_mem_cos`) across visible group memories

### Requirement: Asynchronous OpenWait wakeup rendering in interactive chat
Interactive `monoagens chat` SHALL poll lifecycle cues and rendered `body-cli` responses asynchronously while awaiting terminal input so proactive replies triggered after `WaitDeferral` (`OpenWait` expiry) are displayed automatically without requiring an extra keystroke.

#### Scenario: Delayed proactive reply prints after OpenWait expires
- **WHEN** a turn records `WaitDeferral` (`OpenWait`) and `GatewayRuntime` emits a `chat.speak` action after `due_ms` expires unanswered
- **THEN** the interactive terminal loop prints `Mono> ...` automatically and restores the active session prompt

### Requirement: Calibrated System 1 proactive helping on unanswered group questions
The neural training pipeline SHALL include unaddressed group proactive-helping trajectories (distinguishing unanswered information questions with high `max_visible_mem_cos` from statements, reactions, casual banter, and answered threads) encoded with real `EmbeddingGemma 2` vectors, calibrated via temperature scaling and `CIC`, and verified against Lab offline `development` and `validation` suites without `Delta_NI` or false-speak regressions.

#### Scenario: System 1 wakes proactively on unanswered question with visible memory while staying silent on chatter
- **WHEN** an unaddressed group question matches an earlier visible group statement (`max_visible_mem_cos >= 0.65`)
- **THEN** the calibrated V2.2 controller assigns high `Wait`/`Go` probability rather than `Note`/`Silent`, while casual statements and chatter without unanswered questions remain `Silent`

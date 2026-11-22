# cli-interactive-chat Specification

## Purpose
TBD - created by archiving change cli-interactive-chat-repl. Update Purpose after archive.

## Requirements

### Requirement: Interactive line-by-line chat REPL with backward-compatible batch mode
`monoagens chat` SHALL run a live line-by-line interactive REPL when `--messages` is empty and `stdin` is a terminal (or `--interactive` is passed), submitting each entered line immediately and printing replies without waiting for EOF, while preserving existing `--message`, `--json`, and piped non-TTY behavior.

#### Scenario: Interactive commands switch rooms, group audiences, and speakers live
- **WHEN** a user enters `/group room:ta owner,budi,sari` and `/as budi` followed by an unaddressed message and then a message mentioning `@mono` without pre-declaring `--actors`
- **THEN** the CLI dynamically registers simulated actors into the interactive Valve session, preserves the active `body-cli` channel `PlaceId` while displaying the group room label, builds percepts with the multi-party `Audience` and speaker `ActorId`, leaves `addressees` empty for casual group chatter, and populates `soul` in `addressees`/`named` when `@mono` or `@soul` is mentioned

#### Scenario: Batch `--message` and `--json` behavior remains backward compatible
- **WHEN** `monoagens chat --message "hello" --json` executes
- **THEN** it processes the provided messages non-interactively and emits the existing JSON payload schema without prompt text

### Requirement: Live neural decision trace summary and optional model path override
`monoagens chat` SHALL accept an optional `--model <path>` override for `soul.model_path` and SHALL display a compact per-turn neural status summary (`outcome`, `reason`, `compute_tier`, `p_go`, `p_user_correction`) when interactive mode or `--show-brain` is enabled without altering JSON output.

#### Scenario: Brain summary reflects recorded DecisionTrace per turn
- **WHEN** a chat turn completes with `--show-brain` enabled
- **THEN** the CLI prints the turn's recorded `DecisionOutcome`, `DecisionReason`, optional `ComputeTier`, and probabilities read from the event log

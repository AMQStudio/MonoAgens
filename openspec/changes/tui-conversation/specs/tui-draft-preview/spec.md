# Specification — `tui-draft-preview`

## ADDED Requirements

### Requirement: Draft Preview Only for the Submitting TUI
When the model streams, the gateway SHALL publish the incrementally extracted draft speech text on `preview.subscribe` only to the TUI that submitted the turn, and SHALL never give a preview channel to an adapter, robot or any other body.

#### Scenario: Adapter sees no draft
- **WHEN** a turn streams with a TUI subscriber and a fake adapter body is connected for the same place
- **THEN** the TUI receives draft updates and the fake adapter receives nothing before the final `chat.speak`.

### Requirement: Final Speak Is Unchanged
The final `chat.speak` SHALL be delivered only after the full structured output is parsed and passes the action door, and SHALL be identical to what a non-streaming run produces.

#### Scenario: Streaming equals non-streaming
- **WHEN** the same scripted response is run once streamed and once not streamed
- **THEN** the delivered `chat.speak` payloads are byte-for-byte equal.

### Requirement: Previews Are Not Events
Previews SHALL NOT be recorded as events, SHALL NOT enter the input queue and SHALL NOT influence any decision.

#### Scenario: Replay is unchanged
- **WHEN** a streamed turn is recorded and replayed with `monoagens replay`
- **THEN** the event log has no preview entries and replay reports identical decisions to a non-streamed run.

### Requirement: Draft Bubble Resolves to the Real Outcome
The TUI SHALL show the draft as a dim "draft" bubble and SHALL replace it with the real reply, the valve verdict when held or denied, or "Mono chose not to reply"; a cancel or provider failure SHALL remove it.

#### Scenario: Valve denies the speech
- **WHEN** the draft shows text and the action door then denies the speak action
- **THEN** the draft bubble is replaced by the valve verdict and the denied text stays visible nowhere else.

### Requirement: Live Draft Preview Setting
`Appearance -> Live draft preview` SHALL default to on, and when off the provider SHALL not request streaming and no preview SHALL be shown.

#### Scenario: Setting off
- **WHEN** the setting is off and a turn runs against the fake provider
- **THEN** the request body has no `stream: true` and no draft bubble appears.

### Requirement: Streaming Provider Dialects
`openai_chat` SHALL stream with SSE and `anthropic_messages` with its streaming events, and both SHALL record token usage exactly as the non-streaming path does.

#### Scenario: Usage recorded under streaming
- **WHEN** the fake server streams a response with usage in the final chunk
- **THEN** the recorded model call has the same token counts as the non-streamed response.

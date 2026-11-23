# Specification — `tui-turn-control`

## ADDED Requirements

### Requirement: Esc Twice Interrupts a Running Turn
While a turn is busy and no overlay is open, the first Esc SHALL show a prompt to press Esc again and a second Esc within 1.5 s SHALL call `turn.cancel`, end the busy state and show the muted notice "Interrupted.".

#### Scenario: Interrupt a slow turn
- **WHEN** a scripted turn is waiting on a delayed provider and the user presses Esc twice within 1.5 s
- **THEN** the busy state ends, "Interrupted." appears under the user message, and the event log contains a model failure with reason `cancelled_by_owner`.

#### Scenario: Single Esc does nothing
- **WHEN** the user presses Esc once and waits longer than 1.5 s
- **THEN** the turn continues and the prompt disappears.

### Requirement: Interrupt Reports When It Cannot Stop
When `turn.cancel` returns `cancelled: false`, the TUI SHALL say the turn could not be stopped and SHALL leave the busy state to end normally.

#### Scenario: No model call in flight
- **WHEN** the user interrupts while the turn is not waiting on a model call
- **THEN** a notice says it could not be stopped and the turn finishes on its own.

### Requirement: Queued Messages Survive an Interrupt
Messages queued before an interrupt SHALL stay queued and be submitted in order after the cancel completes.

#### Scenario: Queue after cancel
- **WHEN** one message is queued, then the running turn is interrupted
- **THEN** the queued message is submitted as the next turn.

### Requirement: Retry the Last Failed Turn
`F5` or `Alt+R` SHALL re-submit the user text of the last turn that ended in error or interrupt as a new turn, and SHALL do nothing when the last turn succeeded.

#### Scenario: Retry after error
- **WHEN** a turn fails with a provider error and the user presses `F5`
- **THEN** the same text is sent as a new turn, and pressing `F5` after a successful turn does nothing.

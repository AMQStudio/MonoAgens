# Specification — `tui-conversation-ux`

## ADDED Requirements

### Requirement: Composer History Search
`Ctrl+R` SHALL start a reverse incremental search over composer history, with Enter accepting the match and Esc restoring the previous draft.

#### Scenario: Search finds an older message
- **WHEN** history holds `alpha` and `beta`, and the user presses `Ctrl+R` and types `alp`
- **THEN** the composer shows `alpha`, and Enter accepts it for editing.

### Requirement: External Editor
`Ctrl+G` SHALL open the draft in `$VISUAL` or `$EDITOR` through a file under the data directory, restore the TUI afterwards and load the edited text into the composer, and SHALL show a toast when no editor is set.

#### Scenario: Editor returns text
- **WHEN** `$EDITOR` is a script that appends a line and the user presses `Ctrl+G`
- **THEN** the TUI returns to the alternate screen and the composer contains the appended line.

### Requirement: Cleared Draft Recall
After `Ctrl+C` clears the composer, Up on an empty composer SHALL recall the cleared draft before older history.

#### Scenario: Recall
- **WHEN** the user types `hello`, presses `Ctrl+C`, then Up
- **THEN** the composer shows `hello`.

### Requirement: Per-Turn What-Happened Block
Each completed turn SHALL show a collapsed summary line of model calls, capability steps and valve verdicts read from the event log, and `Ctrl+O` SHALL expand it into one row per item.

#### Scenario: Expand
- **WHEN** a scripted turn with one model call and one speak action completes and the user presses `Ctrl+O` on it
- **THEN** the block expands to a row for the model call and a row for the action with its valve verdict.

### Requirement: Message Actions
The user SHALL be able to select a message by keys or mouse and copy it, quote it into the composer with a `> ` prefix, or open `/why` to see the valve explanation for it.

#### Scenario: Quote
- **WHEN** the user selects a reply and presses `q`
- **THEN** the composer contains the reply text with each line prefixed by `> `.

#### Scenario: Why
- **WHEN** the user selects a reply and presses `w`
- **THEN** an overlay shows the same explanation content as `monoagens log --explain` for that event.

### Requirement: Notify When Unfocused
When the terminal is unfocused and a reply or approval prompt arrives, the TUI SHALL ring the bell, emit OSC 9 and OSC 777 notifications and set a title badge until focus returns, according to the setting `off`, `bell`, `desktop` or `both`.

#### Scenario: Reply while unfocused
- **WHEN** a FocusLost event is received, setting is `both`, and a reply arrives
- **THEN** output contains the bell, an OSC 9 and an OSC 777 sequence with the reply's first line, and the title contains a badge until FocusGained.

#### Scenario: Setting off
- **WHEN** the setting is `off` and a reply arrives while unfocused
- **THEN** no notification sequence is written.

### Requirement: Optional Image Preview
Where the terminal is detected to support kitty graphics or sixel, the TUI SHALL show an inline preview of image attachments, and SHALL keep the existing attachment card otherwise.

#### Scenario: No graphics support
- **WHEN** detection reports no graphics support
- **THEN** attachments render as the existing `WxH` card and no graphics escape is written.

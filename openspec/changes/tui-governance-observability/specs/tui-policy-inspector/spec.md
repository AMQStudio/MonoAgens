# Specification — `tui-policy-inspector`

## ADDED Requirements

### Requirement: Read-Only Policy View
`/policy` SHALL show the valve toggles with their state, the two absolute lines, the active Cedar rules and the RBAC summary, with no editing actions.

#### Scenario: Toggle state shown
- **WHEN** a valve toggle is off in the configuration and `/policy` opens
- **THEN** the view lists it as off and offers no edit key.

### Requirement: Matches the CLI Inspector
The content of `/policy` SHALL be produced from the same data as `monoagens inspector`.

#### Scenario: Same rules as the CLI
- **WHEN** both are run against the same data directory
- **THEN** the set of Cedar rule identifiers listed is identical.

### Requirement: Rule Source Text
Pressing Enter on a rule SHALL show its source text.

#### Scenario: Open a rule
- **WHEN** the user presses Enter on a listed rule
- **THEN** its source text is displayed and Esc returns to the list.

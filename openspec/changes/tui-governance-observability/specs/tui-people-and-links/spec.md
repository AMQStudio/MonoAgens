# Specification — `tui-people-and-links`

## ADDED Requirements

### Requirement: People View
`/people` SHALL list persons with their platform accounts and roles.

#### Scenario: Person with two accounts
- **WHEN** a person has a WhatsApp and a Telegram account in the configuration
- **THEN** `/people` shows the person with both accounts and their role.

### Requirement: Link Initiate and Verify
`/people` SHALL initiate and verify an account link with the same behaviour as `monoagens link`, showing the instructions or code it produces.

#### Scenario: Initiate then verify
- **WHEN** the user initiates a link for a person and then verifies with the code
- **THEN** the account appears under that person and the result matches `monoagens link` on the same data.

### Requirement: Unlink With Confirmation
Unlinking an account SHALL ask for confirmation and SHALL change nothing when declined.

#### Scenario: Decline unlink
- **WHEN** the user chooses unlink and declines
- **THEN** the account remains linked.

### Requirement: Owner-Only Changes
Link changes SHALL be accepted only from the owner's connection.

#### Scenario: Non-owner connection rejected
- **WHEN** a link method is called from a connection that is not the owner's admin socket or in-process client
- **THEN** it returns an error and changes nothing.

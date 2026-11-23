# Specification — `guest-access`

## ADDED Requirements

### Requirement: Unregistered Senders Are Admitted as Guests
When guest access is on (default on, `[valve] guest_access = true`), the inbound door SHALL admit a message from an adapter actor that is not a registered person, attesting the role named by `[roles] fallback` (default `guest`), instead of refusing it as an unknown audience; with guest access off the current refusal SHALL remain. The first message of a new guest SHALL be recorded so the guest is recognised after a restart without process memory.

#### Scenario: New WhatsApp sender
- **WHEN** guest access is on and `wa:user:6285111242960@s.whatsapp.net` (no `[[person]]`) sends `halo`
- **THEN** the inbound door verdict is allow with role `guest`, not `refuse: unknown_audience`, and after a restart the same sender is still recognised as a guest.

### Requirement: RBAC Decides Whether a Guest May Use the Agent
Waking the agent for a message SHALL require the permission `agent.converse`, resolved by RBAC like any other permission (role allow/deny lists and `[permission_bases]`); guests SHALL NOT have it unless the owner grants it. A message refused for this reason SHALL be recorded with reason `agent_not_permitted` and SHALL NOT reach the model.

#### Scenario: Guest without permission
- **WHEN** a guest without `agent.converse` sends `halo`
- **THEN** no model call is made and the event log shows `agent_not_permitted` for that message.

#### Scenario: Owner grants guests the agent
- **WHEN** the owner adds `agent.converse` to the `guest` role allow list in the RBAC editor
- **THEN** the next guest message wakes the agent and gets a reply.

### Requirement: Gateway Commands Answer Guests Without the Model
`/help`, `/whoami` and `/link` sent from an adapter SHALL be answered by the gateway with zero model calls for any admitted sender, including guests without `agent.converse`, listing only the commands and tools their permissions allow.

#### Scenario: Guest asks whoami
- **WHEN** a guest sends `/whoami`
- **THEN** the reply names their account, role `guest`, and whether they may use the agent, and no model call is made.

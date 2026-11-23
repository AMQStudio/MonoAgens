# Specification — `people-editor`

## ADDED Requirements

### Requirement: People and Accounts Are Editable in the TUI
`/people` SHALL let the owner create and delete persons, mark a person as owner (exactly one owner), add and remove platform accounts (`body_id` + `account_id`) picked from senders seen in the event log or typed, and assign roles; every change SHALL be written through `config.apply` to `[[person]]` and take effect on the next message without a restart. The settings coverage test SHALL include the `[[person]]` owner and accounts keys.

#### Scenario: Owner adds their WhatsApp account
- **WHEN** the owner opens `/people`, selects the owner person and adds the recent sender `wa:user:6285111242960@s.whatsapp.net` for `body-whatsapp`
- **THEN** `[[person]]` for the owner lists that account, and the next WhatsApp message from it is handled as the owner.

#### Scenario: Promote a guest
- **WHEN** the owner picks a guest from recent senders and creates a person with role `family`
- **THEN** that sender's next message is attested with role `family`, not `guest`.

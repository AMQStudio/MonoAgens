# Specification — `adapter-identity-stability`

## ADDED Requirements

### Requirement: WhatsApp Account Identity Is Stable Across LID Addressing
The WhatsApp adapter SHALL report a DM sender and a DM place by the contact's phone-number account whenever WhatsApp provides it (the message's alternate sender JID, or the client's LID-to-phone mapping), and SHALL fall back to the LID only when no phone number is known, so the same person keeps the same account id regardless of WhatsApp's addressing mode.

#### Scenario: LID-addressed DM from a known contact
- **WHEN** a DM arrives with sender `130799689031758@lid` and alternate sender `6285111242960@s.whatsapp.net`
- **THEN** the percept's actor is the account for `6285111242960`, its place is the DM place for `6285111242960`, and the reply is delivered to that chat.

#### Scenario: LID with no known phone number
- **WHEN** a DM arrives from a LID with no alternate sender and no stored mapping
- **THEN** the percept uses the LID account and the adapter logs one info line naming the LID so the owner can link it.

### Requirement: Group Participants Use the Same Stable Accounts
In groups the adapter SHALL map each participant and the sender to the phone-number account when known, using the same rule as DMs, so audience members and actors match the DM identities.

#### Scenario: Group sender with LID
- **WHEN** a group message arrives with a LID sender whose phone number is known
- **THEN** the actor and the matching audience member are the phone-number account.

# Spec Delta

## Purpose

Ensures direct messages and bot mentions from chat adapters address the soul so permitted guests wake the agent and communicative reply capabilities remain disclosed under Tool RAG budgets.

## ADDED Requirements

### Requirement: Adapter DMs Address the Soul
WhatsApp and Telegram 1:1 direct message percepts SHALL include `ActorId("soul")` in both `addressees` and `audience`. Group messages mentioning or replying to the bot SHALL include `ActorId("soul")` in `addressees`, while unaddressed ambient group chatter SHALL remain unaddressed.

#### Scenario: WhatsApp guest DM wakes the agent without neural weights
- **WHEN** guest access is enabled, `guest` role allows `agent.converse` and `chat.speak`, no neural gate weights are loaded (`soul.model_path` unset), and an unregistered WhatsApp sender sends `"testtt"` in a 1:1 DM
- **THEN** `InboundTranslator` sets `addressees = ["soul"]` and includes `"soul"` in `audience`, Core computes `turn_allocated = true`, wakes the model, and delivers the reply to WhatsApp.

#### Scenario: Telegram 1:1 DM addresses the soul while ambient group messages do not
- **WHEN** a Telegram user sends a 1:1 DM to the bot vs an unmentioned message in a group chat
- **THEN** the DM percept includes `"soul"` in `addressees` and `audience`, while the unmentioned group message leaves `addressees` empty.

### Requirement: Communicative Reply Capability Is Always Disclosed
`ToolRagIndex::to_core_catalog` SHALL mark `chat.speak` capabilities as `is_static_core: true` so the active place's reply capability is always disclosed to the model regardless of total registered capability count or `k_cap`.

#### Scenario: Short guest DM with 28 registered capabilities
- **WHEN** 28 capabilities are registered across multiple plugins and a guest sends a short DM `"testtt"`
- **THEN** `chat.speak` is disclosed in the context catalog and action dispatch does not fail with `capability 'chat.speak' not disclosed`.

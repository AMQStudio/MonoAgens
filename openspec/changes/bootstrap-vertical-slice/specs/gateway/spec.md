# Spec Delta

## Purpose

The gateway is the home between the core and everything outside. It runs plugins, routes
actions to the body that owns a place, maps accounts to people, loads the configuration and
talks to model providers, while knowing no platform and translating nothing.

## ADDED Requirements

### Requirement: The core is reached only through three doors
The gateway SHALL act on the core only through the inbound door (events going in), the context
door (what reasoning is shown) and the action door (acts going out). Every percept, every
context and every action SHALL pass its door.

#### Scenario: An action leaves the core
- **WHEN** reasoning uses a capability
- **THEN** the action passes the action door before any body receives it

#### Scenario: A context is built
- **WHEN** reasoning is about to be called
- **THEN** the assembled context passes the context door before the model sees it

### Requirement: Plugins register capabilities and places
A plugin SHALL register with the gateway the places its body owns and the capabilities it
offers. The gateway SHALL route each action to the body that owns the action's place.

#### Scenario: Two bodies are running
- **WHEN** an action targets a place owned by the second body
- **THEN** only the second body receives the action

#### Scenario: No body owns the place
- **WHEN** an action targets a place no running body owns
- **THEN** the action ends in a receipt with outcome failed and a reason, and nothing is sent

### Requirement: Accounts map to one id per person
The gateway SHALL map the accounts that bodies report to one id per person, as configured, so
that the same person through different bodies is the same source to the core. An account with
no mapping SHALL get its own stable id.

#### Scenario: The same person on two bodies
- **WHEN** two accounts on different bodies are configured as one person
- **THEN** events from both carry the same source id to the core

#### Scenario: An unknown account
- **WHEN** an account with no configured mapping appears twice
- **THEN** both events carry the same new source id, and that id differs from every configured person

### Requirement: Features are attached before the core sees an event
The gateway SHALL attach to each percept the numeric features the core needs, including an
embedding vector when an embedding provider is configured, before the event enters the queue.

#### Scenario: The embedding provider is down
- **WHEN** the embedding provider fails for a percept
- **THEN** the percept still enters the queue, without a vector, and the failure is recorded

### Requirement: A broken plugin does not stop the soul
A plugin that fails while running SHALL be left out, and the owner SHALL be told. A broken
configuration, or a built-in plugin that fails at start-up, SHALL stop start-up with a clear
error.

#### Scenario: A body crashes while running
- **WHEN** a running body fails
- **THEN** the loop keeps running, actions for that body's places end in receipts with outcome failed, and the failure is reported to the owner

#### Scenario: The configuration is invalid
- **WHEN** the configuration cannot be parsed or fails validation
- **THEN** the program does not start and prints the location and reason of the first error

### Requirement: The gateway never learns
The gateway SHALL keep nothing across a restart except as events in the log or as owner-set
configuration.

#### Scenario: Restart after an unknown account was seen
- **WHEN** the program restarts after assigning an id to an unknown account
- **THEN** the same account receives the same id again

### Requirement: One versioned configuration file
Everything the owner sets SHALL live in one configuration file that carries a version. A file
with a newer version than the program knows SHALL stop start-up with a clear error.

#### Scenario: A configuration from a newer program
- **WHEN** the configuration's version is newer than the program supports
- **THEN** the program does not start and names both versions

### Requirement: Secrets live apart from the configuration
Keys and secret endpoints SHALL be read from the environment or from a separate secrets file,
never from the configuration file.

#### Scenario: A key is put in the configuration
- **WHEN** the configuration file contains a field for a provider key
- **THEN** start-up fails with an error that says where secrets belong

### Requirement: Model providers are interchangeable
The gateway SHALL reach the language model and the embedding model through provider
interfaces, with at least a provider for OpenAI-compatible endpoints and a scripted provider
that returns prepared outputs. Changing the provider SHALL require configuration only.

#### Scenario: The scripted provider is selected
- **WHEN** the configuration selects the scripted provider
- **THEN** the program runs a full loop without any network access

#### Scenario: A provider call fails
- **WHEN** the model provider returns an error or times out
- **THEN** the wake is recorded as failed with the provider's error class, and the loop continues

### Requirement: Every hook and action runs under a time limit
Each call into a plugin and each action handed to a body SHALL run under a time limit.

#### Scenario: A body hangs on an action
- **WHEN** a body does not return from an action within the limit
- **THEN** the action ends in a receipt with outcome unknown and the loop continues

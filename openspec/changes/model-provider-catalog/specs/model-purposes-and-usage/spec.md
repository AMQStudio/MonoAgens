# Specification — `model-purposes-and-usage`

## ADDED Requirements

### Requirement: Optional Model per Purpose
When `[model.purposes.<purpose>]` names a provider and model, MonoAgens SHALL use it for calls of that purpose and the `[model]` pair for all others; the setting SHALL be absent by default.

#### Scenario: Classify uses a small model
- **WHEN** `[model.purposes.classify]` sets model `small` and the soul makes a classify call and a reasoning call
- **THEN** the classify request carries `small` and the reasoning request carries the `[model]` model.

### Requirement: Titler Follows the Classify Model
The titler SHALL use the classify purpose's model when set, otherwise the main model.

#### Scenario: Titler model
- **WHEN** a classify model is configured and a chat is titled
- **THEN** the title request carries the classify model id.

### Requirement: Cost Shown Only When a Price Is Known
The status bar SHALL show cost only when a price is known from discovery or from user-entered per-model prices, and SHALL hide the cost otherwise instead of showing `$0.00`.

#### Scenario: No price known
- **WHEN** the active model has no price and a turn completes
- **THEN** the status bar shows no cost segment.

#### Scenario: Price from discovery
- **WHEN** the fake discovery response includes input and output prices for the active model and a turn uses known token counts
- **THEN** the status bar cost equals tokens times price.

### Requirement: Usage View
`/usage` SHALL show tokens in and out per purpose and the budget left.

#### Scenario: Usage after two calls
- **WHEN** one reasoning call and one classify call have completed
- **THEN** `/usage` lists both purposes with their token counts and shows the remaining daily budget.

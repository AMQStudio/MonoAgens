# Spec Delta

## Purpose

The `monoagens` program is how an owner and a developer meet the soul: talking to it through
the terminal body, running the Lab, replaying a log and reading what the soul decided and why.

## ADDED Requirements

### Requirement: The owner can talk to the soul in a terminal
`monoagens chat` SHALL start the soul with the terminal body and let the person at the terminal
exchange messages with it. The terminal's user SHALL be the owner unless configured otherwise.

#### Scenario: A first conversation
- **WHEN** the owner runs `monoagens chat` with a configured model and types a message
- **THEN** the soul's reply appears in the terminal and the exchange is in the log

#### Scenario: The soul speaks first
- **WHEN** a promise falls due while `monoagens chat` is open
- **THEN** the soul's message appears in the terminal without the owner typing anything

### Requirement: The soul remembers between runs
State SHALL persist across runs of `monoagens chat` in the configured data directory.

#### Scenario: Chatting again the next day
- **WHEN** the owner closes `monoagens chat`, opens it again later and asks about something said before
- **THEN** the soul can answer from what it recorded

### Requirement: First run needs no setup beyond a model
With no configuration present, `monoagens chat` SHALL create a default configuration and data
directory, state where they are, and state which environment settings a model needs if they
are missing.

#### Scenario: Nothing is configured
- **WHEN** `monoagens chat` runs on a machine with no configuration and no model settings
- **THEN** it creates the default configuration, prints its location and the missing model settings, and exits with a non-zero status

### Requirement: The Lab is run from the program
`monoagens lab run` SHALL run a scenario or a scenario set with the scripted or the live
provider, with optional ablations and seed, and SHALL exit with a non-zero status when any
check fails.

#### Scenario: Running the development set
- **WHEN** `monoagens lab run` is given the development set
- **THEN** it prints each scenario's result and a summary, writes a run record and exits with status zero only if all checks passed

### Requirement: A log can be replayed
`monoagens replay` SHALL replay a recorded log without calling a model and SHALL report whether
every decision was reproduced.

#### Scenario: A clean replay
- **WHEN** a log recorded by this version is replayed
- **THEN** the program reports that all decisions were reproduced and exits with status zero

### Requirement: Decisions can be inspected
`monoagens log` SHALL show recorded events in order, filterable by place, source, kind and
time, and SHALL be able to explain a single decision with its numbers.

#### Scenario: Why did it stay silent
- **WHEN** the owner asks to explain a decision event with outcome silent
- **THEN** the program prints the signals, salience, habituation, threshold terms, probability and draw of that decision

### Requirement: Mechanisms can be listed
`monoagens lab mechanisms` SHALL list every registered mechanism with its id, claim, neutral
value and current value.

#### Scenario: Listing the brain map
- **WHEN** `monoagens lab mechanisms` is run
- **THEN** each mechanism of the core appears once with its claim and neutral value

### Requirement: Output is usable by programs
Commands that report results SHALL offer a machine-readable output format, and errors SHALL go
to the error stream with a non-zero exit status.

#### Scenario: A script reads Lab results
- **WHEN** `monoagens lab run` is run with the machine-readable option
- **THEN** standard output contains only the structured result

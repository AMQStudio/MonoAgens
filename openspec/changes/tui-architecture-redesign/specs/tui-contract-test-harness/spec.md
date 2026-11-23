# Spec Delta

## Purpose

Eliminates Cargo test link duplication and brittle full-screen snapshot churn by consolidating TUI integration tests into one binary with behavior-focused `TestBackend` contract assertions.

## ADDED Requirements

### Requirement: Single Consolidated Integration Test Binary and Behavior-Focused Contract Tests
`crates/tui/tests/` SHALL be consolidated into one integration test binary (`crates/tui/tests/it/main.rs`) sharing `tests/it/common/mod.rs` and domain submodules. Tests SHALL inject deterministic `Clock` and `Clipboard` doubles and assert observable contracts on `ratatui::backend::TestBackend` buffers across `40x12` and `80x24` viewports, replacing brittle full-screen ASCII snapshots.

#### Scenario: Single integration test link and contract assertions pass without `.snap` churn
- **WHEN** `cargo test -p monoagens-tui` runs
- **THEN** Cargo links one integration test binary (`it`), verifies focus isolation, paste sanitization across all inputs, mouse click/wheel/scrollbar drag, RBAC Simple ON/OFF toggle TOML round-trips, friendly label rendering/search, and `40x12`/`80x24` footer integrity without brittle full-screen ASCII snapshots.

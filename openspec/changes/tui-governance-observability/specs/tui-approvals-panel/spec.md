# Specification — `tui-approvals-panel`

## ADDED Requirements

### Requirement: Approvals Panel Lists All Pending Approvals
`/approvals` SHALL list every pending approval from `approvals.list`, including actions held during adapter-originated turns, with capability, place, requesting actor label, age and origin.

#### Scenario: Adapter hold appears
- **WHEN** a fake adapter turn leads to a held action requiring approval
- **THEN** `/approvals` lists it with the adapter's plugin id as origin.

### Requirement: Grant and Deny With Confirmation
From the panel `g` and `d` SHALL ask for confirmation and then resolve the approval through `approvals.resolve` using the same path as `monoagens approvals grant` and `deny`.

#### Scenario: Grant
- **WHEN** the user selects a pending approval, presses `g` and confirms
- **THEN** the approval leaves the list and the held action proceeds through the valve exactly as after `monoagens approvals grant`.

#### Scenario: Declined confirmation
- **WHEN** the user presses `d` and declines the confirmation
- **THEN** the approval stays pending.

### Requirement: Pending Count Badge
The status bar SHALL show the pending approval count while it is greater than zero and SHALL remove it at zero.

#### Scenario: Badge follows the count
- **WHEN** two approvals are pending and the user resolves one and then the other
- **THEN** the badge shows 2, then 1, then disappears.

# Specification — `tui-rbac-editor`

## ADDED Requirements

### Requirement: Role Creation and Editing
The RBAC tab SHALL let the user create, edit and delete roles with base tier, approval mode (including inherit), label, context, allow and deny lists, and settings, creating `[rbac]` on first edit.

#### Scenario: Create a role
- **WHEN** the user creates role `family` with base `trusted` and adds `chat.speak` to its allow list
- **THEN** `[rbac.roles.family]` is written with those values and the valve uses it on the next turn.

### Requirement: Capability Picker for Allow and Deny
Allow and deny lists SHALL offer a picker over the capability ids known from connected bodies plus free-text entry.

#### Scenario: Pick from discovered capabilities
- **WHEN** a connected body reports capability `chat.speak` and the user opens the allow picker
- **THEN** `chat.speak` is listed and selecting it adds it to the list.

### Requirement: Referenced Roles Cannot Be Deleted
The editor SHALL refuse to delete a role referenced by the fallback role or any actor assignment and SHALL name where it is referenced.

#### Scenario: Delete refused
- **WHEN** the user deletes a role that is the fallback role
- **THEN** the role remains and a message says it is the fallback role.

### Requirement: Fallback Role Selection
The RBAC tab SHALL set the fallback role with a selector over existing roles.

#### Scenario: Change fallback
- **WHEN** the user selects role `guest-plus` as fallback
- **THEN** `[rbac].fallback_role` is `guest-plus`.

### Requirement: Actor Role Assignment
The editor SHALL list `actor_roles`, add an actor by picking one seen in the event log or typing an id, and assign one or more roles.

#### Scenario: Assign a role to a seen actor
- **WHEN** the user picks an actor from the seen-actor list and assigns `family`
- **THEN** that actor's `actor_roles` entry contains `family`.

### Requirement: Place Rules and Permission Bases
The editor SHALL edit allow and deny lists per place id and the minimum tier per capability id (a tier or owner-only).

#### Scenario: Owner-only capability
- **WHEN** the user sets capability `tool.shell` to owner-only
- **THEN** `[rbac].permission_bases` stores owner-only for it.

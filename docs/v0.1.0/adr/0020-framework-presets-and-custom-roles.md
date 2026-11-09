# ADR-0020: Framework, Presets, and Custom Roles — One Soul Engine, Many Products on Top

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** Operators use the harness for different products: a personal assistant, an organization assistant, customer service, a personal trainer, and more. Today the core ships a generic-assistant toolset and hardcoded role names (`administrator`, `member`, `guest`). This ADR positions MonoAgens as a framework: the core is the Soul engine, and products are presets of gateways, tool packages, plugins, roles, and a soul template on top.
- **Supersedes:** in part ADR-0006 §4 (role seed of 4.1: fixed role names), ADR-0008 §4 (the `administrator` row of the canonical term register in 4.6 becomes the `protected` flag)
- **Related:** ADR-0011 (charter), ADR-0014 (gateways, tools, plugins), ADR-0018 (roadmap)
- **Implementation:** draft

---

## 1. Context

- `roles.yaml` seeds `administrator`, `member`, and `guest`; code and prompts refer to these
  names (`administrator` is the protected top role in AGENTS.md and ADR-0008).
- Tools such as file editing, shell, and cross-room messaging are core and always present,
  whatever the deployment is for.
- RBAC matters whenever more than one person talks to the Soul: a personal assistant in a group
  must not let strangers run its owner's tools, and customer service must isolate customers
  from each other.

## 2. Decision drivers

- The core contains only what every product needs: one Soul, memory, privacy, moral and RBAC,
  flows, durability, gateways.
- Few choices for the operator: a working product in one step, details later.
- Role names belong to the operator's domain language, not to the code.
- Safety does not depend on naming: the protected role and the default for unknown senders are
  flags.

## 3. Options considered

1. **Keep a generic assistant as the product.** Rejected: other products would fight the
   defaults.
2. **Empty framework without presets.** Rejected: nobody knows where to start.
3. **Framework core plus a small number of presets (chosen).**

## 4. Decision

### 4.1 Positioning

MonoAgens is a framework for persistent single-Soul agents: identity, memory, privacy, moral
rules and RBAC, flows, durability, and many gateways. Products are built on top with gateways
(the Soul's apps), tool packages (its instruments), plugins (added abilities), roles, and a
soul template (ADR-0014).

### 4.2 Presets

A preset is plain configuration outside the core: a soul template, the enabled tool packages
and plugins, the roles, and the gateway policy. Version 1 ships two:

| Preset | Content |
|---|---|
| Assistant | core + `assistant` tool package; roles `owner` (protected) and `visitor` (default for unknown, chat only) |
| Blank | core only; roles `operator` (protected) and `visitor` (default for unknown, no tools); the starting point for other products |

`monoagens init` asks for one preset and creates everything; the panel can change any part
later. Further presets (customer service, coaching, organization) are added when a real
deployment needs them, and anyone can write and share their own.

### 4.3 Custom roles

- Role names are free. The code never refers to a role by name.
- Exactly one role carries `protected: true`: the top role that cannot be deleted, demoted, or
  locked out; it alone may hold `shell` and approve operator-level changes.
- Exactly one role carries `default_for_unknown: true`: the role of any sender not yet known;
  fail closed, it starts with no tools.
- Validation refuses a roles file without exactly one of each.
- Every other role is defined by the operator: tools, commands, gateway policy (ADR-0014),
  rooms, memory administration.

### 4.4 What this changes elsewhere

- ADR-0011 §4: core tools and commands are only those operating core mechanisms.
- ADR-0014 §4: gateways, tool packages, and plugins are the three packaging concepts.
- AGENTS.md §5 ("`administrator` is the protected top role") and the canonical term register
  are updated to the `protected` flag when this ADR is implemented.
- README is repositioned from "assistant" to "framework plus presets" at the release rebaseline
  (ADR-0018).

## 5. Configuration

`roles.yaml`, generated with comments (names are examples):

```yaml
roles:
  # Role names are yours to choose. Two roles are special and marked by a flag, not by name.
  owner:
    protected: true          # Exactly one role: the top role. Cannot be deleted or locked out.
                             # Only this role may use shell and approve operator-level changes.
    tools: ['*']
    commands: ['*']
  visitor:
    default_for_unknown: true  # Exactly one role: anyone the Soul does not know yet.
                               # Starts with no tools (fail closed); add only what strangers may use.
    tools: []
    commands: [help, link, whoami]
  staff:                     # Any other role: name, tools, commands, and gateway policy are free.
    tools: [read, web_search]
```

`config.yaml` records the preset it was created from (`preset: assistant`) for information
only; nothing reads it at runtime.

## 6. Metrics and gates

- No role name appears in core code, prompts, or tests outside fixtures (selfcheck).
- Role validation tests: zero or two `protected` roles fail; zero or two
  `default_for_unknown` roles fail.
- `monoagens init` with each preset produces a deployment that passes the golden scenarios
  relevant to it.

## 7. Acceptance criteria

- Renaming every role in `roles.yaml` changes no behavior.
- The Blank preset runs a Soul with only core tools and commands.
- The Assistant preset reproduces today's assistant behavior through the `assistant` package.

## 8. Risks and mitigations

- **Operators lock themselves out.** Mitigation: the protected role cannot be removed or
  demoted; validation refuses such a file.
- **Presets multiply.** Mitigation: two in version 1; more only with a real deployment behind
  them.
- **Existing deployments use fixed names.** Mitigation: pre-release; a migration maps
  `administrator` to `protected: true` and `guest` to `default_for_unknown: true`.

## 9. Interaction with other ADRs

- ADR-0006: RBAC stays in the core; fixed role names go.
- ADR-0008: the register entry for `administrator` becomes the `protected` flag.
- ADR-0011: the core charter is what every preset shares.
- ADR-0014: presets enable gateways, tool packages, and plugins.
- ADR-0018: presets and repositioning are part of the roadmap.

## 10. Code references

- `monoagens/engine/access/roles.py`, `roles.yaml`, `monoagens/admin/templates.py`,
  `monoagens/admin/init.py`, `monoagens/admin/wizard.py`.
- `monoagens/engine/tools/`, `monoagens/engine/commands/`.

*This ADR lives at `docs/v0.1.0/adr/0020-framework-presets-and-custom-roles.md`.*

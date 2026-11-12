# Plugins — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the core
([constitution §1](constitution.md)). Decision numbers (Dxx) trace to the history in git. Work:
[tasks/](tasks/README.md).

## 1. What a plugin is
A feature built on the middleware (a *package* in core.md). A plugin is a folder that bundles any
of: tools, middleware, commands, installed skills, standing rules, knob-offset bounds, a
state namespace and dependencies, declared in one manifest (§3). It reaches the core only through
the middleware's six levers ([middleware.md §2](middleware.md)) (D9, D41).
- Tools (search, files, email) are shipped by plugins (D35).
- Role plugins (assistant, customer service, sysadmin) may ship a birth certificate, used only at
  birth; they can never overwrite identity afterwards (D20, D35).
- Installed, run and uninstalled by the install module ([middleware.md §5](middleware.md)).
  Uninstalling removes the plugin's tools, middleware, rules and knob offsets; its events stay
  in the log as history.

## 2. Catalog
| Plugin | Kind | Status | Levers it uses |
|---|---|---|---|
| modes (daily / professional) | behavior | example | rules in `<self>`, notes, knob offsets, `set_mode` |
| memory-view & manage | feature | **Draft** | a command and a tool to list and correct memories |
| web search / fetch | tool | planned | a tool |
| files | tool | planned | a tool |
| simulated emotions | behavior | idea | own state via events, notes, rules, knob offsets |
| roles (assistant, customer service, sysadmin) | role | idea | rules, skills, maybe a birth certificate |
| plugin suggestions | feature | **Draft** | a note to the owner from the local catalog |
| moderation, content filters, rate limits | feature | idea | pass/replace/stop |

- **memory-view & manage** (Draft, D37): list what the soul remembers about the asker or this
  place; corrections via `revise` with the asker as `origin_actor`; owner-only Markdown export as
  a view, never the source of truth; both absolute lines apply.
- **Plugin suggestions** (Draft, D28): only to the owner, only from the local catalog, at most
  once per missing capability per habituation window, never installs anything.

## 3. Manifest (Draft)
One file at the root of a plugin declares everything it brings (D41). The install module reads
it; the core never does.

```yaml
name: work-modes
version: 0.1.0
state_namespace: work-modes          # its state keys are work-modes:* (projected from its events)
dependencies: []                     # other plugins that must load first; missing or failing fails boot
tools: [modes_tool]                  # e.g. offers a `set_mode` capability
middleware:
  - { hook: context, scope: shared, entry: modes.annotate }   # attaches "mode: professional"
commands: [mode]
skills: []
rules:                               # standing rules shown in <self>, in this order
  - "In professional mode, keep replies short and formal."
knob_offsets:                        # bounded, temporary; recorded as disposition.updated
  talkativeness: { min: -0.2, max: 0.0 }
```

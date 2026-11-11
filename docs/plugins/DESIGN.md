# Plugins — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. Builds on the core and may never change it ([core design](../core/DESIGN.md),
[core formats](../core/FORMATS.md), frozen).

## 1. What a plugin is
A feature built on the middleware. A plugin is a folder that bundles any of: organs (tools),
middleware, commands, installed skills, standing rules, knob-offset bounds, a state namespace and
dependencies, declared in one manifest ([FORMATS.md](FORMATS.md)). The install module reads the
manifest; the core only sees the resulting senses, organs and hooks. (The core documents call a
plugin a *package*.)

## 2. Catalog (planned and example plugins)
| Plugin | Kind | Status | Levers it uses |
|---|---|---|---|
| memory-view & manage | feature | Draft | a command and organ to list and correct memories; `revise` |
| modes (daily / professional) | behavior | example | rules in `<self>`, notes, knob offsets, `set_mode` |
| simulated emotions | behavior | idea | own state via events, notes, rules, knob offsets |
| roles (assistant, customer service, sysadmin) | role | idea | rules, skills, maybe a birth certificate |
| web search / fetch | tool | Step 6 | a tool organ |
| files | tool | Step 6 | a tool organ |
| plugin suggestions | feature | Draft | a note to the owner from the local catalog |
| moderation, content filters, rate limits | feature | idea | pass/replace/stop |

## 3. Life cycle
Installed, run and uninstalled by the install module (middleware). Uninstalling removes the
plugin's organs, middleware, rules and knob offsets; its events stay in the log as history.

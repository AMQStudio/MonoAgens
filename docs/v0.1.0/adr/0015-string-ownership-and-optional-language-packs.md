# ADR-0015: String Ownership and Optional Language Packs — English Mandatory, Every Owner Keeps Its Strings

- **Status:** proposed
- **Date:** 2026-11-09
- **Deciders:** Muqsith (owner)
- **Context:** One core catalog holds strings for the core, every gateway, and persona, and a test requires every language to match English key for key, so no partial pack can exist and every feature costs a translation. This ADR makes English the only mandatory language, gives each owner its own strings, and makes other languages optional packs.
- **Supersedes:** in part ADR-0008 §4 (catalog parity of 4.4)
- **Related:** ADR-0011 (charter), ADR-0014 (package skeleton)
- **Implementation:** draft

---

## 1. Context

Audit evidence (`.audit/07-config-i18n-soul.md`, `.audit/04-access-commands-guard.md`):

- `tests/test_i18n.py:20` asserts full key-set equality between catalogs.
- 286 keys in `monoagens/lang/en.yaml` and `id.yaml`, including 24 persona-only keys and
  gateway UX strings such as the WhatsApp reaction hint.
- Command descriptions exist twice: an English literal in code and a catalog entry per language.
- `t()` prints the key when a string is missing and the raw template on a bad placeholder.
- Much of the catalog is prose the model could phrase in the user's language.

## 2. Decision drivers

- A developer writes English once and is done.
- A missing translation degrades gracefully, never breaks.
- Strings live with the code that shows them.
- Fewer static strings: the model already writes replies in the user's language.

## 3. Options considered

1. **Keep full parity.** Rejected: blocks contributions and causes burnout.
2. **Move translations to a separate repository.** Rejected for now: no translator community
   yet; keeping packs in-repo is the common practice at this size.
3. **English mandatory per owner, optional in-repo packs with fallback (chosen).**

## 4. Decision

### 4.1 Ownership

- Core: `monoagens/lang/en.yaml` holds only core mechanism notices and command descriptions.
- Each gateway and plugin: its own `lang/en.yaml` (ADR-0014 skeleton) for its UX and tool
  strings; the WhatsApp reaction hint moves to the WhatsApp gateway.
- Persona strings move to the expression plugin.
- Command descriptions are defined once, as catalog keys of their owner.

### 4.2 Packs and fallback

- Other languages are optional files beside the English one (`lang/id.yaml`); Indonesian stays
  in the repository as an optional pack.
- Lookup order: requested language, then English. A missing key in an optional pack is a
  non-blocking warning; a missing English key fails the gate.
- `t()` raises in tests and logs in production for missing English keys and bad placeholders.

### 4.3 Fewer static strings

- Mechanism notices carry structured data; gateways render them (ADR-0013).
- Prose the model can phrase (apologies, explanations) is not stored as catalog text.
- Configuration keys and values are language-neutral identifiers; natural language appears only
  in `soul.yaml` content and catalogs.

### 4.4 Language switching

`/language` changes the active language through the comment-preserving writer and refuses a
code without at least a core pack.

## 5. Configuration

- `soul.language` keeps selecting the active language; packs are discovered from files.

## 6. Metrics and gates

- English completeness per owner: gate.
- Optional pack coverage per owner: reported, not gated.
- No user-facing literal outside catalogs (selfcheck rule, unchanged in spirit).

## 7. Acceptance criteria

- Deleting `id.yaml` from any owner leaves every test green and the UI in English.
- The core catalog contains no gateway or persona key.
- Each command description exists in exactly one place.

## 8. Risks and mitigations

- **Mixed-language UI in partial packs.** Mitigation: coverage report; few static strings.
- **Strings scattered across packages.** Mitigation: one loader, one lookup function, one
  coverage report across owners.

## 9. Interaction with other ADRs

- ADR-0008: language hygiene stays; parity is replaced by §4.
- ADR-0013: mechanism notices become rendered requirement kinds.
- ADR-0014: every package carries `lang/en.yaml`.
- AGENTS.md §1 is updated when this ADR is implemented.

## 10. Code references

- `monoagens/engine/i18n.py`, `monoagens/lang/en.yaml`, `monoagens/lang/id.yaml`.
- `tests/test_i18n.py`, `scripts/selfcheck.py`.
- `monoagens/gateways/whatsapp/gateway.py` (reaction hint),
  `monoagens/engine/commands/` (descriptions), `monoagens/engine/commands/language.py`.

*This ADR lives at `docs/v0.1.0/adr/0015-string-ownership-and-optional-language-packs.md`.*

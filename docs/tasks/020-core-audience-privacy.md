# 020 — Core: privacy follows people (D45)
Status: todo | Wave: 1 (right after 001) | Depends on: 001
Read first: [core.md](../core.md) §8 (absolute line 2), §7 (memories: who, place, private)
May touch: `monoagens/hooks.py`, `monoagens/memory.py`, `monoagens/pfc.py`, `monoagens/store.py` (audience lookup only), `tests/`

## Goal
Replace the per-place privacy filter written in 001 T3 with the audience rule: a private item may
enter a context only if everyone who will see the result was part of the item's audience, or the
context is a direct chat with the owner.

## Tasks
- [ ] T1 Audience of a place: the set of person ids present there (after the alias map); for a direct chat, that one person.
- [ ] T2 Context-hook filter: allow a private memory or line only if audience(context) ⊆ audience(source), or the context is the owner's direct chat; content from non-private places is unrestricted.
- [ ] T3 `recall` obeys the same rule.
- [ ] T4 Tests: Fulan's private chat on body A is usable in Fulan's private chat on body B once the accounts are linked; never in a group with anyone else; a private group's content is usable in a direct chat with one of its members; unlinked accounts are different people.

## Done when
All four tests pass, all earlier tests and replay pass, and no private content reaches a person outside its audience in any test or replay.

## Notes

# 020 — Core: privacy follows people, sharing, reaching a person (D45, D46)
Status: todo | Wave: 1 (right after 001) | Depends on: 001
Read first: [core.md](../core.md) §8 (absolute line 2), §4 (reaching a person, `self.share`), §7 (memories: who, place, private)
May touch: `monoagens/hooks.py`, `monoagens/memory.py`, `monoagens/pfc.py`, `monoagens/organs.py`, `monoagens/kernel.py`, `monoagens/store.py` (audience lookup only), `tests/`

## Goal
Replace the per-place privacy filter written in 001 T3 with the audience rule: a private item may
enter a context only if everyone who will see the result was part of the item's audience, or the
context is a direct chat with the owner; content reaches someone new only by consent; the soul can
act in any body from any body and address a person instead of a place.

## Tasks
- [ ] T1 Audience of a place: the set of person ids present there (after the alias map); for a direct chat, that one person.
- [ ] T2 Context-hook filter: allow a private memory or line only if audience(context) ⊆ audience(source), or the context is the owner's direct chat; content from non-private places is unrestricted.
- [ ] T3 `recall` obeys the same rule.
- [ ] T4 Tests: Fulan's private chat on body A is usable in Fulan's private chat on body B once the accounts are linked; never in a group with anyone else; a private group's content is usable in a direct chat with one of its members; unlinked accounts are different people.
- [ ] T5 `self.share(item | episode, to: person | place)` (a place shares with everyone present there): passes the action hook only when `origin_actor` is in the content's audience; writes `share.granted`; the filter then counts the new person as audience.
- [ ] T6 `act` offers the capabilities of every running organ; a communicative act may take `to: person` and optionally `via: body`, resolved to the person's direct place (the named body, else the body where last active); no direct place → honest `act.result` failure.
- [ ] T7 A communicative act aimed at another place is drafted with a desk built for that place's audience.
- [ ] T8 Replay scenario: the owner, in a Minecraft-like test organ, asks what Budi sent in a file on another body → answered (owner alone there) or moved to a private channel (others present); "send our work to Budi and ask if it fits" → `share.granted`, sent to Budi's direct place, a wait Expectation; Budi's reply resolves it and the owner hears back where the owner is now. Owner cannot share Fulan's private line to Budi. Second part: "finish my code, then send it to Budi via WhatsApp and to the group" → a long-running chain, a promise Expectation fired by its completion, delivery to Budi on WhatsApp, a share to the group's audience; with two candidate groups the soul asks which one before sending.

## Done when
All tests and the replay scenario pass, all earlier tests and replay pass, and no private content reaches a person outside its audience in any test or replay.

## Notes

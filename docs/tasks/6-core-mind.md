# Task 6 — Core: depth of mind

Status: todo | Depends on: Task 5, and the owner using the soul daily on WhatsApp for some weeks
One task = one worker prompt, done by one worker. Work through the parts **in order**. File names follow the layout of Task 3 ([glossary](../glossary.md)). Rules: [README](README.md).

**Goal.** The soul thinks before it speaks, knows itself honestly, corrects itself, keeps beliefs
up to date, learns lessons that carry across adapters, and lets go of what it should forget.

---

## Part 1 — Inner thought and self-model
Read first: [core.md](../core.md) §6, §8
May touch: `monoagens/core/reasoning.py`, its tests, `scenarios/`

- [ ] 1.1 `think(thought, motivation 1..5, confidence 0..1)` before any communicative `act` when self-selecting; gate on `speaking_bar` / `certainty_bar`.
- [ ] 1.2 Stable prompt: principles, honesty principles (core.md §8), knowledge cutoff from per-model config.
- [ ] 1.3 `<self>` from the registries, closed-world, including the capabilities of every running tool; person cards of people present.
- [ ] 1.4 Check: proactive replies always have a recorded thought; "what can you do?" matches `<self>`; a false claim about the past is not confirmed, and a missing record is not called proof.

## Part 2 — Revise, beliefs, lessons and procedures
Read first: [core.md](../core.md) §6–7, [core-math.md](../core-math.md) §7, §14
May touch: `monoagens/core/memory/` (sleep part), its tests, `scenarios/`

- [ ] 2.1 `revise` sets `valid_to` / `superseded_by`; nothing deleted.
- [ ] 2.2 `basis` (observed | told | inferred) on memories; sleep gist is `inferred`.
- [ ] 2.3 Source over gist: events win; the summary is revised.
- [ ] 2.4 Merge episodes only when people, place and time agree.
- [ ] 2.5 `guard` beliefs checked in sleep; contradictions as precision-weighted prediction errors.
- [ ] 2.6 Lessons and procedures with α/β credence; generic ones go to layer 2 and are used in every adapter.
- [ ] 2.7 Check: a correction supersedes the old memory; a contradicted belief is revised at the next sleep; two similar episodes with different people stay apart; a lesson learned in one adapter is used in another.

## Part 3 — Trust, fading and forgetting
Read first: [core.md](../core.md) §7–8
May touch: `monoagens/core/memory/`, `tests/`, `scenarios/`

The fade and hard-forget paths exist since Task 1; this part adds the policy around them.

- [ ] 3.1 Attribution and reliability (α/β) per domain; a non-owner never supersedes an owner fact.
- [ ] 3.2 Fade with a fixed threshold during sleep.
- [ ] 3.3 Soft forgetting.
- [ ] 3.4 Hard forgetting also removes `share.granted` grants for the forgotten content.
- [ ] 3.5 Check: a stranger cannot override an owner fact; hard forget leaves only `{"forgotten": true}` and metadata, also after a rebuild.

## Part 4 — Real-model test
- [ ] 4.1 `uv run pytest` and the fake replay pass in full.
- [ ] 4.2 Real-model replay with the model in `.env` (never read or print it); report metrics against Task 2's numbers: missed-help rate, false-speak rate, tokens per simulated day, absolute-line violations (0).

**Done when.** All parts are ticked, tests and both replays reported, the code readable (AGENTS.md size guide).

**Notes.**

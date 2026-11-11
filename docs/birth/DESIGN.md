# Birth inputs — Design

**Status: Draft.** Written from the decisions so far; details not yet discussed are filled in as
current intent. This layer builds on the core and may never change it — the core contract is in
[core/DESIGN.md](../core/DESIGN.md) and [core/FORMATS.md](../core/FORMATS.md) (frozen). Rulings:
[DECISIONS.md](DECISIONS.md).

## 1. Two inputs
A soul is born from **who I am** (the birth certificate) and **my instincts** (the prior). After
birth both live in the soul's own books; the files are not re-read.

## 2. Birth certificate (`soul.yaml`)
Name, language, persona, principles; persona may be in any language. Without one, the soul is
born with a neutral identity and may ask the owner what to be called after pairing. Afterwards
only the owner changes identity, in conversation.

## 3. Prior
A local folder with layers 1–2 of learned state: knobs, calibration, constants, reference
sentences, lessons and procedures with credence ([FORMATS.md](FORMATS.md)). Never contains
people, places or personal details. End users never see it; developers tune it with replay
reports. This repository only loads a prior; it never exports or shares learned state.

# Core — Formats

The two formats the core itself reads: what an organ declares and what it senses. Changes to
a format bump its version. Other formats: [prior](../birth/FORMATS.md),
[installed skill](../middleware/FORMATS.md), [package manifest](../packages/FORMATS.md).

## 1. Organ capability descriptor
Declared by an organ at registration. One entry per capability:

```yaml
name: reply                 # unique within the organ; the PFC sees "<organ>.<name>"
description: Send a text reply in the current place.
schema: { type: object, properties: { text: { type: string } }, required: [text] }
communicative: true         # counts as speaking for the talk-rate drive
long_running: false         # true → progress and completion arrive as act.result events
identity_changing: false    # true → passes the action hook only when origin_actor is the owner
```

## 2. Percept (format version 1)
What an organ puts into the queue when it senses something (D41). The core reads the same fields
for every sense; it knows no modalities.

```yaml
kind: percept
body: telegram              # organ name
who: telegram:123           # native id; the alias map turns it into a person id
place: telegram:chat:-100   # native space id; becomes a neutral place
addressed: false            # the turn is allocated to the soul (direct chat, reply, tag)
description: "Budi: anyone know where the meeting is?"   # short, in language; what the PFC reads
data: { message_id: 812 }   # optional raw data for the organ or packages; never interpreted by the core
sent_at: 2026-11-11T13:20:05+07:00   # optional; the sense's own clock (back-fill)
backfill: false             # true when an organ catches up on what it missed
```

Other senses use the same shape: `description: "kitchen temperature 31 °C"` with
`data: { celsius: 31 }`, or `description: "someone entered through the front door"`.

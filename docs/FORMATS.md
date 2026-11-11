# FORMATS.md — File Formats

Formats referenced by DESIGN.md. Changes to a format bump its version.

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

## 3. Prior (format version 1)
A soul is born from a prior: a plain folder with layers 1–2 of learned state. Never contains
layer 3 (episodic memory, people, places, personal details). End users never see it; developers
tune it with replay reports, not by guessing numbers (D36).

```
prior/
  manifest.yaml       # format_version, created, embedder (name + version), llm_family, language
  dispositions.yaml   # knobs, calibration, constants (below)
  references.yaml     # per language: reference sentences for the info_gap signal
  lessons.yaml        # list of {text, credence: {alpha, beta}}
  procedures.yaml     # list of {about, steps, lessons, credence: {alpha, beta}}
```

### 3.1 dispositions.yaml
Three sections, from "tune this" to "leave this alone":

```yaml
knobs:            # behavior, in plain words; the only values a developer normally changes
  talkativeness: 0.45
  restraint_after_speaking: 0.5
  spontaneity: 0.2
  helpfulness: 0.6
  boredom_with_repeats: 0.5
  patience_seconds: 90
  reserve_when_unfamiliar: 0.3
  speaking_bar: 4            # 1..5
  certainty_bar: 0.5
  memory_persistence: 0.5

calibration:      # measured per embedder/model by tools; never set by hand
  fok_threshold: 0.0
  fok_scale: 1.0
  wake_gate_lo: 0.35
  wake_gate_hi: 1.0          # 1.0 = "clearly yes" band off (D21)

constants:        # stable engineering values; change only with a decision
  own_name_weight: 0.5
  relevance_weight: 0.2
  talk_rate_window_minutes: 10
  promise_debt_weight: 0.1
  budget_weight: 0.3
  note_band: 0.2
  forward_window_minutes: 30
  context_weight: 1.0
  importance_weight: 0.5
  learning_rate: 0.02
  # the desk (D37): what one wake's prompt may hold
  recent_messages: 20          # roughly one phone screen; older lines via recall
  recent_tokens: 2000
  memory_candidates: 8
  memory_tokens: 1500
  thread_result_tokens: 3000   # latest result shown verbatim in <doing>
  volatile_tokens_max: 8000    # whole volatile part; trimmed oldest-first within each section
  recall_results: 10
```

Every knob is in [0, 1] unless stated, has a floor and a ceiling, and is what learning moves
(`disposition.updated talkativeness 0.45 → 0.44`). The mapping from knobs to the gate's internal
values lives in one function in the code; nothing else reads knobs directly.

### 3.2 The knobs

| Knob | What it means | Drives (internally) | If the soul is… |
|---|---|---|---|
| `talkativeness` | how easily it speaks without being addressed | θ base: 0 → 0.85, 1 → 0.35 | too chatty → lower; too quiet → raise |
| `restraint_after_speaking` | how much having just spoken holds it back | talk-rate weight `a` and `social_cost` weight | dominates a conversation → raise |
| `spontaneity` | how varied its decisions are in similar moments | τ: 0 → 0.02 (always the same), 1 → 0.2 | robotic and predictable → raise; erratic → lower |
| `helpfulness` | how strongly a question it can answer pulls it in | `info_gap` and `feeling_of_knowing` weights | misses chances to help → raise; answers what nobody asked → lower |
| `boredom_with_repeats` | how fast repeated stimuli fade | habituation γ: 0 → 0.99/min, 1 → 0.8/min | reacts to every sticker or repeat → raise |
| `patience_seconds` | how long it waits for someone else to answer first | the `wait` Expectation window | jumps in too fast → raise; answers too late → lower |
| `reserve_when_unfamiliar` | how reserved it is with unfamiliar places and people | r in θ, times (1 − familiarity) | too forward in new company → raise; too stiff with new people → lower |
| `speaking_bar` | how strong a thought must be before saying it (1..5) | `motivation_min` | says weak things → raise |
| `certainty_bar` | how sure it must be before stating something | `confidence_min` | states shaky things → raise; hedges too much → lower |
| `memory_persistence` | how long unused memories stay strong | ACT-R decay d: 0 → 0.7, 1 → 0.3 | forgets too fast → raise; drags up old trivia → lower |

The daily model budget and quiet hours are owner settings (owner policy), not knobs.

### 3.3 Loading rules
- Different embedder → `calibration` falls back to the default prior shipped with the repository.
- Different LLM family → load, but scale procedure and lesson credence down (halve alpha and beta)
  until proven again.
- Unknown major `format_version` → refuse with a clear message.
- Replay scenarios that prove the prior are kept in the repository's `scenarios/`, not inside
  the prior (they are tests, not learned state).

## 4. Installed skill (SKILL.md)
Installed skills come from packages; they are static and never learned (D34). The common
SKILL.md format: YAML front matter, then the instructions in Markdown.

```markdown
---
name: shop-customer-service
description: Answer customer questions for a small shop politely and accurately.
version: 1
---
Steps …
```

Only `name` and `description` enter the prompt; the body is loaded when the PFC asks for it.
What the soul learns by itself is stored as `procedure` and `lesson` memories, not as SKILL.md.

## 5. Package manifest (Draft)
One file at the root of a package declares everything it brings (D41). The loader reads it; the
core never does.

```yaml
name: work-modes
version: 0.1.0
state_namespace: work-modes          # its state keys are work-modes:* (projected from its events)
organs: [modes_organ]                # e.g. offers a `set_mode` capability
middleware:
  - { hook: context, scope: shared, entry: modes.annotate }   # attaches "mode: professional"
commands: [mode]
skills: []
rules:                               # standing rules shown in <self>, in this order
  - "In professional mode, keep replies short and formal."
knob_offsets:                        # bounded, temporary; recorded as disposition.updated
  talkativeness: { min: -0.2, max: 0.0 }
```

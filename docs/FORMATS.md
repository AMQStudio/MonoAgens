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

## 2. Prior (format version 1)
A soul is born from a prior: a plain folder with layers 1–2 of learned state. Never contains
layer 3 (episodic memory, people, places, personal details).

```
prior/
  manifest.yaml       # format_version, created, embedder (name + version), llm_family, language
  dispositions.yaml   # name: {value, min, max}
  lessons.yaml        # list of {text, credence: {alpha, beta}}
  procedures.yaml     # list of {about, steps, lessons, credence: {alpha, beta}}
```

Loading rules:
- Different embedder → vector-dependent values (wake-gate edges, feeling-of-knowing threshold)
  fall back to soul.yaml defaults.
- Different LLM family → load, but scale procedure and lesson credence down (halve alpha and beta)
  until proven again.
- Unknown major `format_version` → refuse with a clear message.
- Replay scenarios that prove the prior are kept in the repository's `scenarios/`, not inside
  the prior (they are tests, not learned state).

## 3. Installed skill (SKILL.md)
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

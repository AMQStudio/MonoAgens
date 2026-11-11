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
  skills/<name>/SKILL.md
```

Loading rules:
- Different embedder → vector-dependent values (wake-gate edges, feeling-of-knowing threshold)
  fall back to soul.yaml defaults.
- Different LLM family → load, but scale skill and lesson credence down (halve alpha and beta)
  until proven again.
- Unknown major `format_version` → refuse with a clear message.
- Replay scenarios that prove the prior are kept in the repository's `scenarios/`, not inside
  the prior (they are tests, not learned state).

## 3. Skill (SKILL.md)
The common SKILL.md format: YAML front matter, then the instructions in Markdown.

```markdown
---
name: daily-summary
description: Summarize yesterday's group conversation for the owner each morning.
version: 3
credence: { alpha: 7, beta: 2 }
taught_by: owner          # owner | person id | self
---
Steps …
Lessons: what failed and why …
```

Only `name` and `description` enter the prompt; the body is loaded when the PFC asks for it.
Learned skills live in a folder under git (versions = commits) until a version system is needed.

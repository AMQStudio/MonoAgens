# Birth inputs — Formats

Changes to a format bump its version.

## 1. Prior (format version 1)
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

### 1.1 dispositions.yaml
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

### 1.2 The knobs

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

### 1.3 Loading rules
- Different embedder → `calibration` falls back to the default prior shipped with the repository.
- Different LLM family → load, but scale procedure and lesson credence down (halve alpha and beta)
  until proven again.
- Unknown major `format_version` → refuse with a clear message.
- Replay scenarios that prove the prior are kept in the repository's `scenarios/`, not inside
  the prior (they are tests, not learned state).

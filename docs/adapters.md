# Adapters — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the core
([constitution §1](constitution.md)). Decision numbers (Dxx) trace to the history in git. Work:
[tasks/](tasks/README.md).

## 1. What an adapter is
An adapter connects the soul to one platform or device (Telegram, CLI, WhatsApp, a speaker, a
robot, a game). It senses (`run(put)`) and/or acts (`act(action)`), and it talks only to the
middleware: it gives it percepts and the tools it offers, and receives the actions routed to it
([constitution §4](constitution.md), D57). The core never knows which adapter something came
from; `body` in an event is only a name it carries. Like a sensory cortex, an adapter turns its
native signal into a short description in language plus raw data; like a motor cortex, it turns
an intention into native actions.

## 2. Rules for every adapter
- Declare capabilities with the core's descriptor ([core.md §9.1](core.md)): name, JSON schema,
  description, `communicative`, `long_running`, `identity_changing`, and the tool facts of the
  middleware's `contract.py` (`reads`, `changes`, `runs_code`, `network`); a tool without facts
  fails at boot. The middleware's tool registry offers them to reasoning in a fixed order (D2,
  D32).
- The brain decides intent and content; the adapter decides how (bubbles, TTS, gestures). Fine
  motor control stays in the adapter (D2).
- Sense through the core's `percept` ([core.md §9.2](core.md)): who, place, `addressed`, a short
  description, optional raw data, and `sent_at` when the sense has its own clock (D41).
- Report `addressed` (direct chat, reply, tag) as a neutral field (D40).
- Report native accounts and spaces as they are; the middleware's identity module maps them.
  Translate native command syntax into a neutral `command`, adding presentation only (menus,
  buttons); the middleware handles it (D9).
- Report long actions through `act.result`; never block the loop (D3).
- Mark speaking capabilities `communicative` (they count for talk rate and the forward model,
  D2) and capabilities that change who the soul is `identity_changing`.
- Ask for `cursor(organ)` on start, so input is never lost or duplicated.
- Optionally offer `history(place, before, limit)`; on connect, back-fill what was missed as
  `percept` with `backfill: true` and `sent_at`. The core appraises a backlog per place once, as
  one age-discounted percept; an addressed message still trips the circuit breaker (D37, D40).
- Own retries, rate limits, reconnects and platform quirks; errors become events, never crashes
  of the loop.
- Tokens and URLs come from `.env`; non-secret options (enabled or not, …) from `config.yaml`;
  nothing about an adapter lives in the core or in `soul.yaml`.
- Report `place_kind` (direct | shared) in every percept; only the adapter knows it (D48).
- Declare at boot what it can show besides messages (typing, edit message, buttons, reactions,
  …) in the vocabulary of the middleware's `contract.py`, and render what the middleware's
  `interaction.py` sends its own way; an approval answer (a click, a reaction) comes back as an
  ordinary percept ([middleware.md §9](middleware.md)). It decides nothing about what is shown
  or to whom.

**Loading an adapter** (D49). An adapter is a module with one factory, `create(options, env)`, that
returns the adapter: `options` is its own section of `config.yaml`, `env` gives the variables it
declares (it reads only those). `config.yaml` lists the enabled adapters by name under `bodies:` (the key keeps its name); an adapter outside
this repository also names its module (`module: some_package.minecraft`). The same install
module that loads plugins ([middleware.md §5](middleware.md)) loads adapters, so there is one
loader, not two. The entry point (`monoagens/__main__.py`) never imports an adapter: it asks the
loader for the enabled ones. Adding an adapter means installing its module and adding a few lines
to `config.yaml`, never editing code.

```yaml
bodies:
  telegram: { enabled: true }                       # built in: monoagens.adapters.telegram
  minecraft: { enabled: true, module: mono_minecraft, server: localhost:25565 }
```

## 3. Present and planned adapters
| Adapter | Status | Notes |
|---|---|---|
| Telegram | present | long polling; the Bot API cannot read history, so the soul's own log is its "scroll up" |
| CLI | present | Unix socket inside the main process, thin client |
| WhatsApp | next (Task 5) | ported from V1; history and back-fill through a linked device; files |
| Discord | next (Task 5) | ported from V1 |
| Voice | **Draft** | STT in, TTS out; description = transcript |
| IoT device / robot sim | **Draft** | first non-chat adapter; sensor readings as described percepts |

Tools (search, files, email) are in the [plugin catalog](plugins.md).

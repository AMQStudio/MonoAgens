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

**Native first** (D67). Every adapter, present and new (Telegram, CLI, WhatsApp, Discord, and
any later one), uses everything its platform offers: it reads every kind of message the platform
can deliver, and shows things the best way the platform allows. What one platform lacks is
declared with its reason, never faked; the middleware picks the fallback (§4). No adapter is a
special case: the same checklist and the same tests apply to all of them.

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
- **Report native accounts and spaces as they are** (`who: whatsapp:62812…`,
  `place: discord:channel:123`), never a person id; the middleware's identity module maps every
  account to one person id (D65, [middleware.md §7](middleware.md)). Accounts named inside a
  message (mentions, the author of a quoted message) go in `data.mentions` and
  `data.reply_to.who`, as accounts too. One platform's own quirks stay inside the adapter: when
  the platform gives one account two forms (WhatsApp's LID and phone-number JID), the adapter
  reports one stable form, so one account never looks like two people.
- Translate native command syntax into a neutral `command`, adding presentation only (menus,
  buttons); the middleware handles it (D9).
- Report long actions through `act.result`; never block the loop (D3).
- Mark speaking capabilities `communicative` (they count for talk rate and the forward model,
  D2) and capabilities that change who the soul is `identity_changing`.
- Ask for `cursor(organ)` on start, so input is never lost or duplicated.
- Optionally offer `history(place, before, limit)`; on connect, back-fill what was missed as
  `percept` with `backfill: true` and `sent_at`. The core appraises a backlog per place once, as
  one age-discounted percept; an addressed message still trips the circuit breaker (D37, D40).
  History from before the soul was connected is an import, not a back-fill (§5, D69).
- Own retries, rate limits, reconnects and platform quirks; errors become events, never crashes
  of the loop.
- Tokens and URLs come from `.env`; non-secret options (enabled or not, what is shown, …) from
  its section under `bodies:` in `config.yaml`; nothing about an adapter lives in the core or in
  `soul.yaml`.
- Report `place_kind` (direct | shared) in every percept; only the adapter knows it (D48). A
  private chat (DM) is `direct`; a group, channel, thread or organisation space is `shared`.
- Received files go to the attachment store (`put(file) → ref`, or a deferred ref when the
  bytes are fetched later, §5) and are listed in the percept's `data.attachments`, any number per
  message, with a `description` that sums them up; the send tool takes `attachments: [ref, …]`
  and the adapter declares files per message, largest size and types
  ([middleware.md §10](middleware.md)).
- Offer **place info** (§3): a place's name, form, parent, member count, whether the soul may
  post there, and the platform's role facts ("admin of this group"), used by `/whoami`, person
  cards, routes and Discord role mapping; a missing part is declared, never a silent empty
  answer.
- Declare at boot what it can show besides messages in the vocabulary of the middleware's
  `contract.py` (§4), and render what the middleware's `interaction.py` sends its own way; an
  approval answer (a click, a reaction) comes back as an ordinary percept
  ([middleware.md §9](middleware.md)). It decides nothing about what is shown or to whom.

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
  whatsapp:
    enabled: true
    show: { read_receipts: on, stickers: on }       # optional; middleware.md §9
    tools: [pin]                                    # optional platform-only tools; none by default
  minecraft: { enabled: true, module: mono_minecraft, server: localhost:25565 }
```

## 3. Places: direct, group, channel, organisation (D68)
Privacy needs only `place_kind` (direct | shared). Everything else about a place is place info,
read by the middleware, never by the core:

| Field | Meaning |
|---|---|
| `form` | `chat` (a private chat) · `group` · `broadcast` (a channel where few may post: WhatsApp Channels, a Telegram channel, a Discord announcement channel) · `thread` (a Discord thread, a Telegram forum topic) |
| `parent` | the place this one belongs to: a Discord server for its channels, a channel for its threads, a WhatsApp Community for its groups, a Telegram forum for its topics; none for a private chat |
| `name`, `members` | the place's name and member count, when the platform gives them |
| `can_post` | whether the soul may post there now (false in a broadcast it only follows) |
| `roles` | platform role facts of a person there ("admin of this group", Discord role ids) |

A post to a place where `can_post` is false fails with an honest `act.result`. A Discord role
mapped by the owner holds only in places whose `parent` chain reaches the server it came from
([middleware.md §8](middleware.md)). The adapter refreshes place info when the platform reports
a change (renamed, member joined or left, role changed).

## 4. Native: what goes in and what comes out (D67)
**In: everything the platform delivers becomes a percept.** Text, replies and quotes,
mentions, edits, deletions, reactions by others, stickers, locations, contact cards, polls and
votes, view-once media, any number of files, calls (incoming, missed, ended: "Budi called, not
answered"), and place events (joined, left, renamed, role changed). Each becomes a short `description` plus raw `data`:
- a reaction by someone else on the soul's own message is `addressed`;
- an edit is a new percept with `data.edits: <message id>` ("Budi edited: …");
- a deletion is a percept with `data.deletes: <message id>` ("Budi deleted a message"); the
  soul saw it, so the log keeps it, as a person remembers what was said; forgetting stays an
  explicit request ([core.md §7](core.md));
- a sticker is stored like any file, with a description of its emoji and pack ("Budi sent a
  sticker 😂 from 'Kucing'"); images and stickers are opened only when reasoning chooses to,
  through a reader (vision later).

**Out: few tools for the soul, rich intents through neutral fields.** The soul's tools stay
few and neutral, so the prompt cache holds and choices stay easy. What is a communicative
intent — something the soul means to do, not only how it looks — is a neutral field of `say`,
the same on every adapter, drawn the best way each platform allows:
- `say(text, attachments?, reply_to?, mentions?, sticker?, choices?, thread?, location?)`:
  - `sticker` — the ref of a sticker the soul has received; whether to send one is reasoning's
    choice;
  - `choices: [label, …]` (with `multiple: true` for a vote) — options people pick from; the
    answer comes back as an ordinary percept with `data.choice` (`addressed`, since it answers
    the soul);
  - `thread: "title"` — start a new thread or topic for this message where the place allows it;
  - `location: {lat, lon, name?}` — share a place;
  - `voice: true` — say it aloud: a voice note on chat adapters, speech on devices (§9; Task 5
    Part 2);
- `react(emoji, message_id?)`;
- `edit(message_id, text)` for its own messages, where the platform allows it.
Progress messages, typing, read receipts and approval prompts are not the soul's: the
middleware's `interaction.py` decides them and the adapter draws them.

**Platform-only actions** (pin a message, reject a call, manage a Discord role, set a group's
description, …)
are not neutral and are rarely needed. An adapter may offer them as its own tools, each with its
facts and permission like any tool; they are **off by default** and offered to reasoning only
when the owner lists them under `bodies.<adapter>.tools:`, so they never weigh on the prompt of a
soul that does not use them.

**What an adapter can show** is declared in `contract.SHOWS`: `typing`, `read_receipts`,
`edit`, `delete`, `reply`, `reactions`, `buttons`, `menus`, `polls`, `stickers`, `embeds`,
`threads`, `locations`. `interaction.py` uses the best one available, in a fixed fallback order,
so what is lost is visible in the code and the boot report:
- `choices`: buttons (menus when there are many) → a poll → a numbered list, answered by
  number;
- `thread`: a new thread or topic → a reply in the place, with the title as its first line;
- `location`: a native location → the name and coordinates as text;
- an approval: buttons → reactions (👍 / 👎 on the question) → "reply yes or no" as text;
- a progress message: edit one message → a new message per step;
- a sticker: the sticker → its emoji.

**Coverage is declared and tested** (D67). Every adapter declares at boot, in the vocabulary of
`contract.py`, what it reads (the kinds of §4 "In") and what it shows (`SHOWS`), and names each
gap with its reason ("read_receipts: the Telegram Bot API has none"). The boot report prints the
table per adapter. One contract test runs over every adapter: each kind the platform delivers is
either turned into a percept or declared as a gap, each declared `SHOWS` entry draws something,
and nothing is silently dropped. Adding a platform feature means adding it to the adapter and its
declaration, never a branch elsewhere.

## 5. History: back-fill and import (D69)
- **Back-fill** is what the adapter missed while it was offline: `backfill: true`, as in §2.
- **Import** is the history the account already had before the soul was connected to it
  (WhatsApp's history sync on pairing). The adapter takes all of it that the account offers, like
  someone given WhatsApp Web, and puts it as percepts with `imported: true` and `sent_at`
  ([core.md §9.2](core.md)). Imported percepts are recorded and searchable (`recall`, text
  search), never wake reasoning, never trip the circuit breaker, are never marked as read, and are
  summarized by sleep only for places the soul later becomes active in.
- **Media are fetched when opened.** An imported or back-filled file is put as a deferred ref
  (name, type, size and the adapter's locator, no bytes); the first `open` asks the adapter to
  fetch it. When a platform link has expired, the adapter asks the platform again (WhatsApp:
  a media re-upload request to the phone); a file that cannot be fetched is an honest error.

## 6. An account shared with a person (D66)
Some accounts can also be used by a person at the same time (WhatsApp as a linked device: the
soul may run on its own number or on the owner's own number). No option is needed; the adapter
tells the two apart itself:
- every message the soul sends carries its native message id in the `act.result`, so it is in the
  Log and survives restarts and history sync;
- a message from the account itself whose id is in the Log is the soul's own: it is not a
  percept;
- a message from the account itself whose id is not in the Log was typed by the account's
  holder on the phone: it is a percept from that account (`addressed: false` unless it is in the
  holder's chat with the soul, "Message yourself");
- the adapter reports its own account at boot (`self_account`); when identity maps that account
  to the owner (the owner paired or linked from it), the middleware adds one neutral line to the
  context: speaking in someone else's chat on this adapter speaks through the owner's account.
  Reasoning weighs it. No rule branches on it, except the read receipts of
  [middleware.md §9](middleware.md).

## 7. Present and planned adapters
| Adapter | Status | Notes |
|---|---|---|
| Telegram | present; native in Task 5 | long polling; everything the Bot API offers: inline buttons and the command menu, edit, delete, reply, reactions, polls, stickers, albums (media groups), locations, contacts, forum topics as `thread`, channels as `broadcast`, typing (chat action); declared gaps: no history (the soul's own log is its "scroll up") and no read receipts |
| CLI | present; native in Task 5 | Unix socket inside the main process, thin client; text, files by path, reply; typing and progress as status lines; buttons as numbered choices; declared gaps for the rest |
| WhatsApp | next (Task 5) | ported from V1 on neonize (a linked device); own or owner's number (§6); full history import (§5); files, stickers, polls, reactions, edits; no buttons, so approvals by reaction |
| Discord | next (Task 5) | ported from V1 on discord.py (message content intent); servers, channels, threads, DMs; buttons and menus (an interaction is acknowledged within Discord's 3 s by deferring, inside the adapter); role mapping (§3) |
| Voice notes | Task 5 Part 2 | STT in, TTS out; description = transcript |
| Voice calls | later | waits for a mature calling library to be widely released (WhatsApp: whatsmeow and neonize only see and reject calls; a calling stack such as meowcaller is still experimental and Go-only); then call audio ↔ STT/TTS as a `long_running` act ([project.md §5](project.md)). Until then a call is a percept and the soul answers in chat |
| Home Assistant | **Draft**, Task 5 Part 2 | the first non-chat adapter (§8) |
| Robot (own adapter) | later | microphone and speaker straight to the adapter, with its own reflexes (wake word, voice activity, barge-in, turning to the speaker); after Home Assistant |

Tools (search, files, email) are in the [plugin catalog](plugins.md). Model providers are not
adapters: they are drivers behind the core's model port, in `infra/providers/` ([core.md §3](core.md)).

## 8. Home Assistant as a body (**Draft**, D71)
Home Assistant (HA) already runs a whole voice pipeline — wake word (openWakeWord), speech to
text and text to speech over its Wyoming protocol (Whisper, Piper), and ESP32 voice satellites
through ESPHome — and it already controls the home's devices. The soul does not rebuild any of
that: HA is one adapter, and the soul is HA's conversation agent.
- **Hearing.** A small HA custom integration (shipped in this repository, installed into HA by
  the owner) registers the soul as a conversation agent and forwards each utterance to the
  adapter: a percept with `addressed: true` (the wake word was said), the satellite's area as the
  place (`homeassistant:area:kitchen`, `place_kind: shared`, since anyone in the room hears), and
  the HA user when HA knows who spoke, otherwise an unknown speaker. Absolute line 2 then holds
  as in any group: nothing private is said aloud in a room.
- **Speaking.** The answer to an utterance goes back to HA as the agent's reply, which HA speaks
  through its TTS on that satellite; a silent decision returns an empty reply. Speaking first (a
  reminder, an announcement) goes through HA's announce or TTS service on a satellite or media
  player of the target area. Typing has no meaning here; `voice` is the only way to speak.
- **The home as tools.** Only the entities the owner exposes in HA (HA's own "exposed entities")
  reach the soul: a short `home.state` (facts `reads`) and `home.control(entity, action, data?)`
  (facts `changes`, its permission at tier `trusted` by default), so locks, alarms and heaters go
  through the policy and approval modes like any change.
- **The home as senses.** State changes of the entities the owner lists under
  `bodies.homeassistant.senses:` (a door, motion, a temperature past a threshold) become
  percepts ("front door opened"), never addressed; everything else stays in HA.
- **Latency.** HA waits for the agent's reply, so the adapter sets a reply deadline from config
  (`reply_seconds`); past it, it answers HA with a short holding line and speaks the real answer
  later by announce. The gate and reasoning are never skipped: no speech-to-speech model answers
  for the soul.
- HA speaks with its own pipeline (Wyoming, Piper); the soul's TTS driver (§9) is used only if
  the owner wants another voice on the home's speakers.
- `.env`: `HA_URL`, `HA_TOKEN` (a long-lived access token). The client library for HA's
  WebSocket API, if one is needed, asks the owner first (AGENTS.md).

## 9. Speech: hearing and speaking (**Draft**, D72)
**One soul, one mind.** Every speech engine is only ears and a mouth. What to say, whether to
speak at all, memory and privacy stay with the soul: every utterance is a percept that passes the
gate, and every spoken word comes from a `say` the soul chose. No model answers for the soul.

**Drivers by protocol, not by model** (`infra/providers/speech/`, ported from V1's
`monoagens/providers/speech.py`). Local and cloud speech models are many and change every month,
so a driver speaks a protocol and covers every model behind it:

| Door | Covers | Notes |
|---|---|---|
| OpenAI-shaped HTTP (`/audio/transcriptions`, `/audio/speech`) | OpenAI, Groq, and the many local servers that mirror the shape (faster-whisper, Kokoro, …) | in V1; a different server is a different `base_url` |
| Wyoming (Home Assistant's voice protocol, JSON over TCP) | HA's local voice services: Whisper, Piper, openWakeWord, … | written with plain asyncio streams, no new dependency |
| Native | ElevenLabs, Deepgram | their own APIs; in V1 |

Chosen by `.env`, shared by every adapter, never tied to one:

```
STT_PROVIDER=openai            # openai | wyoming | elevenlabs | deepgram
STT_BASE_URL=                  # a local server or Wyoming address (tcp://host:10300)
STT_MODEL=
TTS_PROVIDER=wyoming           # openai | wyoming | elevenlabs
TTS_BASE_URL=tcp://localhost:10200
TTS_VOICE=
VOICE_REALTIME=                # optional realtime engine (§ below); empty = none
ELEVENLABS_API_KEY=            # one key variable per cloud provider
```

Each driver reports at boot the languages it supports; when the soul's language (`soul.yaml`
`language`) is not among them the boot report warns, so a model that only knows English never
fails silently on Indonesian. Small models for weak devices (an in-process runtime such as
sherpa-onnx) are a later dependency the owner approves (AGENTS.md).

**Hearing.** The adapter, as the sensory cortex, turns speech into a percept: a voice note is
stored as an attachment and transcribed, and the percept carries the transcript as its
description (`"Budi (voice note, 0:12): …"`) and in `data.transcript`; live speech on a device is
segmented by the adapter (wake word, voice activity) and each turn becomes a percept. Imported
voice notes are transcribed only when opened (§5). A failed or missing STT gives an honest
description ("Budi sent a voice note (0:12), not transcribed").

**Speaking.** `say(..., voice: true)` asks for speech; reasoning chooses it. A chat adapter sends
the platform's own voice note, a device speaks through its speaker; without TTS or a voice note
the text is sent. The words stay in the Log as the `act.out` text, so memory keeps what was said,
not only audio.

**Listening while speaking** (a reflex of the adapter, D2). When someone speaks over the soul,
the adapter stops talking at once (barge-in), and the `act.result` reports what was actually
heard (`spoken_until`), so the soul and its memory know where it was cut off; the interrupting
speech is a percept with `data.interrupted: true`. Echo cancellation, so the soul does not hear
itself, belongs to the device (a microphone array with a DSP, a voice satellite).

**Realtime engines** (OpenAI Realtime, Gemini Live, ElevenLabs, and local ones) may be used as a
faster ear (streaming transcription, end-of-turn detection) and a more expressive mouth (speaking
the soul's text), never as the one who answers: their own replies are switched off. The price is
a little more latency than letting them answer, paid for one identity, one memory and privacy
that holds.

**Latency.** Streaming STT and TTS let the soul start speaking before the whole answer is
synthesised. A websocket client, if a provider needs one, is a new dependency the owner approves.

**Cost.** Seconds transcribed and characters spoken are recorded with each use (in the percept's
`data` and the `act.result`) and appear in the metrics; a daily speech limit comes only if needed.

Out of scope: voice calls (project.md §5); cloning anyone's voice but the one the owner chose for
the soul.


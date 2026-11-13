# Adapters — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the contract
([contract.md](contract.md), [constitution §1](constitution.md)). Decision numbers (Dxx) trace to
the history in git. Work: [tasks/](tasks/README.md).

## 1. What an adapter is
An adapter connects the soul to one platform, device or app: a chat app, the CLI, a robot, a
speaker, a workspace app, a sensor, a game. It is where translation happens — the only place
(D86). Like a sensory cortex it turns native signals into the contract's facts (actors, places,
audiences, events); like a motor cortex it turns the soul's actions into native actions and
answers with an honest receipt. The core and the middleware never know which adapter something
came from; `body` is only a name they carry ([contract.md](contract.md)).

**Native first** (D67). Every adapter uses everything its platform offers: it reads every kind of
event the platform delivers and acts the best way the platform allows. Rich features are the
adapter's own capabilities and arguments, explained to reasoning in its `prompt_section`; what a
platform lacks is simply not offered. No adapter is a special case: the same contract, the same
conformance suite ([contract.md §7](contract.md)) and the same rules apply to all of them.

## 2. Rules for every adapter
- **Provide the surface** of [contract.md §4.1](contract.md): `name`, `run(put)`, `stop()`,
  `capabilities()`, `act(action) -> receipt`, `prompt_section()`, `self_actor()`; and any optional
  surface it supports (`history`, `fetch`, `render`, `ask`). A missing member fails boot.
- **Percepts** follow [contract.md §4.2](contract.md): the native actor (or *unknown*), the
  native place, `occurred_at`, a short `description` in language, `ref`, `about`, `involves`,
  `attachments`, features, and `answer` or `command` when the event is one. Write a fact only
  when it is certain.
- **Report one stable form per actor.** When a platform gives one account two forms (WhatsApp's
  LID and phone-number JID), the adapter reports one, so one person never looks like two.
- **Compute the features**: `addressed` (a direct place, a reply to the soul, a tag, a wake
  word), `named` (the soul's name said; the middleware tells every adapter the soul's current
  name), `addressed_other` (the turn given to someone else). The adapter is the only place that
  reads native text for this.
- **Report audiences** with `place.audience {place, actors, unknown}` whenever a place is first
  seen or its membership changes, folding every platform rule into that one fact
  ([contract.md §4.3](contract.md)). Unsure → `unknown: true`.
- **Capabilities**: its own names and schemas, generic facts, and a permission for each (a
  talking capability usually at base `guest`). Mark speaking `communicative` and anything that
  changes who the soul is `identity_changing`. Explain native features in `prompt_section` (at
  most 1,200 characters).
- **Act and answer**: split, retry, rate-limit and fall back by itself; deduplicate by
  `action.id`, so a resumed action is never sent twice; answer every action with exactly one
  receipt (`done`, `refused` with a reason, `failed`, `unknown`; `held` is the policy's, never a body's). A place where the soul may not
  post answers `refused` with the reason.
- **Translate commands** (a slash command, a menu, a button, a CLI line) into `command: {name,
  args}`; never interpret what a person meant by words.
- **Render lifecycle events and ask questions its own way** (§4), from its own options; never
  decide what is shown or to whom.
- **History**: optionally offer `history(place, before, limit)`; on connect, back-fill what was
  missed with `backfill: true`. History from before the body was connected is an import (§5).
- **Files**: received files go to the attachment store (`put` → ref, or a deferred ref) and are
  listed in `attachments`; capabilities that send files take refs.
- Own reconnects and platform quirks; errors become events, never crashes of the loop. Tokens and
  URLs come from `.env`; non-secret options from its section under `bodies:` in `config.yaml`.

**Loading an adapter** (D49). An adapter is a module with one factory, `create(options, env)`:
`options` is its own section of `config.yaml`, `env` gives the variables it declares (it reads
only those). `config.yaml` lists the enabled adapters under `bodies:`; an adapter outside this
repository also names its module. The entry point never imports an adapter; it asks the loader.
Adding an adapter means installing its module and adding a few lines to `config.yaml`.

```yaml
bodies:
  telegram: { enabled: true }                       # built in: monoagens.adapters.telegram
  whatsapp:
    enabled: true
    render: { composing: on, seen: on, progress: edit }   # this body's own rendering options (§4)
    tools: [pin]                                    # optional platform-only capabilities; none by default
  minecraft: { enabled: true, module: mono_minecraft, server: localhost:25565 }
```

## 3. Places and audiences
A place is wherever an event happens and an action lands: a private chat, a group, a channel, a
thread, a room, a document, a device, a game region. The contract knows only its id and its
audience; everything else about it (its name, that it is a thread of a channel of a server,
that only admins may post) is part of a percept's `description` or the adapter's
`prompt_section`, for reasoning to understand.

How adapters fold their platform into the audience:

| Platform | Audience |
|---|---|
| a private chat | the other person |
| a WhatsApp or Telegram group | the members now (newcomers do not see the past) |
| a Discord channel | its members; `unknown` when anyone who joins later can read the history |
| a channel that only broadcasts (WhatsApp Channels, a Telegram channel) | `unknown` |
| a room a robot or speaker is in | the people detected; `unknown` when it cannot be sure |
| a shared document | its collaborators |
| a sensor | nobody |

## 4. Native: what goes in and what comes out (D67)
**In: everything the platform delivers becomes a percept.** Text, replies and quotes (`about`),
mentions (`involves`), edits and deletions (`about` the earlier event), reactions by others,
stickers, locations, contact cards, polls and votes, view-once media, any number of files, calls
("Budi called, not answered"), and place changes (renamed, joined, left: also a new
`place.audience`). Each becomes a short `description` plus the contract's fields:
- a reaction by someone else on the soul's own message is `addressed`;
- a deletion stays in the log as a person remembers what was said; forgetting stays an explicit
  request ([core.md §7](core.md));
- a sticker or image is an attachment with a description; it is opened only when reasoning
  chooses to.

**Out: the body's own capabilities.** Each adapter offers what it can do with its own names and
schemas — for a chat app typically `say(text, attachments?, reply_to?, mentions?, sticker?,
buttons?, thread?, location?)`, `react(emoji, message)`, `edit(message, text)`; for a robot
`speak(text)`, `move(to)`, `gesture(name)`; for a document `edit(range, text)`, `comment(range,
text)`. Its `prompt_section` tells reasoning what the arguments mean on this platform. A
platform-only action (pin a message, manage a channel) is **off by default** and offered only
when the owner lists it under `bodies.<adapter>.tools:`.

**Lifecycle rendering** ([contract.md §4.5](contract.md)) is the adapter's own choice, set in its
`render:` options:

| Event | Chat | Robot | Workspace app | Game |
|---|---|---|---|---|
| `composing` | typing… | turns to the person, a light | a "writing" cursor | faces the player |
| `seen` | read mark (blue ticks) | a nod | — | — |
| `progress` | one message edited per step, or a new message | a spoken "still working" | a status line | — |
| `thought` | a quoted line | — | a side note | — |

An adapter that shares an account with a person (§6) marks events seen only when the soul speaks
there, so the person keeps their unread marks.

**Questions** (`ask`, [contract.md §4.6](contract.md)): buttons or a menu where the platform has
them, else a poll, else a reaction per choice, else a numbered message; a robot says the question
and listens; an app opens a dialog. The adapter records how each choice will come back and returns
`answer: {prompt, value}` when it does. A reply in words is a normal percept; reasoning
understands it.

## 5. History: back-fill and import (D69)
- **Back-fill** is what the adapter missed while offline: `backfill: true`.
- **Import** is the history an account already had before the soul was connected (WhatsApp's
  history sync on pairing): percepts with `imported: true` and their `occurred_at`. Imported
  events are recorded and searchable, never wake reasoning, are never marked seen, and are
  summarized by sleep only for places the soul later becomes active in.
- **Media are fetched when opened**: an imported or back-filled file is a deferred ref; the first
  `open` calls `fetch(locator)`. A file that cannot be fetched is an honest error.

## 6. An account shared with a person (D66)
Some accounts can be used by a person at the same time (WhatsApp as a linked device on the
owner's own number). The adapter tells the two apart itself: every action's receipt carries the
native id of what it sent, so it is in the Log; an event from the account whose id is in the Log
is the soul's own and not a percept; one whose id is not in the Log was made by the person and is
a percept from that actor. `self_actor()` reports the account; when identity maps it to the owner,
the middleware adds one context line ([middleware.md §7](middleware.md)).

## 7. Present and planned adapters
| Adapter | Status | Notes |
|---|---|---|
| CLI | present | Unix socket inside the main process, thin client; text, files by path, reply; `composing` and `progress` as status lines; questions as numbered choices |
| Telegram | present | long polling with `allowed_updates` for every kind it reads; inline buttons and the command menu, edit, delete, reply, reactions, polls, stickers, media groups (one percept per group), locations, forum topics; a failed download is an error percept, never a crash loop; no history and no read marks on the Bot API |
| WhatsApp | present | neonize (a linked device); own or owner's number (§6); full history import (§5); files, stickers, polls (votes decrypted), reactions, edits; no buttons, so questions by poll; handles logout and disconnect |
| Discord | present | discord.py (message content intent); servers, channels, threads, DMs; buttons and menus (acknowledged within Discord's 3 s inside the adapter); reactions and edits on uncached messages; `allowed_mentions` set |
| Home Assistant | **Draft**, [later task](tasks/later-voice-home.md) | the first non-chat adapter (§8) |
| Voice notes | [later task](tasks/later-voice-home.md) | STT in, TTS out; description = transcript |
| Voice calls | later | waits for a mature calling library ([project.md §5](project.md)) |
| Robot (own adapter) | later | microphone and speaker straight to the adapter, with its own reflexes (wake word, voice activity, barge-in, turning to the speaker) |

Tools (search, files, email) are in the [plugin catalog](plugins.md). Model providers are not
adapters: they are drivers behind the core's model port, in `infra/providers/`.

## 8. Home Assistant as a body (**Draft**, D71)
Home Assistant (HA) already runs a whole voice pipeline — wake word, speech to text and text to
speech over its Wyoming protocol, ESP32 voice satellites — and already controls the home's
devices. HA is one adapter, and the soul is HA's conversation agent.
- **Hearing.** A small HA custom integration (shipped in this repository) forwards each utterance:
  a percept with `addressed: 1` (the wake word was said), the satellite's area as the place, the
  HA user when HA knows who spoke, else *unknown*, and the area's audience as `unknown` unless HA
  knows who is in the room. Nothing private is said aloud in a room.
- **Speaking.** The answer goes back as the agent's reply, which HA speaks on that satellite; a
  silent decision returns an empty reply. Speaking first goes through HA's announce service.
- **The home as capabilities.** Only the entities the owner exposes in HA: `home.state` (facts
  `reads`) and `home.control(entity, action, data?)` (facts `changes`, base tier `trusted`).
- **The home as senses.** State changes of entities listed under `bodies.homeassistant.senses:`
  become percepts with actor *unknown*, never addressed.
- **Latency.** HA waits for the reply; past `reply_seconds` the adapter answers with a short
  holding line and speaks the real answer later. The gate and reasoning are never skipped.
- `.env`: `HA_URL`, `HA_TOKEN`.

## 9. Speech: hearing and speaking (**Draft**, D72)
**One soul, one mind.** Every speech engine is only ears and a mouth. Every utterance is a percept
that passes the gate, and every spoken word comes from an action the soul chose.

**Drivers by protocol, not by model** (`infra/providers/speech/`, ported from V1):

| Door | Covers |
|---|---|
| OpenAI-shaped HTTP (`/audio/transcriptions`, `/audio/speech`) | OpenAI, Groq, and local servers that mirror the shape (faster-whisper, Kokoro, …) |
| Wyoming (JSON over TCP) | Home Assistant's local voice services: Whisper, Piper, openWakeWord |
| Native | ElevenLabs, Deepgram |

Chosen by `.env` (`STT_PROVIDER`, `STT_BASE_URL`, `STT_MODEL`, `TTS_PROVIDER`, `TTS_BASE_URL`,
`TTS_VOICE`, `VOICE_REALTIME`, one key variable per cloud provider), shared by every adapter. Each
driver reports its languages at boot; the boot report warns when the soul's language is missing.

- **Hearing.** A voice note is stored and transcribed; the percept's description carries the
  transcript; live speech is segmented by the adapter (wake word, voice activity). A missing STT
  gives an honest description ("a voice note (0:12), not transcribed").
- **Speaking.** A body's speaking capability may take `voice: true` (a voice note on chat) or
  always speak (a device); the words stay in the Log as the action's text.
- **Listening while speaking** (a reflex of the adapter, D2): barge-in stops speech at once, the
  receipt reports what was actually said (`spoken_until`), and the interrupting speech is a
  percept. Echo cancellation belongs to the device.
- **Realtime engines** may be a faster ear and a more expressive mouth, never the one who answers.
- **Cost**: seconds transcribed and characters spoken are recorded with each use.

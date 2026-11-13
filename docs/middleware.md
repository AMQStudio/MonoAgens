# Middleware — Specification

**Status:** points without a mark are decided; points marked **Draft** state the current intent
and may change without a new decision. Nothing here may change the contract
([contract.md](contract.md), [constitution §1](constitution.md)). Decision numbers (Dxx) trace to
the history in git. Work: [tasks/](tasks/README.md).

## 1. The foundation in the middle
The three layers ([constitution §5](constitution.md), D57):
- **Core** — neutral and primitive, only a brain. It knows only opaque ids, numbers, times and
  descriptions; it restricts nothing itself.
- **Middleware** — the regulator, like drivers. It decides, it is the only path between the core
  and everything else, and it may limit or add what reaches the core.
- **Adapters and plugins** — talk only to the middleware, through the contract: they translate
  their world into its facts.

**Generic, like the core (D81, D86, D87).** The middleware is as generic and as stable as the
core below it. It works on the six facts of the contract — actor, place, audience, event,
capability, receipt — and on nothing else:
- **No body in the middleware.** No adapter, plugin, platform or device name; no native id
  format; no emoji, button, reaction, typing or read mark. It never translates: every native
  form is drawn and parsed by its adapter.
- **No language understanding in the code.** No word lists, stopwords, keyword or substring
  matching, regexes over text. What people mean is understood by reasoning and stated as an
  argument; the middleware checks it against facts. Its own words for people (an approval
  question, a brake notice) come from the message catalog (`monoagens/middleware/lang/<code>.yaml`,
  English as the fallback) in the soul's language.
- **Decisions from facts.** Everything it decides — privacy, authority, routes, approvals — is
  decided from actors, places, audiences, refs, capability facts and permissions.
- **One rule, one home.** Privacy is checked in the context hook and when an attachment is
  opened or forwarded; authority in the policy. Nothing checks a rule twice.
- **Only hooks and levers.** It acts through the three hooks and the six levers (§2); it never
  replaces or wraps a core function or attribute.
- **State only through events** (lever 4): anything it must remember across a restart (a pending
  question, a mark of what was seen) is an event or its projection, never a variable in the
  running process.
- Architecture tests hold these ([contract.md §7](contract.md)).

The middleware runs in the core's three hooks (inbound, context, action) and offers one API
(§2). It is static and never learns — learning belongs to the soul (D34).

```
   Plugins (Soul Tuning, Web Search, Workspace…)  Adapters (CLI, WhatsApp, a robot, an app…)
                   \                                  /
                    ──────── contract, waist 2 ───────
                                 Middleware
  policy · privacy · identity · registry · loader · interaction · roles
                    ──────── contract, waist 1 ───────
                       Core (ids, numbers — mechanisms only)
```

Seven files, one concept each; the contract types themselves live in `monoagens/contract/`
(contract.md §7). A file that grows becomes a folder of the same name; the concepts stay.

| Module | File | What it provides |
|---|---|---|
| [Policy](#3-policy) | `policy.py` | the owner policy, first and last in every hook: the identity line, budget, quiet hours, brakes; whether a call passes, asks or is refused (§8) |
| [Privacy](#3-policy) | `privacy.py` | absolute line 2: audience labels, the one context check, `share`, attachments' audience |
| [Identity](#7-identity) | `identity.py` | actor map (one person id per person), pairing, linking as a merge, person cards, routes (`to:`) |
| [Registry](#4-registry) | `registry.py` | capabilities, commands, skills: one list, routing by place, what a requester may use, `/help` |
| [Loader](#5-install) | `loader.py` | reads `config.yaml`, `config.d/` and manifests; loads adapters and plugins; the boot checks |
| [Interaction](#9-interaction) | `interaction.py` | lifecycle events (composing, seen, progress, thought) and questions asked and answered |
| [Roles](#8-roles-and-permissions) | `roles.py` | what each person may do: roles with a base tier, permissions, approval modes |

**What the middleware itself offers.** Only what regulates the middleware: `share` and `answer`
(capabilities for reasoning), pairing, `/link` and `/unlink`, budget and quiet-hour settings,
`/help`, `/plugin`, `/reload`, `/whoami`, `/whois`, `/show`, `/role` and `/roles show`. They are
gated by permissions like any tool (§8). Anything that is a feature (search, files, `/mode`) is a
plugin. Test: if removing it would leave a rule that can no longer be kept or changed, it belongs
to the middleware; if the soul would only be less able, it is a plugin.

## 2. The API: six levers (D41)
Every plugin and adapter uses the same levers, so the core never knows the case — modes, a
persona layer, simulated emotions, RBAC and moderation all fit:
1. **Pass, replace or stop** an item at a door (percepts, context items, actions). Every Stop is
   recorded.
2. **Annotate** an item; the note travels with it to reasoning, inside its token budget, before the
   policy's last pass.
3. **Declare** standing rules, shown in `<self>` in a fixed order (cache-friendly).
4. **Remember and observe**: write only by putting events (state is a projection under
   `<plugin>:*`, so replay and hard forgetting still work); read the log and state read-only
   (D43). The view returns raw content: a tool that shows it to people passes it through
   `privacy.visible` for the requester's place first.
5. **Sense and act** through its own tools.
6. **Steer knobs** by a bounded, temporary offset recorded as `disposition.updated` with the
   plugin as source; learning never moves an offset, and uninstalling removes it. The middleware
   computes the effective knobs: the prior's value (later the learned value) plus every active
   plugin offset, clamped to the manifest's bounds and [0, 1], recomputed at boot, reload and on
   every new offset.

Limits: nothing crosses the two absolute lines, adds a thought act, signal or book, adds to the
contract, or writes the core books except through events.

**Order in each hook** (D9): `[policy — first] → [middleware modules] → [plugin middleware, in
install order] → [policy — last]`. Name collisions fail at boot.

**What a failure does, by layer (D82).** Every failure is recorded and logged; none is silent,
none is swallowed by a blanket `except`. What happens next depends on what failed, decided in one
place:

| Layer | On failure | Examples |
|---|---|---|
| Guard rule (policy, privacy, identity, roles) | the item stops (fail closed), `guard.stopped` or `error` | the context check, an attachment's audience, a permission |
| Best-effort rule (lifecycle events) | `error` naming the rule; the item **continues** | a body failing to render `composing` or `seen` |
| Adapter | `error` naming it; that adapter alone restarts with backoff | a timeout, a lost connection |
| Tool or plugin call | a receipt with `failed` and the error type, never its text (it may hold a secret) | a search that failed |
| Core loop | `error`, then the next event; never `continue` without a record | a failed insert |

Every rule declares its kind when installed: **guard** (the default) or **best-effort**; the
composer runs best-effort rules in their own wrapper and timeout, so the hook contract (fail
closed) is unchanged. A fault-injection test makes each adapter member, each best-effort rule and
each tool fail in turn and checks that the soul keeps working and records the error.

**A refused act is not the end of the turn (D82).** A refusal returns to reasoning as that act's
receipt (`refused`, with its reason), and reasoning may do something else within the thought-round
limit, the budget and the brakes. Quiet hours and approvals hold the act instead and do not wake
reasoning again.

## 3. Policy
The one middleware that is always present.
- Wraps every module and plugin, first and last, and cannot be removed (D9).
- Enforces the two absolute lines deterministically: the `identity_changing` check on
  `origin_actor` in the action hook, and the privacy rule below in the context hook (D24,
  D32.3–4). The core refuses to run without this policy (fail-closed, D57).

**Private content (absolute line 2, D45, D50, D85, D86).** Privacy follows people, not places,
and needs one fact: the audience ([contract.md §4.3](contract.md)).
- **Labels.** In the inbound hook every event is labelled with its place's audience at that
  moment, after the actor map: the people who could perceive it then. A newcomer never receives
  what was said before they could perceive it. A memory's audience comes from the events it was
  derived from (D53); an attachment's from the events that carried it.
- **The one check.** A context item may enter the context of a place only if every member of
  that place's audience now is in the item's audience. An audience that includes *unknown*
  lets no private item in. The owner's direct place — a place whose audience is only the owner —
  sees everything. Content from places whose audience was *unknown* when it was said is not
  private to anyone and flows freely.
- **Where it runs.** Once, in the context hook, over every context item (its sources give its
  audience), and when an attachment is opened or forwarded. An item that fails is dropped; when
  relevant items were withheld, one line without content says how many and from whose private
  place (D56). Every lifecycle event that shows content passes the same `visible` call.
- **Before the model, by construction (D84).** The model never sees private content it may not
  use in the place it speaks to, so it has nothing to leak. There is no check after the model and
  no text similarity of any kind.
- **Speaking elsewhere** (D48). An act aimed at another place becomes an intent that wakes
  reasoning in the target place ([core.md §4](core.md)). In the action hook the policy decides
  what the intent carries: the drafted words only if every private item the source context held
  is visible at the target; otherwise only what to do and to whom. The intent never carries the
  goal's or the trigger's text.
- **Sharing by consent (D46).** `share(items | episode, to)`: reasoning names the event ids (or
  an episode id) it means; the policy passes only when the chain's `origin_actor` is in the
  audience of every named item, records `share.granted` for exactly those items, and the new
  person joins their audience. A share that names nothing is refused with a question. Each
  person speaks only for what they were part of; the owner's view of everything is a view, not
  ownership.
- **Asking on someone's behalf (D80).** When the soul sends a message on behalf of a person X,
  the one event that answers it — the event whose `about` is that message, or the one that meets
  that chain's `wait` expectation — is labelled with X in its audience too. Never the person's
  later events. X may then share it.
- **Data leaving the machine** (D43): by default nothing does; only a plugin the owner installed
  for it, sending numbers, not content.
- **Privacy rules narrow, never widen.** Other middleware and plugins may add stricter rules; they
  can never let private content reach someone the owner policy would refuse. They may ask for
  more than consent before sharing (confirm before forwarding any file).
- **Routes** (D46, D57): the action hook resolves a target that is not a place (`to:` a person,
  optionally `via:` a body) to a place: a place of that body whose audience is that person and
  the soul, else the place where the person was last active. Several fitting places → the soul
  asks which; none → a `refused` receipt ("no way to reach Budi yet"). A route rule may change the
  default.

**Default owner policy**, active out of the box: the two absolute lines, the daily model budget,
quiet hours, brakes. Budget and quiet hours are owner settings in `config.yaml`, changed by hand or
with `/budget` and `/quiet`, which write their own line (§5).

**Brakes on runaway work** (D63), deterministic, no model call: at most `max_task_steps` steps per
chain and `max_wakes_per_hour` wakes per chain (`config.yaml` → `owner_settings`). When a brake
holds, the chain stops, it is recorded, and the owner is asked in the owner's direct place whether
to continue (§9); nothing stops silently.

## 4. Registry
Capabilities, commands and installed skills follow one pattern, as MCP's tools, prompts and
resources do: a list of names with short descriptions, the full entry on request, one namespace,
collisions failing at boot.

- **Capabilities belong to their body or plugin** (D86). Each adapter and plugin registers its own
  capabilities, with descriptor, facts and permission ([contract.md §3](contract.md)). The
  registry lists every capability in a fixed order and routes each action: a body's capability to
  that body, for a place it owns; an action for a place another body owns is refused with the
  reason. There is no neutral `say` and no hidden capability.
- **Every capability names a permission.** Adapters declare theirs like plugins (a talking
  capability usually at base `guest`); a capability without facts or without a permission fails
  at boot, naming it. A permission without a `base` is the owner's only.
- **Commands are structured.** An adapter turns its native form into `command: {name, args}`
  ([contract.md §4.7](contract.md)); the registry handles it by name in the inbound hook, without
  a model call. `/help` is the same everywhere. A handler that fails answers with an error. A
  command's answer goes through the action hook like any act.
- **Offered per requester.** In the context hook the registry offers a turn only the capabilities
  the requester may use (`roles.py`); in the action hook `policy.py` decides pass, ask or refuse
  from the requester's permissions, the capability's facts and the requester's approval mode (§8).
  The requester is the chain's `origin_actor`.
- **One output marker** (`contract.OUTPUT_MARKER`) for every cut output; **nothing is lost when
  an output is cut** (D62): the full output goes to the attachment store with its `act.result` as
  origin; the result carries the cut text plus `full: {ref, size}`. `registry.more(ref, offset,
  limit)` reads the rest, capped, through the attachment privacy check; without a place it is
  refused.
- **Draft**: per-person rate limits live here.

## 5. Install
- The loader reads each plugin's manifest ([plugins.md §3](plugins.md)) and each enabled
  adapter's factory, assembles capabilities, middleware, commands, skills, rules, knob-offset
  bounds and state namespaces, and refuses to start without the owner policy first and last in
  every hook (D9, D41, D57).
- **Boot checks**: an adapter missing a member of [contract.md §4.1](contract.md), a capability
  without facts or permission, a schema outside the closed subset, a permission declared outside
  its owner's namespace or redefining a middleware permission, a missing dependency, a name
  collision — each fails boot, naming it. The boot report lists every adapter's optional surfaces.
- Plugins are trusted code installed by the owner and run in-process (the Home Assistant model);
  the owner policy holds against plugins that follow the contract, not against malicious code
  (D32.11).
- The installed plugins are listed in `config.yaml`; each plugin's own settings live in
  `config.d/<plugin>.yaml`, holding only what the owner changed; defaults live in the manifest
  ([plugins.md §3–4](plugins.md), [project.md §2](project.md)).
- **Format versions** (D58): every YAML file carries its format version; older ones are upcast in
  memory and named in the boot report; newer than supported fails at boot.
- **The files are the source of truth** (D59). The program writes them only on the owner's
  command, after a backup, atomically, and each edit is parsed back and checked or refused:
  `/plugin enable|disable|update <name>`, `/role add|remove <person> <role>`, `/budget`, `/quiet`,
  `/show`. Every other line, comments included, stays as it is. Every change, by command or by
  hand, is recorded as a history event.
- **`/reload`** validates the whole new configuration first and applies it only if everything is
  valid; running adapters are not created again; otherwise it refuses with a clear message and
  the running configuration stays. It names what needs a restart.
- **Plugin updates, the dpkg way**: a default the owner never touched is offered for
  `/plugin update`; one the owner changed is only reported.
- Adapters are loaded from `bodies:` in `config.yaml` (the key keeps its name) through each
  adapter's `create(options, env)` ([adapters.md §2](adapters.md), D49).
- Uninstalling removes a plugin's capabilities, middleware, rules and knob offsets; its events stay
  as history.

## 6. Skills
Part of the registry (§4).
- **Installed skills** are static SKILL.md files shipped by plugins. Only the names and
  descriptions of relevant skills enter the prompt; the body is loaded on request; versioned; use
  counts feed the same ACT-R activation as memories (D11, D34).
- Skills do not learn; what the soul learns is a core `procedure` or `lesson` memory (D34).

```markdown
---
name: shop-customer-service
description: Answer customer questions for a small shop politely and accurately.
version: 1
---
Steps …
```

## 7. Identity
**One person, one id** (D65). The core never sees a native actor, only a person id (D10, D57).
- **The actor map** (`state: alias:<actor>` → person id): the inbound hook maps every native
  actor an adapter reports — the event's `actor` and its `involves` — to a person id. *unknown*
  stays *unknown*.
- **Registration is automatic.** An actor seen for the first time gets a new opaque person id
  (`person:p_7f3a`), recorded as `identity.registered {actor, person}`. One actor has one id (a
  unique constraint). The owner's id is always `person:owner`.
- **Linking is a merge.** Linking two actors of one person merges their person ids into the
  **older** one (`person:owner` always wins), recorded as `identity.merged {from, into}`; the
  middleware puts the core's `source.merged {from, into}` ([core.md §8](core.md)). The log is
  never rewritten; reads follow the merge.
- **Who may link.** The owner (`/link <actor|person|name> <actor|person|name>`), or the person:
  `/link` from actor A gives a six-digit code (single use, 10 minutes); `/link <code>` from actor B
  merges B into A's person. Wrong guesses are counted per guessing actor (three in the code's
  lifetime) and never touch other people's codes; the owner cannot redeem a code someone else
  issued. The soul may suspect and ask the owner; it has no tool that links. A wrong link is a
  privacy leak (D45).
- **Unlinking** (`/unlink <actor>`) gives that actor a new person id from now on
  (`identity.unlinked`). What was merged stays merged.
- **A body's own actor** (D66): when an adapter's `self_actor()` maps to the owner (the soul runs
  on the owner's own account), identity adds one context line for that body: acting there acts as
  the owner. Reasoning weighs it; no rule branches on it.
- **Becoming the owner** (D33): on first start the soul shows a short pairing code; the first
  person who sends it from any body becomes the owner (three wrong tries per actor). Changing the
  owner later needs the current owner.
- **Person cards** (D55): through the context hook, for the people present or involved in the
  trigger, their name and where they can be reached, next to what the soul has learned of them;
  `roles.py` adds each role's and the person's `context`, read at call time.
- **`/whoami`**: the asker's person id, label, linked actors, roles with tier and approval mode,
  and this place's description. In a place whose audience is more than the asker, linked actors
  go to the asker's direct place, never to the group. **`/whois <name or actor>`**: another
  person's id, label and linked actors; permission `identity.whois`.

## 8. Roles and permissions
`roles.py` answers **what this person may do**; `policy.py` turns that into pass, ask or refuse.
Always present; empty by default: the owner may do everything, everyone else only talks (D59).
Modelled on Discord roles (several per person, permissions add up, `@everyone` as the floor) and
on the base roles of GitHub and Google Cloud.

- **Four fixed tiers**: `guest < member < trusted < admin`; the owner is above all and always
  free. Plugins and adapters know only the tiers, never the owner's role names.
- **Permissions are declared** by plugins and adapters in their manifest or registration, each
  with a label, a description and the lowest `base` tier that gets it; a permission without a
  `base` is the owner's only. `role_settings` may give a value per tier.
- **Roles are defined by the owner** in `config.yaml` → `roles:`: a **tier role** has a `base`;
  a **group role** has none and gives nothing but what it lists; either may add `allow`, `deny`,
  `settings`, an `approval` mode, a `label` and a `context`. An `allow` gives exactly the listed
  permissions, never a tier.
- **People** (`config.yaml` → `people:`), keyed by person id: `p_7f3a: { label: "Budi", roles:
  [member, toko-a], context: "The owner's brother." }`. `roles` is the only record of who holds
  which role; a person not listed holds the `fallback` role. Roles are given to person ids only;
  nothing a platform reports grants anything. A plugin may give roles through the same role
  operation the owner uses, under its own permission.
- **Which value wins**: all of the person's roles together (a `deny` beats an `allow`), then the
  tiers in `config.d/`, then the manifest's. A place may override with the same `allow` and `deny`.
- **Approval modes**, per role: `yolo` passes everything except calls marked `sensitive`;
  `default` asks before `runs_code` or `changes` outside the person's own workspace; `strict` asks
  before every step. The owner answers (§9), and so may anyone with `approval.answer`. An approval
  is consumed by the act it approved; the approved act runs as the requester, never as the owner.
- **The middleware's own permissions**: `config.reload`, `plugin.manage`, `roles.manage`,
  `identity.whois`, `approval.answer` at tier `admin`; `identity.whoami` at `guest`.
- **No one grows their own power**: `/role add` gives only roles below the giver's highest tier.
  Owner only, never movable: anything `identity_changing`, a new owner, editing admin-tier roles.
- No permission overrides the two absolute lines or buys dishonesty (D24, D31).

## 9. Interaction
What the soul shows besides its actions, and questions asked and answered.
- **The core records and changes nothing.** Everything needed is already an event.
- **`interaction.py` decides what and to whom** and emits lifecycle events to the body of a place
  ([contract.md §4.5](contract.md)): `composing(place)` from the moment a reasoning result is
  going to speak there until its last receipt (silence shows nothing); `seen(event)` once an event
  is recorded and being processed (never for imported or back-filled events, never twice after a
  restart); `progress(place, step)` for long work; `thought(place, text)` for thoughts. Content
  (thoughts, steps) passes `privacy.visible` for that place first.
- **The body renders**, its own way or not at all, from its own options in `config.yaml`
  ([adapters.md §4](adapters.md)). The middleware never knows how.

**Settings** (`config.yaml` → `interaction:`; `/show <name> <value>` writes the line):

```yaml
interaction:
  thinking: owner_direct     # off | owner_direct (a place whose audience is only the owner) | all
  steps: long                # off | long (work that takes long) | all
```

**Questions** (approvals, a brake's "continue?"). `interaction.py` calls the place's body `ask`
with the question and its choices from the message catalog, and records `question.asked {ref,
request, choices}`. An answer counts when:
- a percept carries `answer: {prompt: ref, value}` for that ref, from an actor allowed to answer;
  or
- reasoning, having understood an answer in words, calls the `answer(request, value)` capability,
  and the trigger is from an allowed actor and is either `about` the question or in that actor's
  direct place.

Unanswered questions expire (`question.expired`) after the owner setting `question_ttl`. A
question and its answer are events, so an answer after a restart still counts, and an answered
approval whose act has not run yet runs after the restart. A body without `ask` gets the question
as an ordinary message through its communicative capability.

## 10. Attachments
Files that arrive with an event or leave with an action, any number. The core never reads them
(D61).
- **Vocabulary** (contract): an attachment is `{ref, name, mime, size}`. A percept lists them in
  `attachments`; its `description` sums them up for reasoning.
- **Store** (`infra/`): an adapter calls `put(file) → ref`; anyone who opens one calls
  `open(ref)`, which `privacy.py` checks. An attachment carries the audience labels of the events
  it came with; a file from a private place goes to someone else only after `share`. One file sent
  in several places is one stored file, deleted when no non-forgotten event carries it.
- **Deferred refs** (D69): an adapter may put a file without its bytes (`put_deferred(name, mime,
  size, locator) → ref`); the first `open` that passes the privacy check calls the body's
  `fetch(locator)`. A fetch that fails is an honest error.
- **Sending**: a body's capability takes attachment refs in its own arguments; the policy checks
  each ref's audience against the target place (the same one check), and "confirm before
  forwarding a file" applies per attachment. How many files fit in one native message is the
  body's business: it splits by itself and answers with one receipt.
- **Reading**: Workspace's `read` takes a `ref` ([plugins.md §5.3](plugins.md)). Images are opened
  only when reasoning chooses to; vision is a separate, later decision.

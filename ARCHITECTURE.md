# Architecture

This document describes the architecture of MonoAgens: the parts of the program, what each one
is responsible for, how they connect, and the lines the design holds on to. Read it once, early,
before changing anything.

It is not a specification. Behavior, formats and messages belong to the specification, and
formulas and their sources belong to the research notes. Statements marked **Architecture
invariant** are the lines the design holds on to. Everything else describes the current intent
and may be redesigned, as long as the invariants still hold.

## Bird's eye view

MonoAgens is one persistent soul that lives across apps and bodies (a chat app, a terminal, a
home, later a robot or a game world), and a research harness for one question: **how far can a
harness carry a mind?**

The bet: today's language models reason well, but they are a prefrontal cortex without the rest
of the brain. MonoAgens is the rest of the brain: perception, attention, memory, expectation,
restraint and learning, built from studied brain functions rather than from scenarios. Most
agent frameworks start from "a message arrived, run the model". MonoAgens starts from
"something happened, record it, then decide whether it is worth thinking about". Staying silent
is a decision, and it is recorded like any other.

It is not a task runner that starts, works and exits. It is one long-lived process that
remembers, keeps promises and decides when to speak.

The program has four parts, stacked like an hourglass:

```
        Plugins   bodies and their worlds, tools, hooks, commands, skills
     ─────────── protocol, waist 2 ───────────
        Gateway   runs plugins, routes, maps people; the valve sits in its doors
     ─────────── protocol, waist 1 ───────────
        Core      the mind: ids, numbers, events
```

A fifth part, the **Lab**, sits beside the stack and measures it.

The rule between the layers: **plugins translate, the gateway routes, the valve decides from
facts, the core records and carries out.**

## The parts

### Protocol

The protocol is the skin: the one contract between the layers. Every body, present or future,
is described with the same six facts:
- **actor**: who caused an event;
- **place**: where it happened and where an action lands;
- **audience**: who could perceive it;
- **event**: what happened;
- **capability**: what a body can do;
- **receipt**: the honest outcome of an action.

**Architecture invariant:** the protocol is small, generic and stable. It changes only
additively, and only by the owner. A fact that does not make sense for every kind of body does
not belong in it.

**Architecture invariant:** translation happens only at the edge. Everything specific to a
platform, a device or a feature is translated by its plugin into the six facts. Nothing else
crosses.

### Core

The core is the mind and the subject of the research. It is built as a closed list of brain
functions (the brain map). Each function is one mechanism with a measurable claim: what gets
worse without it.

The core:
- records every event;
- keeps memory and consolidates it during sleep;
- holds expectations of what should happen next;
- pays attention through signals, salience and drives;
- decides cheaply, without a model call, whether a moment deserves reasoning.

Reasoning is the language model. It is woken only when the gate says so, and acts only through a
few thought acts: think, act, expect, resolve, revise.

The core is fully autonomous. It runs with no valve at all, and then it is governed only by its
own principles and the model's judgment. Its capabilities are open (it can recall the whole log
and act in any place through any body), and every one of them leaves through the gateway's
doors, where the valve sits.

**Architecture invariant:** the core knows only opaque ids, numbers and descriptions. It never
reads an event's raw content or its labels, and has no notion of person, owner, group, role,
platform, plugin or model provider. Speaking is a capability like any other, never hard-coded.

**Architecture invariant:** every mechanism has a neutral value at which it has no effect and
the loop still runs. Switching a mechanism off means moving it to that value, never choosing
another code path. The loop stays one loop whatever is switched off.

**Architecture invariant:** the inside of the core changes only with evidence from the Lab. A
mechanism whose removal changes nothing measurable is removed.

**Architecture invariant:** no emotions, moods or simulated feelings, and no hard-coded word
lists for understanding language. Understanding belongs to reasoning.

### Gateway

The gateway is the home between the core and everything outside. It:
- runs plugins, either built in or as separate programs;
- registers their capabilities and routes each action to the body that owns its place;
- maps the accounts a body reports to one id per person;
- asks the owner questions and stores attachments;
- loads the configuration and talks to model providers.

It acts on the core only through three doors (inbound, context, action) and offers plugins a
small set of levers. It never wraps or replaces a core function.

**Architecture invariant:** the gateway is as generic as the core. It knows no platform and
translates nothing.

**Architecture invariant:** the gateway never learns. Anything it must remember across a restart
is kept as events, never as process memory.

### Valve

The valve is the policy inside the gateway's doors: the two absolute lines, privacy, roles and
permissions, the budget, quiet hours, brakes on runaway work, and the switch of every core
mechanism. It is on by default. The owner may switch it off, wholly or in part, for experiments.
Whether it is on is recorded, so every result says which mode it came from.

**Architecture invariant:** the valve runs first and last in every door and always runs inside
the process. Other rules may only narrow what it allows, never widen it.

**Architecture invariant:** the valve decides from facts, deterministically, never through the
model.

### Plugins

Everything from outside the core and the gateway is a plugin. A plugin is one package that may
contribute:
- an adapter: a body and its world, such as a chat app, a robot, a game or a sensor;
- tools;
- hooks;
- commands;
- skills;
- knob offsets.

A plugin influences the soul only through the gateway's levers. It never reaches inside the
core.

**Architecture invariant:** an adapter never thinks. It never decides for the soul whether
something is worth perceiving, saying or doing. It senses, translates and carries out.

**Architecture invariant:** one broken plugin never stops the soul. An installed plugin that
fails is left out and the owner is told. Only a broken configuration or a broken built-in
plugin stops start-up.

### Lab

The Lab runs scenarios against the real program through test bodies. It can set any mechanism to
its neutral value (ablation) and measure the difference against that mechanism's claim. Every run
records what produced it: core version, prior version, model, valve state and switches.
Scenarios come in three sets: development (used to design), validation (checked now and then)
and sealed (never seen while designing).

The Lab is how the core earns its changes. Nothing is claimed about the soul that the Lab does
not show.

## How it runs

### One turn of the loop

1. Take one event from the queue.
2. Record it, then update state.
3. Pass the inbound door.
4. Check open expectations.
5. Appraise without any model call: either a commitment short-circuits the draw, or signals,
   salience, the threshold and a seeded draw decide.
6. If reasoning wakes: build the context, pass the context door and run the model in the
   background. Its result comes back later as an event.
7. For each action: pass the action door, record it, hand it to the gateway and open a
   prediction of what should follow.
8. On idle ticks: maybe sleep and consolidate memory.

**Architecture invariant:** there is one queue and one consumer. Every source only puts events
into the queue; only the loop processes them, one at a time, in order.

**Architecture invariant:** record first, then act. Incoming events are written before they are
processed, and a decision to stay silent is written too.

### The path of one event

```
body senses something
  → plugin translates it into a percept
  → gateway maps the actor to a person, labels the audience
  → core records it, then appraises it
       ├─ not worth thinking about → silence (recorded)
       └─ worth it → context → reasoning → action
  → gateway routes the action to the body that owns the place
  → body carries it out and returns a receipt
  → core compares the outcome with its prediction and learns
```

Every action ends in exactly one receipt: done, refused, held, failed or unknown. An outcome that
matches the soul's own prediction is marked as self-caused, so the soul does not react to the
echo of its own acts.

## Boundaries

**Waist 1, core ↔ gateway.** Events go in and actions come out. Context passes through one door.
The gateway acts on the core only through its doors and levers.

**Waist 2, gateway ↔ plugins.** Actors, places, audiences, percepts, actions and receipts,
lifecycle events, answers and commands. A built-in plugin speaks it inside the process. An
installed plugin is a separate program in any language that speaks it as messages, so a crash
on that side never takes the soul down.

**Architecture invariant:** dependencies point one way. The gateway builds on the core's
contract, and plugins build on the gateway's. A lower layer never changes for a need of an upper
one; a need from above enters as a plugin, never as a change to the skin.

## Cross-cutting concerns

### The two absolute lines

1. Only the owner changes who the soul is.
2. Private content never reaches anyone who was not part of it.

Both protect the soul's integrity (who it is, and whose secrets it keeps), not its behavior.
Privacy follows people, not places: every event carries the audience that could perceive it, and
private items are filtered out before the model sees the context, so there is nothing to leak.
The core records the facts these lines need (who started each chain, who could see each item)
even when the valve is off, so switching it back on still protects what was said in between.

### Identity

The soul's identity is the core's own: its name, persona and principles, with one source in the
configuration. It always exists; a default is created on first start. Every change to it is
recorded as an identity change, and only the owner can make one.

### Memory

Everything that persists is an event. State is a projection of the log and can be rebuilt from
it. The log is append-only; the one exception is hard forgetting, through a single path that
records its own event. Recall ranks memories by activation (how recent and how often used), not
by similarity alone.

### Configuration and secrets

One configuration file holds everything the owner sets. It carries a version, and an older
version is migrated automatically after a backup. Secrets and endpoints live apart from it and
never appear in the log or in a receipt's text. What the soul learns lives in its own memory,
not in the configuration.

### Versions in data

Every stored format carries its version. Older versions are upcast when read and never
rewritten; a newer version than the program knows fails loudly. Names stored in data are part
of the protocol; a rename touches code only.

### Reproducibility

Randomness comes only from a seeded generator, and time only from an injected clock. Every model
call is a gate decision, recorded with its purpose and cost, and counted in a daily budget.
Replaying the same events gives the same decisions.

### Failure

Nothing fails silently. A guard rule at a door fails closed; a best-effort rule is skipped.
Every hook call and background job runs under a time limit. One part can fail; it cannot freeze
the loop.

### Complexity budget

The core holds a small, fixed number of primitives: books, thought acts, doors, signals, drives
and one gate formula. A new primitive must replace an existing one or be refused.

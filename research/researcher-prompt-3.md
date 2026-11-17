# Research brief 3 for MonoAgens: external benchmark datasets

You are the researcher for MonoAgens. Read `research/researcher-prompt-2.md` for the project and
the rules every finding must respect; they still hold. This round is narrow: the public
benchmark datasets the Lab must score against.

## Read first

Repository: https://github.com/AMQStudio/MonoAgens, branch `main`.

1. `openspec/changes/lab-datasets/` — `proposal.md`, `design.md`, and `specs/`. Every
   `[pending R-nn]` there waits on this brief.
2. `crates/lab/src/benchmark.rs`, `standard_benchmark_registry()` — the 22 registered datasets,
   their domains, the primary metrics the Lab expects, and the caveats recorded in Run 2.
3. `research/findings/2026-11-16-run-2.md`, `R-39` — why these benchmarks were chosen.

The registered ids are: `locomo`, `longmemeval`, `memory_agent_bench`, `group_mem_bench`,
`stale`, `pm_bench`, `pis`, `pitch`, `proact`, `proactive_bench`, `capa`, `when2speak`,
`ubuntu_irc`, `molweni`, `fantom`, `opentom`, `hi_tom`, `exploretom`, `tomi` (optional),
`multilexnorm`, `indocollex`, `mpbench`.

## Rules for this brief

- Every fact needs a primary source: the dataset's own repository, dataset card or paper. Give
  the link for each one.
- Say plainly when you cannot confirm that a dataset exists publicly under that name, or when a
  name in the registry matches more than one dataset.
- Do not choose for the owner. Where a choice is needed (for example, which of two splits),
  give the options and what each one implies.
- Licenses: report the terms. Clearing them is the owner's decision.

## Questions

**R-51 Provenance.** For each registered id:
- whether it exists publicly, and its canonical name;
- its canonical source (git repository, Hugging Face dataset, or direct download), plus a pinned
  revision (commit SHA or release tag) and its date;
- the files needed and their sizes;
- whether the source publishes checksums;
- its license (SPDX id if any), and whether it allows local copies, commercial use and
  redistribution of derived files, plus any access gate (sign-up, request form).

**R-52 Official evaluation protocol.** For each dataset and each primary metric the registry
lists:
- the official definition;
- the split meant for reporting;
- the official scoring script and its revision;
- whether a language-model judge is required, and if so which model and prompt;
- known contamination in model training data;
- which listed metrics are not official, and what the closest official one is.

**R-53 Mapping to percepts.** MonoAgens perceives only events with the six facts (actor, place,
audience, event, capability, receipt) in time order, and never sees gold answers. For each
dataset, describe how its units (sessions, turns, speakers, questions, probes, timestamps) map
to percepts and to what the soul must do or say. Say which datasets cannot be represented
without leaking gold or inventing facts (for example, missing timestamps or speakers).

**R-54 Sampling and power.** `core-and-lab` targets a paired non-inferiority comparison with
`S >= 354–471` items (`A-16`). For each dataset, say:
- how many items it holds per relevant split;
- whether a seeded subset can reach that power without biasing the categories (for example
  LoCoMo Category 5);
- how to stratify the subset;
- a rough per-item model-call cost shape (turns per item, judge calls per item), so the owner
  can budget live runs.

## Deliver

One findings file, `research/findings/<date>-run-3.md`, with a table per question keyed by
dataset id. The architect turns the answers into manifest entries and tasks. The owner clears
licenses.

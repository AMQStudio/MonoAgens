# Birth inputs — Decisions

Status: decided points are final unless marked **Draft**; Draft points state the current
intent and may change without a new decision. Nothing in this layer may change the core
([core rulings](../core/DECISIONS.md), frozen). Decision numbers (Dxx) trace to the history in
git.

---

## 1. Birth inputs
- **Birth certificate** (`soul.yaml`): name, language, persona, principles; persona may be in any
  language. Read once at birth (D35).
- **Prior**: a local folder loaded at birth — `manifest.yaml` (format version, date, embedder,
  LLM family, language), `dispositions.yaml` (knobs, calibration, constants),
  `references.yaml`, `lessons.yaml`, `procedures.yaml` (with credence). Replay scenarios that
  prove a prior live in the repository's `scenarios/`, not inside it. A default prior ships with
  the repository (D25, D30, D34, D36).
- **Knobs**: about ten values named by behavior — talkativeness, restraint after speaking,
  spontaneity, helpfulness, boredom with repeats, patience, reserve when unfamiliar, speaking
  bar, certainty bar, memory persistence — each with an "if the soul is too X" guide. One
  function maps them to θ, τ, γ and the weights. Calibration is measured per embedder and model,
  never set by hand; constants change only by a decision. End users never see the prior
  (D36, D40).
- **Loading rules**: different embedder → calibration falls back to defaults; different LLM family
  → load, halving procedure and lesson credence until proven; unknown major format → refuse with
  a clear message (D30).
- This repository only loads a prior; it never exports, uploads, shares or aggregates learned
  state (D18, D25).

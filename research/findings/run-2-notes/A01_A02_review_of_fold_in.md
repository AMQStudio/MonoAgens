# A-01 Review of the fold-in

**Status:** answered

## Answer

An exhaustive audit of `openspec/changes/core-and-lab/` (`proposal.md`, `design.md`, `tasks.md`, and all 14 files under `specs/*/spec.md`) against Round 1 (`research/findings/2026-11-15-run-1.md`, findings `R-01`–`R-28`), `RESEARCH.md`, and `ARCHITECTURE.md` finds that the fold-in is structurally faithful to Round 1 and the owner rulings in Decision 1, with several helpful engineering extensions and **14 deviations, omissions, or internal contradictions needing correction** in `core-and-lab` (plus 1 acknowledged temporary invariant departure in Decision 13).

The highest-impact defects in `core-and-lab` are:
1. **Sign inversion when combining `R-19` and `R-26` (`design.md` D12; `bounded-learning/spec.md`):** treating a missed judged silence as `r = 0` inside `w ← clamp(w + learning_rate·(r − P)·s)` produces `(0 − P) < 0`, which *lowers* signal weights after a missed silence instead of raising them, while answered judged silences have no `Good` outcome at all.
2. **Unreachable neutral values in `R-01`/`R-03`/`D6`/`D18`:** `w_sc = −(0.2 + 0.6·restraint)` and `a = 0.1 + 0.5·restraint` cannot reach `0` for `restraint ∈ [0, 1]`; Activation recall (`A = B + context_weight·cos + importance_weight·importance`, Row 10) has no parameter to zero `B` to reach `similarity only`; and 10 of the 22 rows rely on structural/enum switches rather than numeric parameters despite `ARCHITECTURE.md` and `design.md` D18 requiring neutral values reachable without separate code paths.
3. **Missing protocol and thought-act fields required by Decisions 8, 10, 11, and 12:** `design.md` D5, `protocol/spec.md`, and `tasks.md` 2.1–2.2 omit `continued-attention` (`D8`), physical effect `channel`/`Δ_obs`/`Δ_pred` (`D8`), external `domain` id (`D10`), and `repair` (`D12`); `design.md` D9 omits `domain?` on thought acts (`D10`, `beliefs/spec.md`) and typed fields for `consolidate` outputs (`D11`); `expectations/spec.md` L13 uses singular `place and source` vs plural `places, people` (`D8`, `R-07`) while `expect` lacks `places`/`people`; and `sleep-consolidation/spec.md` L105 uses `resolved as reached`, which is absent from `resolve`'s outcome enum (`D9`).
4. **Vector fade threshold mismatch (`R-03` vs `D6`/`D11`):** `R-03` sets `fade threshold = −2.6` using the old `§5` `B` formula (`n=1, L=30d, d=0.5`), whereas Petrov's hybrid (`D6`) gives `−3.29` and `D11` applies `fade_threshold` to total activation `A` (which has no query context during sleep).
5. **Unsupported or hazardous scenarios across specs:** `sleep-consolidation/spec.md` L104–106 asserts `resolved as reached`; `event-log/spec.md` L14–16 requires crash recovery of a recorded-but-unprocessed event without support in `design.md` D3 (`Core::open`); and `lab/spec.md` L100–102 flags mechanisms with no measured metric change even though 11 of 22 rows have no metric in `lab/spec.md` (see A-02).

---

### Summary Verdict Matrix (`R-01` to `R-28` and Cross-Document Checks)

| Item | Primary Location in `core-and-lab` | Summary Comparison vs Round 1 / `ARCHITECTURE.md` | Verdict |
|---|---|---|---|
| Owner rulings (4+4) | `design.md` D1 (L47–58) | Resolves owner, text search, effect vs response, doors, `κ`, Petrov hybrid, `memory.recall`, `info_gap` | `faithful` (`Store` port text search: `acceptable extension`) |
| `R-01` Brain map | `design.md` D18 (L429–463); `cognitive-loop/spec.md` (L87–99) | All 22 rows match; Row 10 `similarity only` unreachable via parameter; 10 rows use structural off-states; Row 18 omits lessons/procedures; Row 6 `τ → 0` needs step-limit handling | `deviation needing correction` (Row 10 neutral parameterization) |
| `R-02` `social_cost` | `design.md` D6 (L171–174); `appraisal-gate/spec.md` (L150–165) | Matches formula; graded address deferred (`D` Non-Goal); `w_sc = 0` conflicts with `R-03` map | `faithful` (see `R-03` for `w_sc = 0` reachability) |
| `R-03` Dispositions | `design.md` D6, D11, D15; `lab/spec.md`; `tasks.md` 3.8, 9.7, 10.10, 20.3 | `w_info = 0` overrides `0.6·helpfulness`; `w_sc`/`a` cannot reach `0`; `fade_threshold = −2.6` mismatches `D6`/`D11`; pre-calibration defaults & learning-bound params missing; `1/n` check missing from `lab/spec.md`; `per model` clarified to `per embedder` with `calibrate` | `deviation needing correction` (`w_sc`/`a` neutral, `fade_threshold`, missing defaults) |
| `R-04` Familiarity & closeness | `design.md` D6 (L179–188); `appraisal-gate/spec.md` (L135–149) | Petrov hybrid matches (`n > k`, `t_n > t_k` guard unstated); `appraisal-gate/spec.md` prose says "closeness" instead of `B_p` | `deviation needing correction` (missing denominator guard `n > k, t_n > t_k` in documented formula) |
| `R-05` Importance | `design.md` D7 (L202–205); `memory-recall/spec.md` (L110–113); `tasks.md` 5.2, 16.2 | Exact match; `i_surprise` staged into Stage B | `faithful` |
| `R-06` `info_gap` | `design.md` D1, D19 (L471); `tasks.md` 9.7 | Shipped with empty references and `w_info = 0`; deferred to A-14 | `faithful` |
| `R-07` Goals, chains, threads | `design.md` D4, D6, D8 (L234–240); `expectations/spec.md` (L11–14, L182–200) | Attribution order matches; `expect` lacks `places`/`people`; `expectations/spec.md` L13 says singular `place and source`; "relevance raised" unquantified; `sleep-consolidation` uses `reached` | `deviation needing correction` (`expect`/spec record mismatch & `reached`) |
| `R-08` Timing by medium | `design.md` D8 (L218–223); `expectations/spec.md` (L169–181) | `continued-attention` omitted from `D5`/`protocol`/`tasks.md` 2.2; `stakes factor` undefined; D19 omits R-08 open estimator | `deviation needing correction` (missing protocol fact `continued-attention`) |
| `R-09` Sense of agency | `design.md` D1, D8 (L224–227); `expectations/spec.md` (L71–93) | Rules match; `D5`/`protocol` omit percept `channel`, `Δ_obs`, and capability `Δ_pred`; echo rule requires chat capability to declare sensed effect | `deviation needing correction` (missing protocol fields for physical effects) |
| `R-10` Promise failure | `design.md` D8 (L228–233); `expectations/spec.md` (L32–53) | Drops `(promise_debt = 1)` at start-up overdue; `renegotiated`/`kept_late` absent from `resolve` enum; `resolve(..., kept)` vs receipt timing | `acceptable extension` (except `resolve`/state alignment) |
| `R-11` Context rebuild | `design.md` D9 (L271–280), D15 (L400–402); `reasoning/spec.md` (L11–68); `lab/spec.md` (L223–230) | Adds sorted `capabilities`, knowledge cutoff (`§11`), truncation priority, and `baseline` command; omits `baseline` from D15 command list (L393–394) and omits non-inferiority margin | `acceptable extension` (with D15 command-list omission & missing margin) |
| `R-12` Core primitives | `design.md` D1, D4, D9 (L257–269); `protocol/spec.md`; `memory-recall/spec.md` | 6 books & `memory.recall` match; adds `basis?`, `value?`, `cost?` on `think` and `expected?` on `act`; omits `domain?` (`D10`) and `consolidate` typed fields (`D11`); `resolve` lacks `reached` | `deviation needing correction` (missing `domain?`, `consolidate` fields, `reached`) |
| `R-13` Embedding model | `design.md` D14 (L365–376); `model-providers/spec.md` (L45–65) | Matches EmbeddingGemma-300M 256-dim; adds `scripted`/`none` providers and double tagging (vectors + run record) | `faithful` / `acceptable extension` |
| `R-14` Backlog chunking | `design.md` D6 (L193–196); `appraisal-gate/spec.md` (L116–134) | "down" -> "stopped"; "asleep" retained (open in A-04); formula & 4-chunk limit match | `faithful` |
| `R-15` Control of report | `design.md` D9 (L284–287); `reasoning/spec.md` (L138–160) | Restricts to communicative acts; adds `ContextPlan` instruction for coarser grain; omits `value+cost == 0` guard, auto-recall query construction, and informativeness metric | `deviation needing correction` (missing zero/omitted `value, cost` guard & auto-recall query) |
| `R-16` Recollection vs familiarity | `design.md` D7 (L208–211); `memory-recall/spec.md` (L82–100) | Hard forgetting dropped (Non-Goal); "normalized activation" unnormalized; summary-marked-recollected prose slightly blurred | `faithful` |
| `R-17` Source monitoring | `design.md` D10 (L293–296); `beliefs/spec.md` (L11–29) | Adds "downgrade is recorded" with no dedicated event kind in D4; neutral label phrasing differs slightly from D18 | `acceptable extension` |
| `R-18` Content miss | `design.md` D8 (L244–248); `expectations/spec.md` (L137–150) | Adds explicit 4 stored fields; "token count" -> generic "length"; scenario prose blurs z-score-only trigger | `acceptable extension` |
| `R-19` Judging silence | `design.md` D8 (L249–253), D12 (L339); `expectations/spec.md` (L151–168); `bounded-learning/spec.md` (L14) | `note` + seeded `ρ` of `silent` match; adds missed judged silence to `r = 0` in delta rule (sign inversion bug!); omits `r = 1` for answered silence; `reply-linked or addressed` differs from `R-08` "anyone's reply" | `deviation needing correction` (sign inversion bug in learning) |
| `R-20` Credit for evidence | `design.md` D10 (L300–302); `beliefs/spec.md` (L55–76, 92–99) | Matches formula & `used_ids`; `beliefs/spec.md` L56 omits `told` items while L86 credits them via source prior | `deviation needing correction` (internal contradiction in `beliefs/spec.md` L56 vs L86) |
| `R-21` Reliability by domain | `design.md` D10 (L303–305), D11 (L322); `beliefs/spec.md` (L77–91); `tasks.md` 15.4, 16.6 | Matches concept; omits `domain` field in `D5`/`protocol`/`D9`; `tasks.md` 16.6 adds "seeded method" | `deviation needing correction` (missing `domain` in protocol/D9) |
| `R-22` Sleep (wave 1) | `design.md` D9, D11 (L307–324); `sleep-consolidation/spec.md` (L1–110) | Adds resumability events (`episode_closed`, `sleep_run`) and belief supersession (`§12`); `> N_min` vs `≥ N_min`; `consolidate` acts lack typed fields; `resolved as reached`; verbatim check vs no-content-read rule | `deviation needing correction` (`> N_min` mismatch, `reached`, `consolidate` act fields) |
| `R-23` Event boundaries | `design.md` D11 (L311–314); `sleep-consolidation/spec.md` (L24–43) | Matches formula & fallback; adds "fixed interval SHALL NOT end an episode" | `faithful` |
| `R-24` Merging episodes | `design.md` D11 (L319–320); `sleep-consolidation/spec.md` (L53–61) | Spec extends to "two episodes or their gists"; scenario varies both place and source | `acceptable extension` |
| `R-25` Perception ledger | `design.md` D12 (L328–333); `perception-ledger/spec.md` (L1–75) | Adds `body_state` for self card; limits cards in context to triggering event's audience (misses promisees on ticks) | `acceptable extension` (audience edge case on ticks) |
| `R-26` Learning | `design.md` D12 (L335–352); `bounded-learning/spec.md` (L1–113) | Sign bug on missed silence; `repair` signal undefined; teacher weight missing from inline formula; step-cap & freeze-days params missing from `R-03` | `deviation needing correction` (sign bug & undefined `repair`) |
| `R-27` Human-likeness | `design.md` Non-Goals, D15; `lab/spec.md` (L130–139); `tasks.md` 19.1 | `SPEAK/SILENT` annotations & 0.8/0.8 worked example match; still embeds `1/n` metric; blind test excluded | `faithful` |
| `R-28` Citations | `design.md` D19 (L476); `tasks.md` 21.2–21.4 | All R-28 corrections & `RESEARCH.md` fixes scheduled in `tasks.md` 21 | `faithful` |
| Cross-cutting & Invariants | `design.md` D2, D3, D4, D13, D15, D19; `event-log/spec.md`; `tasks.md` | `research/backlog.md` missing on disk (scheduled in `tasks.md` 21.1); D13 acknowledges omitting absolute lines; crash-before-process resume omitted from `D3`; `baseline` missing from D15 command list; `wakes per event` & `kept late` in `tasks.md` 10.5 absent from `lab/spec.md` | `deviation needing correction` (D15 command list, crash-resume gap, `lab/spec.md` vs `tasks.md` 10.5) |

---

### Detailed Audit: Exact Quotes, Line Numbers, and Verdicts

#### 1. Owner Rulings & Terminology (`design.md` Context, D1, D3)
- **Hook-point terminology vs `ARCHITECTURE.md`:**
  - `research/findings/2026-11-15-run-1.md` L17: *"ARCHITECTURE.md says 'gateway', 'doors', 'valve' and 'Waist 1, core ↔ gateway', and its 'hooks' are something a plugin contributes. §16 says 'the core gives hooks, the dispatch door and the gate'; ARCHITECTURE.md gives the doors to the gateway. ... Use ARCHITECTURE.md's terms."*
  - `openspec/changes/core-and-lab/design.md` D1 L54: *"Who owns the hooks (§16 against `ARCHITECTURE.md`) | The core owns the three hook points and calls them inside its turn. What runs in them (the valve, plugin hooks) belongs to the gateway."* Also `openspec/changes/core-and-lab/specs/cognitive-loop/spec.md` L48: *"Three doors are hook points for whatever governs the soul"*.
  - **Verdict:** `faithful` to Decision 1 and compatible with `ARCHITECTURE.md` L168–175 (the core calls `Doors` at turn steps 3, 6, 7), though using the phrase "hook points" for doors slightly overlaps `ARCHITECTURE.md` L136 ("hooks" as plugin contributions).
- **Text search behind `Store` port:**
  - `research/findings/2026-11-15-run-1.md` L20: *"Either move it behind a gateway capability that returns ids, or amend the invariant."*
  - `openspec/changes/core-and-lab/design.md` D1 L52: *"Text search sits behind the store port. The core passes the id of the query event; the store reads text and returns ids."*
  - **Verdict:** `acceptable extension`. Putting text search behind `Store` (`monoagens-store`) instead of a gateway capability keeps `monoagens-core` free of content reading (`ARCHITECTURE.md` L85–86).
- **Per-actor numbers replacing "owner" in core:** `design.md` D1 L51 supplies `(α₀, β₀)` and `teacher weight` per actor (`faithful`).

#### 2. `R-01` The Brain Map (`design.md` D18; `cognitive-loop/spec.md`; `tasks.md` 3.7)
- **22 rows match:** Comparing `2026-11-15-run-1.md` L61–84 with `design.md` D18 L437–461, every row, mechanism, neutral value, claim, and section matches (`faithful`), including preserving `R-01` Row 18's omission of lessons/procedures (`idle gists; merge on people, place, time`) and Row 15's omission of knob/band/scoped learning (`delta rule`). Minor wording edits (`recall@k at 7/30 days` -> `recall at 7 and 30 days`; `promise-keeping F1` -> `promise keeping`) keep meaning, though `R-01` L59 explicitly excluded *"reasoning from absence and the global workspace"*, which `design.md` D18 L433 leaves out (*"Engineering concerns (§9–11, §13, §16) get no row"*).
- **Row 10 neutral value unreachable via parameter:**
  - `design.md` D18 L432: *"with a neutral value reachable by a parameter"*. (`ARCHITECTURE.md` L90–91: *"Switching a mechanism off means moving it to that value, never choosing another code path."*)
  - `design.md` D18 L448 (Row 10): *"Activation recall | B + context + importance | similarity only | recall at 7 and 30 days"*.
  - `design.md` D7 L201: *"`A = B + context_weight·cos + importance_weight·importance`"*.
  - While `importance_weight = 0` drops importance, **no parameter in `R-03` or `design.md` can zero `B`** (`d = 0` leaves `B = ln(n) ≠ 0`). Furthermore, 10 of 22 rows (`7, 12, 13, 14, 16, 18, 19, 20, 21, 22`) rely on boolean/enum `Switches` (`off`, `none`, `left to reasoning`, `frozen`, `each off`, `none; never`, `absent`, `no marking`, `fixed interval`, `per event`) rather than numeric parameters.
  - **Verdict:** `deviation needing correction` (Row 10 needs an explicit `base_level_weight` parameter or `Switches` exemption must be documented against `ARCHITECTURE.md` L90–91 and `design.md` D18 L432).
- **Row 6 `τ → 0` limit:** `P = σ((S_eff − θ) / τ)` divides by zero at `τ = 0`, and `spontaneity` maps to `[0.02, 0.2]`, never reaching `0` (`RESEARCH.md` L72–73). Code must evaluate the `τ → 0` step function when Row 6 is neutral (`acceptable extension`).

#### 3. `R-02` `social_cost` & `R-03` Dispositions Table
- **`w_sc` and `a` cannot reach neutral `0`:**
  - `research/findings/2026-11-15-run-1.md` L153: *"restraint ρ → w_sc = −(0.2 + 0.6ρ), a = 0.1 + 0.5ρ."*
  - `openspec/changes/core-and-lab/design.md` D6 L176 & L178: *"`w_sc = −(0.2 + 0.6·restraint)`"* and *"with `a = 0.1 + 0.5·restraint`"*.
  - `openspec/changes/core-and-lab/design.md` D18 L440 & L442: Row 2 neutral `w_sc = 0`; Row 4 neutral `a = b = c = 0`.
  - For `restraint ∈ [0, 1]`, `w_sc ∈ [−0.8, −0.2]` and `a ∈ [0.1, 0.6]`, so neither neutral value is reachable via `restraint` (`deviation needing correction` inherited from `R-03` into `design.md` D6).
- **`w_info = 0` overrides `0.6·helpfulness`:**
  - `design.md` D6 L175: *"`w_info = 0.6·helpfulness`"*.
  - `design.md` D19 L471: *"`info_gap` is built with no reference set and `w_info` = 0; the set arrives with the next change"*.
  - `tasks.md` 9.7 L93: *"keep `w_info` at 0 in the prior"*.
  - **Verdict:** `acceptable extension` for Stage A/B, though `prior/dispositions.json` must clarify whether `w_info` is temporarily an explicit override field alongside `helpfulness`.
- **`fade_threshold = −2.6` vs Petrov's hybrid and `A` vs `B`:**
  - `research/findings/2026-11-15-run-1.md` L177: *"`fade threshold | −2.6 (B, single use, 30 days, d = 0.5)`"*.
  - `openspec/changes/core-and-lab/design.md` D11 L321: *"events with `A < fade_threshold` get `vector_released`."*
  - `openspec/changes/core-and-lab/specs/sleep-consolidation/spec.md` L71–72: *"events whose activation has fallen below the fade threshold SHALL be released."*
  - First, `−2.6` was computed from the discarded `RESEARCH.md` §5 formula (`ln(1/(1−0.5)) − 0.5·ln(720) = −2.597`); under Petrov's hybrid (`D6` L186) with `k = 1, n = 1, t_1 = 720 h, d = 0.5`, `B = ln(720^(−0.5)) = −3.29`. Second, `D11` compares `A` (which includes `context_weight·cos(context, memory)`), even though background sleep has no query context (`cos` undefined unless set to `0` or fading uses `B`).
  - **Verdict:** `deviation needing correction`.
- **`calibrate` command and pre-calibration / learning-bound parameters:**
  - `design.md` D15 L397–399 clarifies `R-03`'s *"per model"* (L172) to *"per embedder"* and adds `calibrate` (`lab/spec.md` L213–222) (`acceptable extension`).
  - However, `R-03` gives no numeric defaults for `fok_threshold`, `fok_scale`, `wake_gate_lo`, or `lo bounds` before `calibrate` runs (or when `embed = none`), and `bounded-learning/spec.md` L52 & L61–62 reference *"the configured share of its range"* and *"the configured number of days"* which are absent from `R-03` (`tasks.md` 3.8 needs explicit entries for all of them).
  - Also `R-03` L184 claims *"between 0.5× and 2× the 1/n rate"* (`tasks.md` 20.3), whereas `lab/spec.md` L132–133 reports the share against `1/n` without encoding the `[0.5, 2.0]` check (`acceptable extension`).

#### 4. `R-04` Familiarity and Closeness (`design.md` D6 L179–188; `appraisal-gate/spec.md` L135–149)
- **Missing denominator guard in Petrov's hybrid:**
  - `design.md` D6 L186: *"`B ≈ ln[Σ_{j=1..k} t_j^(−d) + (n−k)(t_n^(1−d) − t_k^(1−d)) / ((1−d)(t_n − t_k))]`, with `t_j` the ages in hours of the k most recent uses, `t_n` the age of the first use, n the use count, and t floored at one minute."*
  - When `n ≤ k` or `t_n == t_k` (e.g., all uses occurred within the last minute and sit at the 1-minute floor), `(t_n − t_k) = 0` produces `0/0`. Neither `R-04` L211 nor `design.md` D6 L186 states the guard `n > k ∧ t_n > t_k` (`deviation needing correction` in documented formula).
- **Spec prose conflates `closeness` with `B_p`:**
  - `appraisal-gate/spec.md` L138–139: *"Familiarity of a place SHALL combine the activation of the soul's presence there **with its closeness to those present**."*
  - `design.md` D6 L183 computes `B_ctx = (B_P + Σ_{p∈Present} B_p) / (1 + |Present|)`, `familiarity = σ(B_ctx)`—combining `B_P` with raw activations `B_p` rather than `closeness(p) = σ(B_p)` (`acceptable extension` / loose prose in spec; follow `D6`).
- **Graded address omitted:** `R-04` L205 `"(or with address degree ≥ 0.5)"` is excluded by `design.md` Non-Goals L41 (`faithful`).

#### 5. `R-05` Importance & `R-06` `info_gap`
- **`R-05`:** `design.md` D7 L202–205 and `memory-recall/spec.md` L110–113 match `max(i_reason, i_commit, i_surprise)`; `tasks.md` 5.2 implements the first two in Stage A and `16.2` adds `i_surprise` in Stage B (`faithful`).
- **`R-06`:** `design.md` D1 L57–58, D19 L471, and `tasks.md` 9.7 leave the reference set empty and `w_info = 0`, awaiting A-14 (`faithful`).

#### 6. `R-07` Goals, Chains, and Threads (`design.md` D4, D8 L234–240; `expectations/spec.md` L11–14, L182–200)
- **Plural `places, people` vs `expect` and `expectations/spec.md`:**
  - `design.md` D8 L235 (matching `R-07` L315): *"It holds its chain, **places, people**, a completion condition (description and vector), a due time and an expiry."*
  - `design.md` D9 L262: *"`expect(kind, about_ids, condition, due, expiry)`"* (no `places` or `people` arguments).
  - `expectations/spec.md` L12–13: *"An expectation SHALL be a record with a kind (effect, response, wait, promise or goal), a condition, **the place and source** it concerns, an expiry and the event that created it."*
  - **Verdict:** `deviation needing correction` (contradiction between plural `places, people` for goals and singular `place and source` in `expectations/spec.md`, plus missing place/person inputs on `expect`).
- **Unquantified `relevance raised`:** `design.md` D8 L239 repeats `R-07` L321 (*"relevance raised"*) without specifying the formula (`faithful` to `R-07`).
- **`resolved as reached` absent from `resolve` enum:**
  - `sleep-consolidation/spec.md` L104–105: *"Scenario: A goal was reached — WHEN an episode contains a goal that was **resolved as reached**"*
  - `design.md` D9 L263 (and `R-12` L530): *"`resolve(id, outcome ∈ {kept, failed, cancelled, answered})`"* (`reached` does not exist!).
  - **Verdict:** `deviation needing correction` (internal contradiction between `sleep-consolidation/spec.md` and `design.md` D9).

#### 7. `R-08` Timing by Medium (`design.md` D8 L218–223, D5; `protocol/spec.md`; `expectations/spec.md` L169–181)
- **Missing `continued-attention` protocol fact:**
  - `design.md` D8 L223 (from `R-08` L362): *"Answering evidence is a `done` receipt, the addressee's next event in the place, or **a continued-attention number the body reports**."*
  - `design.md` D5 L156–166 (`What the core is told`), `protocol/spec.md` L99–110 (`A percept carries what its body can report`), and `tasks.md` 2.2 L22 list `addressees, named, reply_to, correlation id, length, negative flag` and omit any `continued-attention` field.
  - **Verdict:** `deviation needing correction` (internal contradiction between `D8` and `D5`/`protocol/spec.md`/`tasks.md` 2.2).
- **Undefined `stakes factor` and `D19` table omission:** `design.md` D8 L220 repeats *"times a stakes factor"* without defining it (open in A-08), and `D19` does not mention `R-08` in its table (`faithful` to `R-08`, minor `D19` omission).

#### 8. `R-09` Sense of Agency (`design.md` D8 L224–227, D5; `protocol/spec.md`; `expectations/spec.md` L71–93)
- **Missing `channel`, `Δ_obs`, and `Δ_pred` fields in protocol:**
  - `design.md` D8 L225–226 (from `R-09` L400): *"when it is on the declared channel within `[t + lat_min, t + lat_max]` and `|Δ_obs − Δ_pred| ≤ k·σ_channel`."*
  - `protocol/spec.md` L58–59 and `design.md` D5 L162 define `channel`, `window`, and `noise` on the capability descriptor, **but neither `D5`, `protocol/spec.md` L99–110, `D9`, nor `tasks.md` 2.1–2.2 defines `Δ_pred` (predicted change) or a percept's reported `(channel, Δ_obs)`**.
  - **Verdict:** `deviation needing correction`.
- **Echo detection vs capability effect declaration:**
  - `expectations/spec.md` L55–56 opens an `effect` prediction *"when the capability declares a sensed effect"*, and L73 marks self-caused *"only when it matches an open effect prediction"*. Thus L78–80 (*"A body reports the soul's own message back"*) requires communicative capabilities on echoing bodies to declare a sensed effect (`acceptable extension` / body-descriptor constraint).

#### 9. `R-10` Promise Failure (`design.md` D8 L228–233; `expectations/spec.md` L32–53)
- **Omitted `(promise_debt = 1)` at start-up overdue:**
  - `R-10` L443: *"At start-up, promises with due < now are marked overdue (`promise_debt = 1`), and reasoning wakes once per place."*
  - `design.md` D8 L232–233: *"At start-up, promises with due < now are marked overdue and reasoning wakes once per place."*
  - **Verdict:** `acceptable extension` (keeps `promise_debt` as the `[0, 1]` due-soon share per `expectations/spec.md` L103, though it quietly drops `R-10`'s `promise_debt = 1` override).
- **`renegotiated` state & `resolve(..., kept)` timing:** `resolve` enum (`D9` L263) lacks `renegotiated` (`revise` presumably transitions it), and `expectations/spec.md` L43–44 (*"carries out the promised act and resolves the promise"*) must not mark `kept` before the keeping `act` gets a `done` receipt per L33–35 (`acceptable extension`).

#### 10. `R-11` Context Without a Transcript (`design.md` D9 L271–280, D15 L393–402; `reasoning/spec.md` L11–68; `lab/spec.md` L223–230)
- **Context ordering additions (`capabilities` and `<now>` knowledge cutoff):**
  - `R-11` L479–486 lists 7 slots: `1. identity; 2. cards ...; 3. commitments ...; 4. working set ...; 5. recalled memories ...; 6. last N events; 7. <now> and the trigger`.
  - `design.md` D9 L271–276 and `reasoning/spec.md` L12–15, L48–68 insert **`capabilities (sorted)`** at position 2 and append **`with the model's knowledge cutoff`** to `<now>` (`RESEARCH.md` §11 L195–198).
  - **Verdict:** `acceptable extension` (required for tool use, prefix caching in `reasoning/spec.md` L55, and temporal grounding in `RESEARCH.md` §11).
- **Context truncation order added in `reasoning/spec.md` L26–30:** *"lowest-ranked recalled memories and the oldest recent events SHALL be left out first; identity, the triggering event and fired expectations SHALL never be left out."* (`acceptable extension`; note `design.md` D9 omits the context size limit parameter, whereas `reasoning/spec.md` and `tasks.md` 7.1 enforce it).
- **Cross-body scenario conditionality (`reasoning/spec.md` L18–20):** *"WHEN a source talks to the soul through one body and continues the topic through a different body -> THEN the context for the second wake contains the relevant earlier events although they happened in another place"*. Note this depends on `memory-recall` retrieving those cross-place events into top-`k` (`faithful` when recall hits; would fail if `embed = none` and wording differs).
- **`baseline` command & missing non-inferiority margin (`design.md` D15 L393–394, L400–402; `lab/spec.md` L223–230; `tasks.md` 10.11):**
  - `design.md` D15 adds `baseline` (`D2` L99 `baseline.rs`, `D15` L400–402, `lab/spec.md` L223–230, `tasks.md` 10.11), **but `design.md` D15 L393–394 omits `baseline` from the CLI command list** (`run, compare, coverage, calibrate, mechanisms, log, explain, replay, talk`). Also, `R-11` L502 required *"with a non-inferiority margin fixed in advance"*, which neither `D15` nor `lab/spec.md` specifies (deferred to A-16).
  - **Verdict:** `acceptable extension` for the command, with a `deviation needing correction` for omitting `baseline` in `D15` L393–394.

#### 11. `R-12` The Core's Primitives (`design.md` D1, D4, D9 L257–269; `protocol/spec.md`; `memory-recall/spec.md`)
- **Added thought-act fields (`basis?`, `value?`, `cost?`, `expected?`):**
  - `R-12` L527–528: `think(thought, motivation 1–5, confidence, used_ids[], importance?)` and `act(capability, place, audience, args)`.
  - `design.md` D9 L259–260: `think(thought, motivation 1–5, confidence, used_ids[], importance?, basis?, value?, cost?)` and `act(capability, place, audience, args, expected?)`.
  - **Verdict:** `acceptable extension` (directly reconciles `R-12` with `R-17`, `R-15`, and `R-18`; note `reasoning/spec.md` L139 says *"With an answer, reasoning SHALL be able to state the value... and the cost"*, attaching stakes via `think` rather than `act`).
- **Missing `domain?` field and `consolidate` typed fields:**
  - `design.md` D10 L305 and `beliefs/spec.md` L80 state a domain id may be supplied *"by reasoning"*, yet **none of the five acts in `D9` L259–265 carries a `domain?` field**.
  - `design.md` D9 L288 restricts purpose `consolidate` to *"think and revise only"*, yet `D11` L315–318 and `sleep-consolidation/spec.md` L44–47, L94–98 require consolidation to write typed `gist` (with place/span/sources), `lesson` (`situation, action, outcome`, credence), and `procedure` (action sequence for a goal, credence) items—fields `think` and `revise` in `D9` lack.
  - **Verdict:** `deviation needing correction`.
- **`memory.recall` receipt IDs vs `ContextPlan` (`design.md` D7 L207 vs `memory-recall/spec.md` L74–76):**
  - `D7` L207 specifies *"its receipt (ids only) continues the chain"*, whereas `memory-recall/spec.md` L76 requires *"the next wake of that chain has the recalled items in its context"*—implying the core expands the receipt's IDs into recalled items in the next `ContextPlan` (`acceptable extension`).

#### 12. `R-13` Embedding Model & `R-14` Backlog Chunking
- **`R-13`:** `design.md` D14 L365–376 and `model-providers/spec.md` L45–65 add `scripted`/`none` embedders and enforce tags on both stored vectors and run records (`faithful` / `acceptable extension`).
- **`R-14`:** `design.md` D6 L193–196 and `appraisal-gate/spec.md` L116–134 replace "down" with "stopped" and retain "asleep" (reopened in A-04) (`faithful`).

#### 13. `R-15` Control of Report (`design.md` D9 L284–287; `reasoning/spec.md` L138–160)
- **Added `ContextPlan` coarser-grain instruction & communicative-act scope:**
  - `design.md` D9 L284–287: *"If `confidence < P_rc`: run `memory.recall` and wake again, up to `n_recall` times; then the plan asks for a coarser grain...; an answer still below `P_rc` is withheld."* `reasoning/spec.md` L141 restricts this to *"a communicative act's stated confidence"*. (`acceptable extension`).
- **Missing zero/omitted `(value, cost)` guard and automatic recall query:**
  - Because `value?, cost?` on `think` are optional (`D9` L259), omitting them or setting both to `0` causes `0/0` in `cost / (value + cost)` unless defaulted to `P_rc = certainty_bar`.
  - Neither `D9` L285 nor `reasoning/spec.md` L150–152 specifies what query text/vector the automatic `memory.recall` uses when reasoning tried to answer rather than call recall.
  - **Verdict:** `deviation needing correction`. (Missing informativeness metric is covered in A-02.)

#### 14. `R-16` Recollection vs Familiarity (`design.md` D7 L208–211; `memory-recall/spec.md` L82–100)
- Drops hard forgetting clause (`R-16` L680) because hard forgetting is a Non-Goal (`faithful`).
- `"normalized activation"` in `F = max(cosine, normalized activation)` is not defined operationally in either `R-16` or `D7` (`faithful` to `R-16`).
- `memory-recall/spec.md` L98–99 says *"the summary can still be marked recollected through its links"*, subtly blurring `R-16` L680's rule that R holds iff recall returns *"at least one log event (not a gist)"* (`acceptable extension` / minor wording blur; `design.md` D7 L210 clarifies *"A gist is tested through its source ids"*).

#### 15. `R-17` Source Monitoring & `R-18` Content Miss
- **`R-17` (`design.md` D10 L293–296; `beliefs/spec.md` L11–29):** `beliefs/spec.md` L24 adds *"and the downgrade is recorded"*, though `D4` L143–147 has no dedicated downgrade event kind (must sit on `belief_changed` or `thought`). `R-17` L720 neutral *"Basis hidden from the context"* vs `D18` Row 16 *"each off"* (`acceptable extension`).
- **`R-18` (`design.md` D8 L244–248, D5 L161; `expectations/spec.md` L137–150):** generalizes `R-18`'s *"token count"* to generic *"length"* (`protocol/spec.md` L102) and specifies 4 recorded miss fields (`who replied, latency share, length, distance`) (`acceptable extension`). Scenario L144 (*"short, fast and far from the act"*) should not imply length/latency trigger a miss when `D8` L246 triggers solely on `z_score(miss) > z_miss`.

#### 16. `R-19` Judging Silence & `R-26` Learning — **CRITICAL COMBINATION BUG**
- **`note` vs `silent` silence prediction (`R-19` L776; `design.md` D8 L249–253; `expectations/spec.md` L151–168):**
  - `design.md` D8 L249–250: *"every `note` decision, and a seeded fraction ρ of `silent` decisions, opens a prediction 'someone else responds to the sender within expiry'."*
  - `expectations/spec.md` L152–153 & L165–167: *"Every note decision, and a seeded fraction of silent decisions, SHALL open a prediction..."* (`faithful` to `R-19`).
  - Note: `design.md` D8 L251 follows `R-19` L777 (`reply-linked or addressed to the sender`), differing from `R-08` L360 which said the silence window *"uses the same rule for anyone's reply"* (`patience`, any third-party event in place).
- **CRITICAL SIGN INVERSION IN DELTA RULE (`design.md` D12 L337–342; `bounded-learning/spec.md` L11–16, L30–38):**
  - `design.md` D12 L337–339: *"`Outcome:` r = 1 when a response prediction was answered in time with no miss or repair, or an effect prediction matched; r = 0 on absence, a miss, a repair, a failed receipt, a negative flag from the body, **or a missed judged silence**."* (`bounded-learning/spec.md` L14 matches verbatim.)
  - `design.md` D12 L342: *"`Weights:` `w ← clamp(w + learning_rate·(r − P)·s)`, scaled by the teacher weight of the actor involved."* (`bounded-learning/spec.md` L31–33 matches.)
  - `R-26` L1016–1017 originally defined `r = 1` / `r = 0` only for **spoken acts** (where `P` is the probability of `go`). By placing *"a missed judged silence"* into `r = 0` without flipping the target relative to `go`, a silent/note event with `P = 0.2` produces `(r − P) = (0 − 0.2) = −0.2 < 0`—which **decreases** positive signal weights `w` and drives `P` **lower** after a missed silence instead of higher! Meanwhile, an *answered* silence prediction (`someone else handled it`) is absent from `r = 1` / Good outcomes. To match `R-19` (*"missed help decreases with learning"*), the target for `P = P(go)` must be `1` for good acts AND missed silences, and `0` for bad acts AND answered silences.
  - **Verdict:** `deviation needing correction` (severe bug from combining `R-19` with `R-26`).
- **Undefined `repair` signal (`design.md` D12 L337–338; `bounded-learning/spec.md` L13–14):** both distinguish *"a repair"* from *"a content miss"* and *"a negative flag"*, yet neither `D5` (`protocol`) nor `D9` (thought acts) defines a `repair` event or flag (`deviation needing correction`).
- **Teacher weight & learning-bound params:** `design.md` D12 L342 puts *"scaled by the teacher weight"* in prose rather than inside `w ← clamp(...)` (`acceptable extension`), and `bounded-learning/spec.md` L52, L61–62 introduce configurable step-cap and freeze-days parameters omitted from `R-03`.

#### 17. `R-20` Credit for Evidence & `R-21` Reliability by Domain
- **Internal inconsistency on `told` credence (`beliefs/spec.md` L56 vs L86; `reasoning/spec.md` L168):**
  - `beliefs/spec.md` L56: *"Lessons, procedures and inferred beliefs SHALL carry credence as two evidence counts, starting from a prior."* (omits `told`!)
  - `beliefs/spec.md` L86: *"new **told** items from that source in that domain start with lower credence"* (`reasoning/spec.md` L168 likewise displays `told, by that source, with its credence`).
  - **Verdict:** `deviation needing correction`.
- **Missing `domain` inputs (`design.md` D10 L305; `beliefs/spec.md` L80 vs `D5`, `D9`, `protocol/spec.md`):** neither external percept/actor facts (`D5`, `protocol/spec.md`) nor `think` (`D9`) defines a `domain` field despite `D10` and `beliefs/spec.md` allowing domains supplied *"from outside or by reasoning"* (`deviation needing correction`).
- **Seeded centroid clustering (`tasks.md` 16.6):** requires *"a seeded method"* (algorithm left to A-12; `acceptable extension`).

#### 18. `R-22` Sleep, `R-23` Event Boundaries, `R-24` Merging
- **`> N_min` vs `≥ N_min`:** `R-22` L873 and `design.md` D11 L309 require *"at least `N_min` unconsolidated events"* (`≥ N_min`), whereas `sleep-consolidation/spec.md` L13 requires *"exceed the configured backlog"* (`> N_min`) (`deviation needing correction`).
- **Resumable sleep & belief supersession (`design.md` D11 L323–324; `sleep-consolidation/spec.md` L62–68, L86–93):** adds `episode_closed` and `sleep_run` progress events and stale-belief supersession (`acceptable extension`).
- **Verbatim vs gist check (`beliefs/spec.md` L35–37; `tasks.md` 16.5 vs `ARCHITECTURE.md` L85):** checking whether *"a gist's detail is absent from its source events"* must execute inside `reason` (`consolidate`) via `revise`, never inside `core`, to preserve `ARCHITECTURE.md` L85 (`acceptable extension` if executed by `reason`).
- **Merging *"episodes or their gists"* (`sleep-consolidation/spec.md` L54, L58–60):** extends `R-24` from episodes to *"episodes or their gists"*, and its scenario (L58–60) varies both source and place simultaneously (`acceptable extension`).

#### 19. `R-25` Perception Ledger, `R-27` Human-likeness, `R-28` Citations
- **`R-25`:** adds `body_state` for the self card (`design.md` D4 L147, `tasks.md` 17.3; `acceptable extension`). `perception-ledger/spec.md` L51 limits included cards to *"the sources in the triggering event's audience"*, which can omit a promisee when the wake trigger is a due-time `tick` whose audience is only the soul (`acceptable extension` / edge case to refine).
- **`R-27`:** `SPEAK/SILENT` annotations & 0.8/0.8 example in `lab/spec.md` L130–139; human blind test excluded as Non-Goal (`design.md` L43); `1/n` flagged as MonoAgens assumption in `tasks.md` 21.2 (`faithful`).
- **`R-28`:** citation corrections scheduled in `tasks.md` 21.2–21.4 (`faithful`).

#### 20. Cross-Cutting Invariants, Missing Artifacts, and Spec-vs-Design Contradictions
1. **Missing `research/backlog.md` (`design.md` D2 L103, D19 L466; `tasks.md` 21.1 L201):** `design.md` D19 states *"`research/backlog.md` tracks each question's status"*, but `research/backlog.md` does not exist on disk yet (creation is deferred to `tasks.md` 21.1).
2. **Acknowledged departure from `ARCHITECTURE.md` Absolute Lines (`design.md` D13 L360–363; `ARCHITECTURE.md` L218–227):** `D13` explicitly notes that the two absolute lines are omitted in `core-and-lab` and deferred to `R-33`, with Lab reporting narrow-to-wide leakage as a baseline (`acceptable extension`, transparently documented).
3. **Crash-before-appraisal recovery gap (`event-log/spec.md` L14–16 vs `design.md` D3 L112–117):** `event-log/spec.md` L14–16 asserts *"WHEN the process stops after an event was recorded and before it was processed -> THEN on the next start the event is in the log and is processed"*, which `design.md` D3 (`Core::open` rebuilding projections) does not address (`deviation needing correction` in `D3` startup lifecycle).
4. **Withheld act event kind (`reasoning/spec.md` L132, L159 vs `design.md` D4 L143–147):** `reasoning/spec.md` requires withheld communicative acts to be *"recorded as withheld with both numbers"*, but `D4`'s 26 event kinds do not name a `withheld` event or distinguish it from `receipt(refused)` (`cognitive-loop/spec.md` L64).
5. **CLI & metric drift (`design.md` D15 L393–394 vs L400; `tasks.md` 10.5 vs `lab/spec.md` L140–150):** `D15` L393–394 omits `baseline` from the CLI command list; `tasks.md` 10.5 expects `wakes per event` and `promises kept and kept late`, neither of which appears in `lab/spec.md` L140–150; and `lab/spec.md` L100–102 flags mechanisms with no measured metric change even though 11 of 22 rows lack metrics in `lab/spec.md`.

## Parameters
not applicable

## Neutral value
not applicable

## Claim
not applicable

## How the Lab would test it
not applicable

## Established / inferred / open
- **Directly observed in repository files:** all quoted passages, line numbers, missing fields (`domain?`, `continued-attention`, `channel`/`Δ_obs`/`Δ_pred`, `consolidate` typed fields, `reached`, `repair`), the `fade_threshold` mismatch (`−2.6` vs `−3.29` and `B` vs `A`), the unreachable neutral values (`w_sc = 0`, `a = 0`, Row 10 `similarity only`), the `> N_min` vs `≥ N_min` mismatch, `beliefs/spec.md` L56 vs L86, missing `research/backlog.md`, omitted `baseline` in `D15` L393–394, the `event-log/spec.md` L14–16 vs `D3` startup gap, and the `(r − P)` sign inversion on missed judged silence in `D12` / `bounded-learning/spec.md`.
- **Inferred:** that applying `r = 0` inside `w + learning_rate·(r − P)·s` on missed judged silence drives `P(go)` downward instead of upward, and that `Core::open` must replay/drain trailing unappraised events to satisfy `event-log/spec.md` L14–16.
- **Open (for the change following `core-and-lab`):** fixing the judged-silence target polarity in the delta rule, adding missing fields to `protocol/spec.md` / `D5` / `D9`, restoring parameter reachability for `w_sc`/`a`/Row 10, reconciling `fade_threshold` with Petrov's hybrid and sleep-time `A`, fixing `sleep-consolidation/spec.md` (`> N_min`, `reached`), and aligning `D15`/`lab/spec.md`/`tasks.md` 10.5.

## Sources
- `/home/muqsith/Projects/Mono/ARCHITECTURE.md` (lines 1–270)
- `/home/muqsith/Projects/Mono/RESEARCH.md` (lines 1–652)
- `/home/muqsith/Projects/Mono/research/findings/2026-11-15-run-1.md` (lines 1–1387)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/proposal.md` (lines 1–105)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md` (lines 1–514)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/tasks.md` (lines 1–219)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/appraisal-gate/spec.md` (lines 1–165)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/beliefs/spec.md` (lines 1–106)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/bounded-learning/spec.md` (lines 1–113)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/cognitive-loop/spec.md` (lines 1–128)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/event-log/spec.md` (lines 1–86)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/expectations/spec.md` (lines 1–200)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md` (lines 1–230)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/memory-recall/spec.md` (lines 1–117)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/model-providers/spec.md` (lines 1–88)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/perception-ledger/spec.md` (lines 1–75)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/protocol/spec.md` (lines 1–110)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/reasoning/spec.md` (lines 1–176)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/sleep-consolidation/spec.md` (lines 1–110)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/soul-identity/spec.md` (lines 1–56)

---

# A-02 The brain map against the Lab

**Status:** answered

## Answer

Inspecting all 22 rows of `openspec/changes/core-and-lab/design.md` Decision 18 (lines 437–461) against `openspec/changes/core-and-lab/specs/lab/spec.md` (metrics and ablation requirements at lines 90–150 and 223–230, plus per-scenario check primitives at lines 45–55) shows that:

- **3 rows are cleanly covered** by named metrics in `specs/lab/spec.md`: Row 1 (*Salience*), Row 10 (*Activation recall*), and Row 19 (*Perception ledger*).
- **8 rows are partially covered or underspecified**: Row 3 (*Habituation*), Row 4 (*Drive threshold*), Row 6 (*Stochastic go / no-go*), Row 7 (*Commitment breaker*), Row 13 (*Prospective memory*), Row 15 (*Bounded learning*), Row 16 (*Memory truth*), and Row 18 (*Consolidation and merge*). In these rows, either some sub-claims are covered while sibling sub-claims are absent (Row 4 covers `1/n` but omits `budget overruns` and `overdue promises`; Row 16 covers `stale beliefs` and un-recollected assertions but omits `misattribution` and conflates `confabulation` with `lures`; Row 18 covers `30-day recall` but omits `blended memories`), or the spec reports only raw components rather than the falsifiable ablation metric (Row 6 gives a delay distribution without a distance scalar; Row 13 omits `kept late` despite `tasks.md` 10.5 requiring it; Row 15 gives precision/recall without F1 or multi-week improvement).
- **11 rows have NO metric in `specs/lab/spec.md`**: Row 2 (*Floor cost*), Row 5 (*Familiarity reserve*), Row 8 (*Patience*), Row 9 (*Report bar on thought*), Row 11 (*Vector fade*), Row 12 (*Prediction and grounding*), Row 14 (*Credence*), Row 17 (*Report control*), Row 20 (*Agency*), Row 21 (*Event boundaries*), and Row 22 (*Backlog chunking*).

Without adding those metrics to `specs/lab/spec.md`, the Lab's ablation command (`lab/spec.md` lines 90–103: *"report, per metric, the difference with a confidence interval ... A mechanism changes nothing"*) cannot test 19 of the 22 rows in full, even though `tasks.md` 11.6, 12.4, 13.4, 14.5, 15.6, 16.8, 17.5, 18.7, and 20.5 assume every row has a measurable ablation difference.

---

### Complete 22-Row Brain Map Audit Against `specs/lab/spec.md`

| # | Function (`D18`) | Claim ("Worse without it") | Status in `specs/lab/spec.md` | Exact Metric / Quote in `specs/lab/spec.md` (and `tasks.md`) | Metric Needed Where Missing or Underspecified |
|---|---|---|---|---|---|
| 1 | Salience | `when-to-speak recall, missed help` | **Covered** | L131–133: *"when-to-speak precision and recall, false speaks, missed replies"* | Covered (`when_to_speak_recall` and `missed_replies` on `SPEAK`-annotated help requests; optionally report `missed_help_on_unaddressed` separately). |
| 2 | Floor cost | `taking others' floors` | **Missing** | None. (`false speaks` at L132 aggregates all unwanted speaks and does not isolate other-floor turns.) | `floor_violations_per_100`: communicative acts by the soul responding to events where `sel_other = 1` or `answered = 1`, per 100 such events (`R-02` L130). |
| 3 | Habituation | `wakes per repeated inconsequential stimulus` | **Partially covered / underspecified** | Scenario L96–98: *"the report shows the number of wakes for both, their difference and its confidence interval"*; general metrics L143 list *"tokens and model calls per event"* (and `tasks.md` 10.5 adds *"wakes per event"*), not repeated-stimulus wakes. | `wakes_per_repeated_stimulus`: wakes (`wake_started` / `go`) divided by consecutive unaddressed, unrewarded events from a repeating `(place, source)` pair; plus add `wakes_per_event` to `lab/spec.md` L143 to match `tasks.md` 10.5. |
| 4 | Drive threshold | `rate against 1/n; budget overruns; overdue promises` | **Partially covered (1 of 3)** | `1/n`: L132–133 *"the soul's share of messages against one over the number of participants"*. `budget overruns`: **missing** (only `tokens ... per event` at L143 and hard cost-guard refusal at L78–80). `overdue promises`: **underspecified** (only `promises kept on time` at L141). | (a) `daily_budget_overrun_rate`: fraction of simulated days where `tokens_today > daily_budget` (and mean `max(0, tokens_today/daily_budget − 1)`) measured without hard cutoff masking; (b) `overdue_promise_rate`: share of promises passing `due` without attempt/resolution. |
| 5 | Familiarity reserve | `false speaks in new places` | **Missing** | None. (`false speaks` at L132 is global across all places/ages, not stratified by place familiarity.) | `false_speak_rate_unfamiliar`: `false speaks` per 100 unallocated events occurring in places with low familiarity (e.g., `familiarity < threshold` or initial contact window), contrasted against familiar places. |
| 6 | Stochastic go / no-go | `distance to human timing` | **Underspecified** | L133–134: *"and the distribution of its reply delays."* (Reports the distribution, not a scalar distance to a human baseline.) | `reply_delay_distance`: scalar distance (e.g., 1-Wasserstein distance or two-sample Kolmogorov–Smirnov statistic) between the soul's reply-delay distribution and the human baseline delay distribution in the same group (`lab/spec.md` L92 requires a per-metric difference with CI). |
| 7 | Commitment breaker | `unanswered allocated turns` | **Partially covered / underspecified** | General L132 *"missed replies"* and per-scenario check L46 *"an action did or did not happen in a place within a time"*; does not isolate `addressed = true` turns. | `unanswered_allocated_turn_rate`: fraction of events with turn allocated to the soul (`addressed` includes soul) that receive no communicative act (or wake) within the scenario window. |
| 8 | Patience | `double answers` | **Missing** | None. (`false speaks` at L132 requires manual `SILENT` annotation after each third-party reply and conflates double-answering with other false speaks.) | `double_answer_rate`: count/rate of soul communicative acts on an unallocated event sent after another actor (neither sender nor soul) already replied in that place within the patience window. |
| 9 | Report bar on thought | `low-value messages` | **Missing** | None. (`false speaks` at L132 measures speaking when silent was warranted, not message value/utility.) | `low_value_message_rate`: share of unallocated communicative acts annotated low-value (or, for ablation verification, acts sent despite `motivation < speaking_bar` or `confidence < certainty_bar` when bars are neutral). Needs scenario value annotations. |
| 10 | Activation recall | `recall at 7 and 30 days` | **Covered** | L141: *"recall hits at gaps of one, seven and thirty days"*. | Covered (ensure both retrieval hit rate `recall@k` and answer accuracy on 1/7/30-day probes, including commitment/surprising items per `R-05`). |
| 11 | Vector fade | `storage and latency` | **Missing** | None. (`design.md` D16 L416 and `tasks.md` 22.2 benchmark `feeling_of_knowing` at 10⁵ events in `docs/testing.md`, but `lab/spec.md` has no storage or latency metric.) | `active_vector_count` / `vector_storage_bytes` and `recall_latency_ms` / `appraisal_latency_ms` (mean and p95, including at scale). |
| 12 | Prediction and grounding | `unnoticed non-uptake` | **Missing** | None. (Only per-scenario check L47 *"an expectation ... reached a given state"*, not an aggregate Lab metric.) | `unnoticed_non_uptake_rate`: fraction of unacknowledged acts, expired response/effect predictions, content misses, repair-initiating replies (`R-18`), or missed silences where the core failed to record a miss/error or wake reasoning for repair. |
| 13 | Prospective memory | `promise keeping` | **Partially covered / underspecified** | L141: *"promises kept on time"*. (`tasks.md` 10.5 adds *"promises kept and kept late"*, absent from `lab/spec.md`.) | `promises_kept_on_time_rate`, `promises_kept_late_rate`, `promises_kept_total_rate` (`(kept + kept_late) / total_due`), `promises_renegotiated_rate`, and `promises_failed_rate` (matching `R-10` L455). |
| 14 | Credence | `calibration` | **Missing** | None. | `credence_brier_score` and `credence_ece` (Expected Calibration Error comparing stored expected credence `E = α/(α+β)` of beliefs, lessons, procedures, and `(source, domain)` reliability against empirical outcome correctness). |
| 15 | Bounded learning | `no improvement over weeks` | **Underspecified** | L132: *"when-to-speak precision and recall"* (no F1 and no early-vs-late improvement over time). | `when_to_speak_f1` and `when_to_speak_f1_improvement` (`F1_late_window − F1_early_window` across multi-week scenarios, matching `R-26` L1035). |
| 16 | Memory truth | `misattribution; confabulation; lures; stale beliefs` | **Partially covered (1 covered, 2 conflated, 1 missing)** | L141–142: *"assertions of detail made without recollection; superseded beliefs presented as current"*. (`misattribution` is **missing**; `lures` and `confabulation` are conflated under `assertions of detail made without recollection`.) | Keep `assertions_without_recollection` (`confabulation`) and `superseded_beliefs_as_current` (`stale beliefs`); add `source_misattribution_rate` (wrong source or claiming `observed` for `told`/`inferred`) and `lure_error_rate` (asserting gist-consistent unpresented DRM lures when verbatim contradicts or lacks them). |
| 17 | Report control | `wrong volunteered answers` | **Missing** | None. (`assertions of detail made without recollection` at L142 tests ungrounded memory details, not factual error rate vs withholding/coarsening under stakes, nor `R-15` informativeness.) | `wrong_volunteered_answer_rate` (incorrect answers volunteered instead of withheld or coarsened, especially high-stakes) paired with `answer_informativeness` (exact vs coarse vs withheld accuracy–informativeness curve per `R-15` L661). |
| 18 | Consolidation and merge | `30-day recall; blended memories` | **Partially covered (1 of 2)** | `30-day recall`: L141 *"recall hits at gaps of one, seven and thirty days"* (and token cost at L143). `blended memories`: **missing**. | `blended_memory_error_rate` (answers or recalls merging entities/places/events across distinct-source episodes) paired with `duplicate_episode_rate` (to also detect the `never merge` trade-off in `R-24` L957). |
| 19 | Perception ledger | `false-belief errors` | **Covered** | L143: *"correct answers on scripted false-belief questions"*. | Covered (`false_belief_error_rate = 1 − correct_answers_on_false_belief_questions`). |
| 20 | Agency | `reactions to own echoes` | **Missing** | None. | `self_echo_wakes_per_100_acts` (wakes or responses triggered by the soul's own echoed message or predicted physical effect) paired with `false_self_attribution_rate` (external events wrongly marked `self`, per `R-09` L417). |
| 21 | Event boundaries | `boundary F1` | **Missing** | None. (Scenarios at L131 annotate `SPEAK/SILENT` only; there is no event-boundary annotation schema or boundary metric.) | `boundary_f1` (precision, recall, and F1 of `episode_closed` boundaries against annotated ground-truth episode boundaries in scenarios, within a specified event/time tolerance). |
| 22 | Backlog chunking | `bursts after downtime` | **Missing** | None. | `post_downtime_wake_burst` (wakes and communicative acts emitted in the immediate recovery window per 100 backlogged events or per backlogged place after downtime). |

---

### Additional Cross-Row Observations on `specs/lab/spec.md`

1. **Discrepancy between `tasks.md` 10.5 and `specs/lab/spec.md` lines 140–150:** `tasks.md` 10.5 (line 101) implements *"wakes per event, calls and tokens by purpose, promises kept and kept late, recall hits at gaps, narrower-audience items in wider contexts"*. Neither `wakes per event` (Row 3) nor `kept late` (Row 13) appears in `specs/lab/spec.md` lines 140–150 (`wakes` appears only inside the Habituation scenario at lines 96–98).
2. **Missing scenario annotation formats in `specs/lab/spec.md`:** `lab/spec.md` line 131 specifies per-event `warranted reply` (`SPEAK/SILENT`) annotations, and line 143 implies scripted false-belief questions. Measuring Rows 2, 9, 12, 16, 17, 18, and 21 across scenarios additionally requires explicit scenario annotations for: `low_value` vs helpful messages (Row 9), ground-truth `episode_boundaries` (Row 21), `stakes (value, cost)` and ground-truth accuracy/grain (Row 17), `basis`/source truth & DRM lures (Row 16), and cross-episode entity identities (Row 18).
3. **Non-brain-map metrics already present in `specs/lab/spec.md`:** Lines 144–145 track *"the share of prompt tokens served from cache"* (from `R-11` / `§9`) and *"the number of items from a narrower audience placed in a wider context"* (leakage baseline for `R-33` / Decision 13). Those serve engineering and policy baselines rather than Decision 18's 22 rows.

## Parameters
not applicable

## Neutral value
not applicable

## Claim
not applicable

## How the Lab would test it
not applicable

## Established / inferred / open
- **Directly observed in repository files:** all 22 rows of `design.md` Decision 18 (lines 437–461) and every requirement/metric in `openspec/changes/core-and-lab/specs/lab/spec.md` (lines 45–55, 78–150, 213–230) and `tasks.md` (lines 101, 184–185). The tally of 3 covered, 8 partially covered/underspecified, and 11 missing rows is verified directly against those texts.
- **Inferred:** the operational metric definitions proposed in the rightmost column (e.g., `floor_violations_per_100`, `credence_brier_score`/`ece`, `reply_delay_distance`, `boundary_f1`, `post_downtime_wake_burst`) follow directly from Round 1's claims (`R-01`–`R-27`) and `specs/lab/spec.md`'s ablation structure, while exact thresholds/tolerances (such as unfamiliarity window for Row 5 or boundary tolerance window for Row 21) are Lab schema choices.
- **Open (for the change following `core-and-lab`):** updating `specs/lab/spec.md` and `lab/src/metrics.rs` / scenario TOML annotations so all 22 rows have explicit, CI-reported metrics matching `tasks.md` ablation requirements.

## Sources
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/design.md` (lines 381–463, Decision 15 & Decision 18)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/specs/lab/spec.md` (lines 1–230)
- `/home/muqsith/Projects/Mono/openspec/changes/core-and-lab/tasks.md` (lines 95–200)
- `/home/muqsith/Projects/Mono/research/findings/2026-11-15-run-1.md` (lines 51–1080, `R-01`–`R-27` claims & Lab tests)

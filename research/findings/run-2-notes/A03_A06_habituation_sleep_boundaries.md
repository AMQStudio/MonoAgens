# A-03 Habituation halves the first stimulus

**Status:** answered

## Answer

Yes — the habituation increment MUST apply **after** evaluating effective salience `S_eff`, not before; decaying `h` before evaluation and incrementing `h` only afterward restores full unhabituated sensitivity (`S_eff = S`) on a first stimulus and full spontaneous recovery (`S_eff → S`) after a long pause, while still attenuating repeated stimuli from the second presentation onward. Across classical and computational habituation literature, habituation is explicitly defined as a response decrement resulting from **repeated** applications of a stimulus (Thompson & Spencer 1966; Rankin et al. 2009 Characteristic #1), whereas pre-evaluation increment (`h ← h·γ^Δt + 1; S_eff = S / (1 + κ·h)` in `RESEARCH.md` §3, `R-01`, and `openspec/changes/core-and-lab/design.md` Decision 6 line 177) forces `h ≥ 1` and `S_eff ≤ S/2 ≤ 0.50` even on stimulus 1 (`κ = 1`), mathematically blocking unaddressed questions (`own_name = 0`) from exceeding `θ₀ = 0.60` (R-03 default) or strictly exceeding `θ₀ = 0.50` (`RESEARCH.md` midpoint) and capping post-pause spontaneous recovery at `S/2`. Evaluating first (`h_prior ← h·γ^Δt`, `S_eff = S / (1 + κ·h_prior)`) and updating afterward (`h ← h_prior + 1`) requires no new primitive, preserves `κ = 0` as neutral, fixes the `core-and-lab` first-stimulus bug, and makes post-pause salience genuinely approach that of an unhabituated first stimulus (`specs/appraisal-gate/spec.md` lines 44–46).

```text
h_prior ← h · γ^Δt
S_eff   = S / (1 + κ · h_prior)
P(wake) = σ((S_eff − θ) / τ)       where θ = θ₀ + a · (1 − familiarity)
h       ← h_prior + 1              (applied once per appraised event or backlog chunk after S_eff)
```

- `h ≥ 0`: per-source habituation trace initialized at `h = 0` (dimensionless count-equivalent trace).
- `Δt ≥ 0`: elapsed time since the previous appraised event from that source (minutes, from the injected clock).
- `γ ∈ (0, 1)`: per-minute exponential recovery factor (spontaneous recovery between arrivals).
- `h_prior`: decayed pre-stimulus habituation state active at arrival (`0` on the first stimulus, returning toward `0` as `Δt → ∞`).
- `κ ≥ 0`: habituation gain (owner ruling, `design.md` Decision 1; neutral `κ = 0` disables habituation so `S_eff = S`).
- `S ∈ [0, 1]`: unhabituated raw salience, per `R-03`: `S = clip_{[0,1]}(0.6·own_name + 0.4·relevance + helpfulness·(0.6·info_gap + 0.4·fok) − 0.5·social_cost)`. Note: `design.md` disables `info_gap` (`w_info = 0`) pending Lab validation.
- `S_eff ∈ [0, 1]`: effective salience fed into the wake gate.
- `θ`: place-adjusted threshold with base `θ₀ = 0.85 − 0.50·talkativeness`, unfamiliarity penalty `a = 0.25 + 0.20·restraint`, and temperature `τ = 0.02 + 0.18·spontaneity` (`R-03`).

**Numerical check with R-03 start values (`own_name = 0` unless noted, `h_prev = 0`, `restraint = 0.5` → `a = 0.35`, `τ ∈ {0.02, 0.11, 0.20}` with default `0.11`):**

| Scenario (`fam=1` unless noted) | Raw `S` | `θ` | BEFORE (`h_used=1, κ=1`): `S_eff` / `P(τ=0.11)` | BEFORE `P` (`τ=0.02 / 0.20`) | AFTER (`h_used=0, κ=1` or `κ=0`): `S_eff` / `P(τ=0.11)` | AFTER `P` (`τ=0.02 / 0.20`) |
|---|---|---|---|---|---|---|
| Typical unaddressed Q (`help=0.5, ig=0.8, fok=0.7, rel=0.3`), `θ₀=0.60` | 0.500 | 0.600 | 0.250 / **0.0399** | 0.0000 / 0.1480 | 0.500 / **0.2872** | 0.0067 / 0.3775 |
| Same typical unaddressed Q, `θ₀=0.50` (`RESEARCH.md` midpoint) | 0.500 | 0.500 | 0.250 / **0.0934** | 0.0000 / 0.2227 | 0.500 / **0.5000** | 0.5000 / 0.5000 |
| Strong unaddressed Q (`help=0.5, ig=1.0, fok=1.0, rel=0.5`), `θ₀=0.60` | 0.700 | 0.600 | 0.350 / **0.0934** | 0.0000 / 0.2227 | 0.700 / **0.7128** | 0.9933 / 0.6225 |
| Strong unaddressed Q, `θ₀=0.50` | 0.700 | 0.500 | 0.350 / **0.2036** | 0.0006 / 0.3208 | 0.700 / **0.8603** | 1.0000 / 0.7311 |
| Max unaddressed Q (`help=0.5, ig=1, fok=1, rel=1`), `θ₀=0.60` | 0.900 | 0.600 | 0.450 / **0.2036** | 0.0006 / 0.3208 | 0.900 / **0.9386** | 1.0000 / 0.8176 |
| Max unaddressed Q (`help=1.0, ig=1, fok=1, rel=1`), `θ₀=0.60` | 1.000 | 0.600 | 0.500 / **0.2872** | 0.0067 / 0.3775 | 1.000 / **0.9743** | 1.0000 / 0.8808 |
| Max unaddressed Q (`help=0.5`), `θ₀=0.85` (quietest soul) | 0.900 | 0.850 | 0.450 / **0.0257** | 0.0000 / 0.1192 | 0.900 / **0.6117** | 0.9241 / 0.5622 |
| Max unaddressed Q (`help=1.0`), `θ₀=0.50` | 1.000 | 0.500 | 0.500 / **0.5000** | 0.5000 / 0.5000 | 1.000 / **0.9895** | 1.0000 / 0.9241 |
| Max unaddressed Q (`help=0.0, ig=1, fok=1, rel=1`), `θ₀=0.60` | 0.400 | 0.600 | 0.200 / **0.0257** | 0.0000 / 0.1192 | 0.400 / **0.1397** | 0.0000 / 0.2689 |
| Max unaddressed Q, new place (`fam=0, help=0.5, rest=0.5`), `θ₀=0.60` | 0.900 | 0.950 | 0.450 / **0.0105** | 0.0000 / 0.0759 | 0.900 / **0.3883** | 0.0759 / 0.4378 |
| Max unaddressed Q, new place (`fam=0, help=1.0, rest=0.5`), `θ₀=0.60` | 1.000 | 0.950 | 0.500 / **0.0164** | 0.0000 / 0.0953 | 1.000 / **0.6117** | 0.9241 / 0.5622 |
| `core-and-lab` (`w_info=0`, `help=0.5, fok=1, rel=1`), `θ₀=0.60` | 0.600 | 0.600 | 0.300 / **0.0614** | 0.0000 / 0.1824 | 0.600 / **0.5000** | 0.5000 / 0.5000 |
| Moderate addressed ping (`name=1, help=0.5, ig=0.5, fok=0.5, rel=0.2`), `θ₀=0.60` | 0.930 | 0.600 | 0.465 / **0.2267** | 0.0012 / 0.3374 | 0.930 / **0.9526** | 1.0000 / 0.8389 |

With pre-evaluation increment (`BEFORE`, `κ = 1`), `S_eff ≤ 0.500` universally on stimulus 1: no unaddressed event (`own_name = 0`) can exceed `θ₀ = 0.60` (`P ≤ 0.2872` at default `τ = 0.11`, `0.0067` at `τ = 0.02`), nor strictly exceed `θ₀ = 0.50` even with maximal signals (`help = 1, info_gap = 1, fok = 1, rel = 1` gives an exact tie `S_eff = 0.500`, `P = 0.500`), and in `core-and-lab` with `w_info = 0` an unaddressed question collapses to `S_eff ≤ 0.300` (`P = 0.0614`). Only an extremely talkative soul (`θ₀ = 0.35`, `talkativeness = 1`) crosses threshold cleanly on stimulus 1 under `BEFORE`. Under post-evaluation increment (`AFTER`), strong/maximal unaddressed questions cross `θ₀ = 0.60` (`P = 0.7128–0.9743`) and `θ₀ = 0.50` (`P = 0.8603–0.9895`) on stimulus 1 while zero-helpfulness (`help = 0` → `S = 0.40`) stays silent (`P = 0.1397`), rapid repetitions decay immediately from stimulus 2 (`S = 0.75`, `Δt = 0.1 min`, `γ = 0.9/min`: `S_eff = [0.750, 0.377, 0.253, 0.190, 0.153]`, `P = [0.7964, 0.1163, 0.0408, 0.0236, 0.0169]`), and a 60-minute pause restores `99.1%` of `S` (`S_eff = 0.7435`, vs `0.3734 = 49.8%` under `BEFORE`). With `κ = 0`, `S_eff = S` on all repetitions (`S_eff = 0.750` forever — nagging is unattenuated).

**Repository error found (`core-and-lab`):** `openspec/changes/core-and-lab/design.md` Decision 6 (line 177) specifies `h ← h·γ^Δt + 1, S_eff = S / (1 + κ·h)` (evaluating `S_eff` after the `+1` increment, copied from `RESEARCH.md` §3 and `2026-11-15-run-1.md` R-01). It must be reversed to decay `h` before `S_eff` and increment `h` after `S_eff`.

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `κ` | Habituation divisiveness gain (`S_eff = S / (1 + κ·h_prior)`) | `1.0` | `0.0–2.0` (`0` = off) | tune in Lab (owner primitive, `design.md` Decision 1 & 6) |
| `γ` | Per-minute spontaneous recovery decay rate (`h_prior ← h·γ^Δt`) | `0.90 min⁻¹` (`τ_rec ≈ 9.5 min`) | `0.50–0.99 min⁻¹` | tune in Lab (`RESEARCH.md` §3 / R-01 placeholder; domain-specific recovery) |
| increment step (`Δh`) | Trace added after evaluating `S_eff` per event/chunk | `+1` per appraisal unit | `0.5–2.0` (subsumed by `κ` on linear trace) | inferred from Staddon & Higa (1996) / Stanley (1976); tune `κ` in Lab |

*(Note: `θ₀`, `a`, `τ`, `w_info`, `w_fok`, `w_rel`, `w_sc`, `w_name` retain R-03 start values.)*

## Neutral value

`κ = 0` (`S_eff = S` for all stimuli, disabling habituation while the loop continues to run normally; notice increment order has no effect at `κ = 0`).

## Claim

Without post-evaluation increment (keeping pre-evaluation increment `h ← h·γ^Δt + 1` before `S_eff` at `κ = 1`), first-stimulus recall on unaddressed questions (`own_name = 0`) at default dispositions (`θ₀ = 0.60` or `0.50`, `helpfulness = 0.5`) collapses (`P ≤ 0.287` / `0.500`), forcing false silence unless `κ` is lowered to `0` (which breaks repeat suppression). Switching to post-evaluation increment preserves first-stimulus missed-help recall alongside repeat-burst false-speak suppression on repeated nagging (`κ = 1`), while setting `κ = 0` increases false-speak rate under rapid low-pause repetition.

## How the Lab would test it

Run sealed multi-turn chat and sensor scenarios under four arms (`N ≥ 50` scenarios covering cold-start unaddressed questions, addressed pings, rapid 5-message nag bursts from one actor, and 30–120 min return-after-pause turns), sweeping `(θ₀ ∈ [0.35, 0.85], κ ∈ {0, 0.5, 1.0, 1.5}, γ ∈ [0.6, 0.98])`:
1. `pre-increment` (`h ← h·γ^Δt + 1` before `S_eff`, `κ = 1`, current `core-and-lab` spec).
2. `post-increment` (`h_prior ← h·γ^Δt`, `S_eff = S/(1 + κ·h_prior)`, `h ← h_prior + 1`, `κ = 1`).
3. `neutral` (`κ = 0`).

Measure: first-stimulus unaddressed question recall (`missed-help` rate), repeat-burst `false-speak` rate on turns 2–5 (`Δt < 1 min`), and post-pause (`Δt ≥ 30 min`) recovery recall parity vs stimulus 1. Pass criterion for `post-increment`: strictly higher first-stimulus helpfulness recall than `pre-increment` at matched burst `false-speak` rate, and lower burst `false-speak` rate than `κ = 0` at matched first-stimulus recall.

## Established / inferred / open

- **Established:** Habituation is defined by response decrement upon *repeated* stimulation (`k ≥ 2`) with partial or complete spontaneous recovery toward baseline during stimulus absence [PR Thompson & Spencer 1966; PR Rankin et al. 2009 Characteristic #1 & #2]. Dual-process theory models decremental S–R habituation separately from non-associative baseline arousal/sensitization rather than suppressing initial stimulus gain prior to repetition [PR Groves & Thompson 1970]. Leaky-integrator and synaptic-efficacy models initialize the inhibitory habituation trace at zero (`0`) before stimulus 1 so decrement begins only after initial transmission [PR Stanley 1976; PR Staddon & Higa 1996]. Under R-03 parameters and `κ = 1`, Python evaluation proves pre-evaluation increment caps stimulus 1 at `S_eff ≤ 0.500`.
- **Inferred:** Setting `h_prior ← h·γ^Δt`, `S_eff = S/(1 + κ·h_prior)`, then `h ← h_prior + 1` transfers this property into MonoAgens's multiplicative-divisive gate without adding parameters, and evaluating `+1` once per backlog chunk (when events are batched by `patience`) prevents delayed offline catch-up from over-habituating a source before live interaction resumes.
- **Open:** Optimal `γ` per medium (chat turn cadence vs high-rate robotics/telemetry sensors, where single `+1` increments per millisecond telemetry frame could saturate `h` unless sensor bodies normalize `Δt` or event rate at the body bridge), and whether `h` should increment less when the soul actually chose to reply vs stayed silent (Groves & Thompson 1970 / Staddon & Higa 1996 model stimulus-driven decrement regardless of overt reply, which is simpler and avoids reward leakage).

## Sources

- **[PR]** Thompson, R. F., & Spencer, W. A. (1966). Habituation: a model phenomenon for the study of neuronal substrates of behavior. *Psychological Review*, 73(1), 16–43. DOI: `10.1037/h0022681` | PMID: `5324565`
- **[PR]** Groves, P. M., & Thompson, R. F. (1970). Habituation: a dual-process theory. *Psychological Review*, 77(5), 419–450. DOI: `10.1037/h0029810` | PMID: `4319167`
- **[PR]** Stanley, J. C. (1976). Computer simulation of a model of habituation. *Nature*, 261, 146–148. DOI: `10.1038/261146a0`
- **[PR]** Staddon, J. E. R., & Higa, J. J. (1996). Multiple time scales in simple habituation. *Psychological Review*, 103(4), 720–733. DOI: `10.1037/0033-295X.103.4.720` | PMID: `8888652`
- **[PR]** Rankin, C. H., Abrams, T., Barry, R. J., Bhatnagar, S., Clayton, D. F., Colombo, J., Coppola, G., Geyer, M. A., Glanzman, D. L., Marsland, S., McSweeney, F. K., Wilson, D. A., Wu, C.-F., & Thompson, R. F. (2009). Habituation revisited: an updated and revised description of the behavioral characteristics of habituation. *Neurobiology of Learning and Memory*, 92(2), 135–138. DOI: `10.1016/j.nlm.2008.09.012` | PMCID: `PMC2754195` | PMID: `18854219`

---

# A-04 Is the soul ever not listening?

**Status:** answered

## Answer

No — while the process is running, the soul is always listening, and `"asleep"` (background consolidation) MUST NOT delay, chunk as backlog, or raise `θ` on incoming events; only actual **`stopped` / `down`** wall-clock downtime (or ingest lag `> patience`) creates backlog, and body-specific Do-Not-Disturb / **quiet hours** remain strictly in the Body/Valve layer (`ARCHITECTURE.md` line 120). Human sleep research establishes that even deep NREM (N3/SWS) and REM sleep maintain a **"standing sentinel"** evaluation mode that continues discriminating own name and voice familiarity (Oswald, Taylor, & Treisman 1960; Perrin et al. 1999; Blume et al. 2018); behavioral unresponsiveness (`Andrillon et al. 2016`), sleep inertia grogginess (`Tassi & Muzet 2000`; `Hilditch & McHill 2019`), and waking local-sleep lapses (`Vyazovskiy et al. 2011`; `Nir et al. 2017`; `Andrillon et al. 2019, 2021`) are **biological hardware bottlenecks** arising because one neural substrate cannot run synchronous SWS slow-wave down-states while simultaneously executing cortical wake responses. Likewise, homeostatic sleep pressure (`Borbély 1982` Process S → MonoAgens `R-31` `unconsolidated / N_min`) properly governs **when background consolidation triggers**, not arousal suppression. By contrast, software **sleep-time compute** (`Lin et al. 2025`, `arXiv:2504.13171v1`) performs offline context reorganization `S(c) → c'` between queries on an immutable snapshot specifically to speed up and improve live test-time responses `T_b(q, c') → a` without blocking or degrading live queries. Consequently, `core-and-lab` contains an explicit internal contradiction that must be fixed by deleting `"or asleep"` from the backlog rule (`design.md` Decision 6 line 194, `specs/appraisal-gate/spec.md` line 118, and `R-14`) and ensuring consolidation reads a snapshot (`≤ t_snapshot`), yields/completes off the critical path, and delivers updates as loop events.

```text
Δθ_sleep     = 0                          (canonical: background consolidation never raises θ)
is_backlog   = (now − t_event > patience_place) ∨ (t_event occurred while core was stopped/down)
t_snapshot   = clock.now() when consolidation starts; incoming event at t > t_snapshot:
               - resets global idle timer
               - is appraised immediately with normal θ, Δθ_sleep = 0
               - consolidation commits via idempotent delta events after completion
```

- `Δθ_sleep`: sleep-state threshold offset added to `θ`; canonical value `0` (not added as a live Core parameter; ablated `> 0` only if testing biological threshold-elevation models in Lab).
- `patience_place`: place-specific ingest lag threshold (`R-14`).
- `stopped/down`: intervals when the Core runtime process was not executing (restarts, crashes, maintenance), detectable via `(t_last_heartbeat < t_event < t_startup)` or `now − t_event > patience_place`.
- `t_snapshot`: timestamp cutoff isolating background consolidation reads from concurrent incoming events so the cognitive loop never blocks (`specs/sleep-consolidation/spec.md` lines 14–16).

**Contradiction report (`core-and-lab` vs `R-14` vs `R-22`):**
1. **`openspec/changes/core-and-lab/design.md` Decision 6 line 193–194** and **`specs/appraisal-gate/spec.md` lines 117–118** classify an event as chunked backlog if *"it arrived while the soul was stopped or asleep"* (inherited from `2026-11-15-run-1.md` R-14).
2. **`design.md` Decision 11 line 310** and **`specs/sleep-consolidation/spec.md` lines 14–18** require consolidation to run *"as background work whose results return as events, and SHALL never delay the appraisal of an incoming event."*
3. **Resolution:** Delete `"or asleep"` from `R-14`, `design.md` line 194, and `specs/appraisal-gate/spec.md` line 118, leaving only `"stopped"` (actual runtime downtime) and `"lag > patience"`. Notice **quiet hours / circadian DND** are body/social delivery norms, already isolated to the **Body/Valve** (`ARCHITECTURE.md` line 120) so Core remains body-generic across 24/7 sensors, robots, and chat rooms.

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `Δθ_sleep` | Optional threshold offset added while background consolidation is active (ablation only; omit in canonical Core) | `0.0` (fixed off) | `0.0–0.35` (Lab ablation only) | inferred from Blume et al. (2018) / Lin et al. (2025); `0.0` canonical |
| `T_idle` | Global idle duration triggering background consolidation (scheduling only, not gate silencing) | `10 min` | `2–60 min` | R-22 / `design.md` Decision 11; tune in Lab |
| `N_min` | Minimum unconsolidated events before consolidation runs | `50` | `10–200` | R-22 / `design.md` Decision 11; tune in Lab |

*(Note: Core does not add a new runtime primitive for sleep threshold, satisfying `ARCHITECTURE.md`'s fixed primitive budget.)*

## Neutral value

`Δθ_sleep = 0` (which IS the recommended production behavior: consolidation runs asynchronously on snapshots without altering wake thresholds or creating artificial backlog).

## Claim

If `"asleep"` (active background consolidation) triggers backlog batching or raises `θ` (`Δθ_sleep > 0` or simulated sleep inertia), the soul will miss time-critical or high-salience incoming interruptions (urgent messages arriving 1 second after consolidation starts) and experience artificial post-sleep grogginess with no computational upside. Conversely, under asynchronous snapshot-isolated consolidation (`Δθ_sleep = 0`, no `"or asleep"` backlog flag), background reorganization improves subsequent recall/latency (`Lin et al. 2025`) with **zero** regression in live wake responsiveness (`false-silence` / `missed-help` latency unchanged from non-consolidating baseline).

## How the Lab would test it

Using synthetic/logged conversational and sensor streams containing high-priority interruptions injected *during* and *0–300 s after* background consolidation:
1. **`async-sentinel` (proposed):** `Δθ_sleep = 0`, `"stopped"`-only backlog, background consolidation on `≤ t_snapshot`.
2. **`asleep-backlog` (current contradictory wording):** events arriving while a background consolidation job is in flight are held/chunked as backlog until consolidation completes.
3. **`threshold-elevated` (biological sleep + inertia ablation):** `Δθ_sleep ∈ {0.15, 0.30}` during consolidation with linear decay over `[60, 600] s` post-wake.

Measure: latency to respond to critical stimuli injected mid-consolidation, `missed-help` rate on interrupts during/immediately post-consolidation, and downstream multi-session memory recall F1. Pass criterion: `async-sentinel` matches baseline live interrupt `missed-help` and response latency (within 10 ms gate overhead) while retaining full consolidation recall gains, strictly beating `asleep-backlog` and `threshold-elevated` on interruption responsiveness.

## Established / inferred / open

- **Established:** Sleeping humans continue evaluating external auditory salience (own name, familiar vs unfamiliar voices) across all NREM stages and REM ("standing sentinel") [PR Oswald et al. 1960; PR Perrin et al. 1999; PR Blume et al. 2018]. Elevated biological arousal thresholds in sleep stem from cortical slow-wave down-states and neuromodulatory motor gating [PR Andrillon et al. 2016], and produce transient post-awakening performance deficits (sleep inertia) [PR Tassi & Muzet 2000; PR Hilditch & McHill 2019] as well as attentional lapses when sleep-like slow waves occur locally during wakefulness [PR Vyazovskiy et al. 2011; PR Nir et al. 2017; PR Andrillon et al. 2019; PR Andrillon et al. 2021]. Sleep homeostasis modulates sleep propensity/recovery rather than serving a software desirability goal during wake [PR Borbély 1982]. LLM sleep-time compute explicitly uses idle periods between interactions to transform persisted context `S(c) → c'` asynchronously so live test-time latency and accuracy improve [PP Lin et al. 2025, `arXiv:2504.13171v1`; IND Letta `sleep-time-compute`].
- **Inferred:** Separating software consolidation onto immutable read snapshots (`≤ t_snapshot`) while keeping the cognitive loop's gate at normal responsiveness (`Δθ_sleep = 0`) achieves the memory benefits of consolidation without importing biological slow-wave blocking, sleep inertia, or local-sleep lapses, and cleanly removes the `"or asleep"` contradiction in `core-and-lab`.
- **Open:** Whether rate-limited/budget-constrained bodies (paid API quotas) need an external **Valve** backoff policy when budget runs low; by `ARCHITECTURE.md`, any cost-throttling or quiet-hours policy lives in the Body/Valve, not inside Core's appraisal `θ`. Exact snapshot-conflict resolution rules if background consolidation writes a superseded record concurrently with a live episode update (standard compare-and-swap / append-only bi-temporal versioning resolves this mechanically).

## Sources

- **[PR]** Oswald, I., Taylor, A. M., & Treisman, M. (1960). Discriminative responses to stimulation during human sleep. *Brain*, 83(3), 440–453. DOI: `10.1093/brain/83.3.440` | PMID: `13731563`
- **[PR]** Borbély, A. A. (1982). A two process model of sleep regulation. *Human Neurobiology*, 1(3), 195–204. PMID: `7185792`
- **[PR]** Perrin, F., García-Larrea, L., Mauguière, F., & Bastuji, H. (1999). A differential brain response to the subject's own name persists during sleep. *Clinical Neurophysiology*, 110(12), 2153–2164. DOI: `10.1016/S1388-2457(99)00177-7` | PMID: `10616121`
- **[PR]** Tassi, P., & Muzet, A. (2000). Sleep inertia. *Sleep Medicine Reviews*, 4(4), 341–353. DOI: `10.1053/smrv.2000.0098` | PMID: `12531174`
- **[PR]** Vyazovskiy, V. V., Olcese, U., Hanlon, E. C., Nir, Y., Cirelli, C., & Tononi, G. (2011). Local sleep in awake rats. *Nature*, 472, 443–447. DOI: `10.1038/nature10009`
- **[PR]** Andrillon, T., Poulsen, A. T., Hansen, L. K., Léger, D., & Kouider, S. (2016). Neural markers of responsiveness to the environment in human sleep. *The Journal of Neuroscience*, 36(24), 6583–6596. DOI: `10.1523/JNEUROSCI.0902-16.2016` | PMID: `27307244`
- **[PR]** Nir, Y., Andrillon, T., Marmelshtein, A., Suthana, N., Cirelli, C., Tononi, G., & Fried, I. (2017). Selective neuronal lapses precede human cognitive lapses following sleep deprivation. *Nature Medicine*, 23, 1474–1480. DOI: `10.1038/nm.4433`
- **[PR]** Blume, C., del Giudice, R., Wislowska, M., Heib, D. P. J., & Schabus, M. (2018). Standing sentinel during human sleep: Continued evaluation of environmental stimuli in the absence of consciousness. *NeuroImage*, 178, 638–648. DOI: `10.1016/j.neuroimage.2018.05.056` | PMID: `29859261`
- **[PR]** Hilditch, C. J., & McHill, A. W. (2019). Sleep inertia: current insights. *Nature and Science of Sleep*, 11, 155–165. DOI: `10.2147/NSS.S188911` | PMID: `31692489`
- **[PR]** Andrillon, T., Windt, J., Silk, T., Drummond, S. P. A., Bellgrove, M. A., & Tsuchiya, N. (2019). Does the mind wander when the brain takes a break? Local sleep in wakefulness, attentional lapses and mind-wandering. *Frontiers in Neuroscience*, 13, 949. DOI: `10.3389/fnins.2019.00949`
- **[PR]** Andrillon, T., Burns, A., Mackay, T., Windt, J., & Tsuchiya, N. (2021). Predicting lapses of attention with sleep-like slow waves. *Nature Communications*, 12(1), 3657. DOI: `10.1038/s41467-021-23890-7`
- **[PP]** Lin, K., Snell, C., Wang, Y., Packer, C., Wooders, S., Stoica, I., & Gonzalez, J. E. (2025). Sleep-time Compute: Beyond Inference Scaling at Test-time. `arXiv:2504.13171v1` (`https://arxiv.org/abs/2504.13171`)
- **[IND]** Letta Sleep-time Compute reference implementation (`https://github.com/letta-ai/sleep-time-compute`)

---

# A-05 Sleep and the open episode

**Status:** partly answered

## Answer

When sleep consolidation triggers at `t_sleep`, an open episode that has not yet hit a surprise boundary should undergo a **provisional (reversible) soft-close** if it has reached a minimum episode size (`|E_open| ≥ N_ep_min`) or become stale (`t_sleep − t_last > τ_stale`), allowing immediate gist consolidation without waiting forever for a next-day message, while preserving exact R-23 boundary invariance by **reopening** if the next incoming event continues smoothly without crossing the boundary threshold. Active systems consolidation research establishes that both overnight sleep (Diekelmann & Born 2010; Rasch & Born 2013) and brief periods of waking quiet rest immediately post-encoding (Wamsley 2019; Tambini & Davachi 2019) replay and consolidate **just-finished waking experience** rather than deferring consolidation of the most recent experience until the next unrelated session arrives; similarly, LLM sleep-time compute (`Lin et al. 2025`) pre-computes structured representations `S(c) → c'` from the full accumulated context `c` before future queries `q` appear. By contrast, EM-LLM (`Fountas et al. ICLR 2025` / official `ContextManager`) keeps the latest un-evicted segment live in local working context (`n_local = 4096` tokens) and skips closing the trailing incomplete block (`i == len(block_sizes) - 1`) only because raw attention already retains those tokens until eviction, whereas MonoAgens triggers sleep explicitly to build long-term gists and update memories during idle gaps (`T_idle = 10 min`). Leaving an open episode forever un-consolidated ("closed episodes only" in `core-and-lab` Decision 19 line 474) strands the final session of every conversation AND creates an **operational spin bug** whenever the open tail contains `≥ N_min` (50) events (`unconsolidated ≥ N_min` stays perpetually true, firing sleep consolidation every `T_idle` without draining the backlog!). Conversely, unconditionally hard-closing every tail at `T_idle` would fragment a single conversation whenever a coffee break slightly exceeds `T_idle`, violating `specs/sleep-consolidation/spec.md` line 29 (*"A fixed interval SHALL NOT end an episode"*). Three explicit states — **`open`**, **`provisional`**, **`sealed`** — resolve both failure modes.

```text
Episode states:  E.state ∈ {open, provisional, sealed}

1) On sleep trigger at t_sleep (consolidate all `sealed` episodes first):
   For each place's current E_open = [e_k, ..., e_m] (size L = m − k + 1, Δt_place = t_sleep − t_m):
     soft_close = (L ≥ N_ep_min) ∨ (Δt_place > τ_stale)
       where τ_stale = max(p95_place, k_stale · T_idle)   [if place p95 not yet calibrated, τ_stale = k_stale · T_idle]
     If soft_close:
       E.state ← provisional
       consolidate(E)    # record provisional gist & beliefs; mark events [e_k..e_m] counted as consolidated
     Else:
       keep E.state = open   # short 1..(N_ep_min−1) recent tail stays open & unconsolidated

2) On arrival of next event e_{m+1} when the latest episode E is `provisional`:
   Evaluate R-23 boundary predicate B(e_{m+1} | E) against E's stored end timestamp t_m, centroid, and participants:
     If B(e_{m+1} | E) == true  ∨  |E| ≥ N_ep_max:
       E.state ← sealed          # provisional gist confirmed cleanly; zero wasted work
       start new E_open = [e_{m+1}]
     Else:                       # smooth continuation of same episode!
       E.state ← open            # reopen & append e_{m+1}
       mark E's provisional gist as dirty/superseded-on-refresh (preserving bi-temporal `superseded_by`)
       count only newly appended / dirty-episode tail toward next consolidation backlog

   *(No boundary spike threshold is altered: provisional close never injects a synthetic score into R-23 window H_t.)*
```

- `open`: live episode actively accumulating events; not yet summarized into long-term episode gists.
- `provisional`: soft-closed at `t_sleep` so its gist and semantic facts exist for waking recall while remaining reversible if no boundary separates the next arrival.
- `sealed`: permanently closed either by an R-23 surprise/gap/Jaccard boundary or reaching `N_ep_max`.
- `N_ep_min`: minimum event count before soft-closing an idle open tail (prevents fragmented 1–3 turn gists on brief pauses).
- `N_ep_max`: hard upper cap on events per episode (bounded retrieval, analogous to EM-LLM's `max_block_size`).
- `τ_stale`, `k_stale`: staleness timeout allowing even tiny `L < N_ep_min` orphaned tails to close and consolidate once genuinely abandoned (`k_stale · T_idle = 60 min` at start values).
- *Body-genericity note:* `N_ep_min = 4` events suits turn-level chat/action bodies; high-frequency continuous sensor streams (e.g., 10 Hz IMU/telemetry) must aggregate frames at the Body bridge or count state-change events rather than raw sensor ticks so `L ≥ N_ep_min` stays meaningful.

**Repository changes needed (`core-and-lab`):**
1. **`design.md` Decision 19 (line 474):** Resolve open item (`Whether sleep closes an episode that has no boundary yet`) from `"consolidation covers closed episodes only"` to the three-state `open → provisional → sealed` rule above.
2. **`specs/sleep-consolidation/spec.md` (line 45):** Update `"For each closed episode, consolidation SHALL record a gist..."` to cover **closed (`sealed`) or provisionally soft-closed (`provisional`)** episodes, while preserving **line 29** (*"A fixed interval SHALL NOT end an episode"*) because a `provisional` close heals cleanly (`reopen`) unless an actual R-23 boundary (surprise, `p95` gap, Jaccard drop) separates pre-idle and post-idle activity.

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `provisional_close` | Mode switch (`off` = `sealed` only; `on` = `open → provisional` on sleep) | `on` (`1`) | `{off (0), on (1)}` | tune in Lab (inspired by Wamsley 2019 / Lin et al. 2025 / EM-LLM working-buffer eviction) |
| `N_ep_min` | Minimum events in `E_open` required to soft-close at `t_sleep` (before `τ_stale`) | `4` events | `3–12` events | inferred from EM-LLM `min_block_size = 8` tokens (scaled for event granularity); tune in Lab |
| `k_stale` | Staleness multiplier over `T_idle` closing even short `L < N_ep_min` tails (`τ_stale = max(p95, k_stale·T_idle)`) | `6` (`60 min` at `T_idle = 10 min`) | `3–24` (`30–240 min`) | tune in Lab (no literature constant exists for chat abandonment) |
| `N_ep_max` | Maximum events in one episode before sealing on arrival | `128` events | `32–256` events | adapted from EM-LLM `max_block_size = 128` (`emllm_code.txt`); tune in Lab |

## Neutral value

`provisional_close = off` (`N_ep_min = ∞, τ_stale = ∞`): sleep consolidation processes only already-`sealed` episodes, leaving `E_open` untouched (`core-and-lab` Decision 19 current placeholder).

## Claim

Under the neutral rule (`sealed` episodes only), when an active session ends peacefully without a closing topic jump or explicit surprise spike, the final conversation before hours of silence remains invisible to episodic gist recall upon return — and if that single coherent session contains `≥ N_min` (50) events, `unconsolidated ≥ N_min` never clears, causing repeated redundant sleep triggers on stale state. Provisional soft-close + boundary-checked reopening delivers higher next-session recall on recent pre-sleep conversations **without** degrading boundary F1 against human-annotated episode splits compared to both neutral (`sealed` only) and naive hard-close (which over-splits across mid-session pauses).

## How the Lab would test it

Evaluate multi-session human-annotated chat logs separated by short mid-task pauses (`10–25 min`) and overnight gaps (`6–14 h`), where probe questions on wake ask specifically about the **last session before silence**, comparing three arms:
1. `sealed-only` (neutral `core-and-lab` placeholder).
2. `hard-close` (permanently seal `E_open` whenever `T_idle` fires).
3. `provisional-reopen` (proposed rule with `N_ep_min ∈ [3, 12]`, `k_stale ∈ [3, 24]`).

Measure: (a) next-session recall accuracy & latency on facts/commitments from the pre-sleep tail; (b) overall episode boundary F1 against human annotators (checking over-segmentation across `10–25 min` pauses); (c) redundant gist re-generation rate on reopened episodes and overnight spin-trigger count (`0` required).

## Established / inferred / open

- **Established:** Event Segmentation Theory holds that active event models persist until transient prediction errors update them, forming discrete episodic representations [PR Zacks et al. 2007]. Biological offline consolidation (SWS/REM sleep and waking quiet rest) actively reactivates and reorganizes the **most recent waking experience** into gist/neocortical memory rather than waiting indefinitely for a later boundary [PR Diekelmann & Born 2010; PR Rasch & Born 2013; PR Wamsley 2019; PR Tambini & Davachi 2019]. In machine episodic memory, EM-LLM retains the trailing incomplete block inside local context while segmenting older blocks within `[min_block_size, max_block_size]` and refining boundaries post-hoc [PR/PP Fountas et al. 2025; IND `EM-LLM-model`], while sleep-time compute explicitly pre-processes current pre-query context before future queries arrive [PP Lin et al. 2025].
- **Inferred:** No empirical study gives a numerical rule for converting conversational/embodied silent tails into reversible episodic checkpoints. The `open → provisional → sealed` state machine combines biological recent-experience consolidation with EM-LLM's two-stage refinement and R-23's exact boundary test, while preventing `N_min` overnight re-trigger spin loops.
- **Open:** Optimal `(N_ep_min, k_stale)` per medium; whether a reopened `provisional` episode should supersede-and-re-gist immediately on the next sleep vs wait until `L` grows by `≥ ΔL_dirty` events (to limit LLM gist recomputations in cost-sensitive deployments).

## Sources

- **[PR]** Zacks, J. M., Speer, N. K., Swallow, K. M., Braver, T. S., & Reynolds, J. R. (2007). Event perception: a mind-brain perspective. *Psychological Bulletin*, 133(2), 273–293. DOI: `10.1037/0033-2909.133.2.273` | PMID: `17338600`
- **[PR]** Diekelmann, S., & Born, J. (2010). The memory function of sleep. *Nature Reviews Neuroscience*, 11(2), 114–126. DOI: `10.1038/nrn2762` | PMID: `20046194`
- **[PR]** Rasch, B., & Born, J. (2013). About sleep's role in memory. *Physiological Reviews*, 93(2), 681–766. DOI: `10.1152/physrev.00032.2012` | PMID: `23589831`
- **[PR]** Wamsley, E. J. (2019). Memory consolidation during waking rest. *Trends in Cognitive Sciences*, 23(3), 171–173. DOI: `10.1016/j.tics.2018.12.007` | PMID: `30683602`
- **[PR]** Tambini, A., & Davachi, L. (2019). Awake reactivation of prior experiences consolidates memories and biases cognition. *Trends in Cognitive Sciences*, 23(10), 876–890. DOI: `10.1016/j.tics.2019.07.008` | PMID: `31445780`
- **[PR / PP]** Fountas, Z., Benfeghoul, M. A., Oomerjee, A., Christopoulou, F., Lampouras, G., Bou-Ammar, H., & Wang, J. (2025). Human-inspired Episodic Memory for Infinite Context LLMs. *ICLR 2025* (`https://openreview.net/forum?id=BI2int5SAC`) | `arXiv:2407.09450v3` (`https://arxiv.org/abs/2407.09450`)
- **[IND]** EM-LLM reference implementation (`https://github.com/em-llm/EM-LLM-model`, `em_llm/memory/context_manager.py`)
- **[PP]** Lin, K., Snell, C., Wang, Y., Packer, C., Wooders, S., Stoica, I., & Gonzalez, J. E. (2025). Sleep-time Compute: Beyond Inference Scaling at Test-time. `arXiv:2504.13171v1` (`https://arxiv.org/abs/2504.13171`)

---

# A-06 The numbers in R-23

**Status:** partly answered

## Answer

Verified inspection of EM-LLM (`arXiv:2407.09450v3` and official code `em-llm/EM-LLM-model`) establishes that EM-LLM's **Eq. 1** defines `-log P(x_t | x_1,...,x_{t-1}; θ) > μ_{t-τ:t} + γ·σ_{t-τ:t}`, where the paper leaves window offset `τ` symbolic while **Appendix D.1 and the YAML configs confirm `γ = 1.0` as the actual default** across 4 of 5 models (`Mistral-7B`, `LLaMA-3.1-8B`, `Phi-3-mini`, `Phi-3.5-mini`; `LLaMA-3-8B` uses `2.0`, whereas `10⁻³` was used solely for Table 2's PG-19 analysis — resolving `R-23`'s uncertainty note!), and the reference code instantiates `τ` using `n_local = 4096` tokens (`1024` on Phi-3) after `n_init = 128` sink tokens (`avg_st = max(ed - n_local, 0)`, `avg_ed = max(ed - exc_length, n_init)` with `exc_block_size = 512`), `min_block_size = 8`, `max_block_size = 128`, and within-chunk mean/`std` for the first chunk (`ed ≤ exc_length`). Because MonoAgens replaces token-level negative log-likelihood with `s_t = max(PE_time, PE_content, novelty)`, an unscaled `PE_time` z-score (`> 1`) or raw cosine distance (`1 − cos ∈ [0, 2]`) breaks `max(·)`: in synthetic tests here, unbounded positive `PE_time` z-scores (`~5.7` on gaps) inflated `μ + γσ` so semantic `PE_content = 0.85` and `novelty = 0.90` missed boundaries (`F1 = 0.34–0.41`), while rescaling `(1 − cos)/2` squashed orthogonal topic shifts (`cos = 0`) to `0.50` (`F1 = 0.38–0.49`). Clamping all three terms onto a shared `[0, 1]` unit scale (`novelty = clip01(1 − cos)`, `PE_content = clip01(miss)`, `PE_time = clip01(z_time^+ / z_max)` with `z_max = 3.0`) prevents variance domination, keeps `s_t ∈ [0, 1]` commensurate with `R-05` `i_surprise`, and stabilizes causal cold starts (`n < w`) using Empirical-Bayes shrinkage toward `(μ₀, σ₀)` plus a warmup/min-episode guard (`n ≥ w_min`, `ep_len ≥ N_ep_min`).

**Verified EM-LLM (`2407.09450v3` + `em-llm/EM-LLM-model`) baseline parameters vs MonoAgens scale:**

| Parameter | EM-LLM paper (`2407.09450v3`) | EM-LLM code (`config/*.yaml`, `em_llm.py`, `context_manager.py`) | MonoAgens transfer & scale note |
|---|---|---|---|
| `γ` (threshold multiplier) | Eq. 1 symbolic; App. D.1 sweeps `{1..3.5}`, picks `1` (4/5 models) / `2` (LLaMA-3); Table 2 PG-19 uses `10⁻³` | `surprisal_threshold_gamma: 1.0` | `γ = 1.0` confirmed default (test `1.0–1.5` online without post-hoc refinement) |
| `τ` / `w` (window offset) | Eq. 1 symbolic (`μ_{t-τ:t}`) | `avg_st = max(ed - n_local, 0)` to `avg_ed = max(ed - exc_length, n_init)`, `n_local = 4096` (`1024` Phi-3), `exc_block_size = 512` → `3584` prior tokens | Subword tokens vs events (~10–100 tokens/turn): `w = 50` events sits near conversational `n_local` (~500–2500 tok) |
| `n_init` (sink exclusion) | Initial attention sinks kept outside retrieval blocks | `n_init: 128` tokens | Subsumed in MonoAgens by working-memory warmup (`w_min = 8` events) |
| `min / max_block_size` | Similarity-graph boundary refinement span bounds | `min_block_size: 8`, `max_block_size: 128` tokens | Map to event counts `N_ep_min = 4`, `N_ep_max = 128` (`A-05`) |
| Start-of-sequence `< τ` | Same Eq. 1 over chunk | First chunk (`ed ≤ 512`): uses current chunk's `mean` + `γ·torch.std`; chunks `≥ 2` grow strictly causal prior slice `[0 : ed-exc_length]` | MonoAgens is strictly causal event-by-event → needs prior-shrunk moments & `n ≥ w_min` |

**Commensurate `[0, 1]` scaling formulas (`s_t` and cold-start `T_t`, per place):**

```text
1. Component scaling (all mapped cleanly to [0, 1]):
   novelty_t    = clip_{[0,1]}(1 − cos(v_t, c_ep))
     • Defined when v_t exists (if no vector exists, omit & use R-23 vectorless fallback).
     • cos ∈ [−1, 1] ⇒ 1 − cos ∈ [0, 2]; orthogonal (cos = 0) and opposite (cos ≤ 0) map to 1.0.
     • c_ep is the L2-normalized centroid vector of events currently in E_open.

   PE_content,t = clip_{[0,1]}(1 − max(cos(v_t, act_vec), cos(v_t, expected_vec)))   [if response expectation resolves]
                = 1.0                                                                 [if effect/silence expectation confirmed missed]
                = 0.0                                                                 [if no active expectation resolved at event t]
     • Uses exact same [0, 1] angular-dissimilarity ceiling as novelty_t; shares R-18 raw miss definition.

   PE_time,t    = clip_{[0,1]}( z_time,t^+ / z_max )   where for ≥ 2 prior gaps:
                  Δt_t  = max(t_t − t_{t−1}, 0)  (seconds, since previous event in place)
                  u_t   = ln(Δt_t + ε_t),   ε_t = 1 s
                  z_time,t^+ = max(0, (u_t − μ_{u,prev}) / max(σ_{u,prev}, σ_{u,floor}))
                  and for < 2 prior gaps: PE_time,t = 0.0
     • μ_{u,prev}, σ_{u,prev}: causal mean and unbiased std of u over last min(count_gaps, w) gaps in place.
     • Positive part max(0, ·): only unexpectedly LONG gaps trigger episode boundaries; rapid reply bursts stay grouped.
     • / z_max (start z_max = 3.0) reuses R-05's exact z-clamp primitive so a 3σ temporal gap matches an orthogonal topic shift (= 1.0).

   s_t = max(PE_time,t, PE_content,t, novelty_t)   ∈ [0, 1]

2. Causal window statistics and cold start (n = |H_t| = min(t − 1, w) prior scores [s_{max(1,t−w)} .. s_{t−1}]):
   For n = 0:   μ_eff = μ₀,  σ_eff = σ₀,  fire = false
   For n ≥ 1:   μ_n = mean(H_t),   v_n = sample_var(H_t) if n ≥ 2 else 0
                μ_eff = (n · μ_n + m₀ · μ₀) / (n + m₀)
                σ_eff = sqrt( ( (n − 1)·v_n + n·(μ_n − μ_eff)² + m₀·(σ₀² + (μ₀ − μ_eff)²) ) / (n + m₀) )
                T_t   = μ_eff + γ · σ_eff

   Boundary trigger at event t:
     (n ≥ w_min ∧ ep_len ≥ N_ep_min ∧ s_t > T_t)   ∨   (ep_len ≥ N_ep_max)   ∨   (hard fallback if no vectors: Jaccard < 0.5 ∨ Δt_t > p95_gap)

3. Compatibility with R-05 encoding surprise (`i_surprise`):
   z_enc,t    = max(0, (s_t − μ_eff) / max(σ_eff, σ_floor))
   i_surprise = clip_{[0,1]}(z_enc,t / z_max)
```

*(Note on `σ_{u,floor} = 0.35`: prevents identical rapid timestamps e.g. `Δt = 1 s, 1 s, 1 s` from driving `σ_u → 0` and firing `PE_time = 1.0` on a `2 s` gap. Similarly `σ_eff` is regularized smoothly toward `σ₀ = 0.15` for small `n`.)*

**Verification simulation (Python, 3 synthetic 50-event regimes × 20 seeds, 2 true boundaries/seed = 40 positive targets, `w = 50, w_min = 8, μ₀ = 0.25, σ₀ = 0.15, m₀ = 5`):**

| Scaling variant (`ep_min`, `γ`) | Tight chat (P / R / **F1**) | Noisy group (P / R / **F1**) | Bimodal breaks (P / R / **F1**) |
|---|---|---|---|
| Raw unbounded `z_time^+` + raw `1 − cos` (`ep_min=1, γ=1.0`) | 0.33 / 0.50 / **0.40** | 0.24 / 0.55 / **0.34** | 0.35 / 0.50 / **0.41** |
| Scaled `(1 − cos)/2` + `tanh(z/2)` (`ep_min=1, γ=1.0`) | 0.47 / 0.88 / **0.61** | 0.36 / 0.75 / **0.49** | 0.28 / 0.68 / **0.39** |
| Scaled `(1 − cos)/2` + `tanh(z/2)` (`ep_min=4, γ=1.5`) | 0.47 / 0.42 / **0.45** | 0.38 / 0.45 / **0.41** | 0.35 / 0.42 / **0.38** |
| Proposed `clip01(1 − cos)` + `clip01(z^+/3)` (`ep_min=1, γ=1.0`) | 0.54 / 1.00 / **0.70** | 0.36 / 1.00 / **0.53** | 0.45 / 1.00 / **0.62** |
| Proposed `clip01(1 − cos)` + `clip01(z^+/3)` (`ep_min=4, γ=1.0`) | 0.53 / 0.80 / **0.64** | 0.36 / 0.78 / **0.49** | 0.47 / 0.85 / **0.60** |
| Proposed `clip01(1 − cos)` + `clip01(z^+/3)` (`ep_min=4, γ=1.5`) | 0.66 / 0.88 / **0.75** | 0.63 / 0.95 / **0.76** | 0.63 / 0.90 / **0.74** |

In the cold-start trace (`n = 1..14`, routine stream `N(0.22, 0.06)`), unshrunk raw `μ + σ` fired **4 false boundaries** (`n = 1, 6, 8, 13`), whereas Empirical-Bayes pooled `(μ₀ = 0.25, σ₀ = 0.15, m₀ = 5)` held `T_EB ∈ [0.318, 0.380]` with **0 false boundaries**. (Notice also why un-refined online `γ = 1.0` had lower precision than `γ = 1.5`: Gaussian tails exceed `μ + 1.0·σ` ~15.9% of the time without EM-LLM's post-hoc modularity/minimum-block merge pass; `core-and-lab` uses pure online detection, making `γ = 1.0–1.5` directly testable in Lab.)

**Repository gaps found (`core-and-lab`):**
1. **`design.md` Decision 11 (lines 311–314)** and **`R-23`** write `s_t = max(PE_time, PE_content, novelty)` with raw `novelty = 1 − cos(...) ∈ [0, 2]` and leave `PE_time`, `PE_content`, and cold-start windowing (`t ≤ w`) undefined. Without `[0, 1]` normalization and prior shrinkage, temporal z-scores break semantic boundaries and early low-`n` variance over-splits episodes.
2. **`2026-11-15-run-1.md` R-23 (line 920)** states *"1.0 is not confirmed as their default"*; EM-LLM Appendix D.1 and YAML configs confirm `γ = 1.0` IS EM-LLM's default (4 of 5 models).

## Parameters

| Name | Meaning | Start value | Range | Source or "tune in Lab" |
|---|---|---|---|---|
| `γ` | Threshold std multiplier (`T = μ_eff + γ·σ_eff`) | `1.0` (test `1.5` if no post-hoc refine) | `0.8–2.0` | EM-LLM (`2407.09450v3` App. D.1 & configs default `1.0`); tune in Lab |
| `w` | Rolling window of prior event surprises | `50` events | `20–200` events | R-23 (`n_local = 4096` / `1024` tokens in EM-LLM scaled to events); tune in Lab |
| `w_min` | Minimum prior events before surprise spike thresholding activates | `8` events | `5–20` events | adapted from EM-LLM `min_block_size = 8` warmup; tune in Lab |
| `z_max` | Cap converting `PE_time` z-score into `[0, 1]` (`clamp(z^+/z_max, 0, 1)`) | `3.0` | `2.0–4.0` | reuses R-05 `clamp(z/z_max, 0, 1)` primitive; tune in Lab |
| `σ_{u,floor}` | Minimum std floor on `ln(Δt + ε_t)` | `0.35` | `0.2–0.6` | tune in Lab (prevents constant-cadence burst explosion) |
| `(μ₀, σ₀, m₀)` | Cold-start Empirical-Bayes surprise prior mean, std, and pseudo-count weight | `(0.25, 0.15, 5)` | `μ₀ ∈ [0.15, 0.35], σ₀ ∈ [0.10, 0.25], m₀ ∈ [3, 15]` | tune in Lab |

## Neutral value

Fixed-interval / vectorless fallback segmentation (`R-23` / `design.md` line 313: `participant Jaccard < 0.5`, gap above place's `p95`, or prediction miss; or fixed chunk size `N_ep_max` for strict non-adaptive neutral ablation, though `specs/sleep-consolidation/spec.md` line 29 prohibits fixed intervals in production).

## Claim

Without `[0, 1]` commensurate scaling and cold-start variance shrinkage, unbounded `PE_time` z-scores (`> 1`) or `[0, 2]` cosine outliers inflate `μ + γσ` so semantic topic jumps and expectation misses go undetected (`Recall` drops from `1.00` to `0.50–0.55` in simulation), while unregularized sample `σ` in the first `< 8` events over-segments early conversation turns into micro-episodes; commensurate `[0, 1]` scaling plus Empirical-Bayes shrinkage preserves high boundary recall without cold-start fragmentation, improving boundary F1 against human annotations and downstream episode-retrieval precision.

## How the Lab would test it

Run annotated multi-place chat/embodied event logs containing timed gaps, expectation misses, topic shifts, and short cold-start histories (`n = 1..50`) across:
1. Unscaled `s_t` (`raw z_time^+`, `1 − cos ∈ [0, 2]`, unshrunk `μ + γσ` from `n ≥ 2`).
2. Half-scaled `(1 − cos)/2` + `tanh(z/2)`.
3. Proposed `[0, 1]` commensurate scaling (`clip01(1 − cos)`, `clip01(miss)`, `clip01(z_time^+ / 3.0)`) + Empirical-Bayes prior `(μ₀, σ₀, m₀)` + warmup `w_min`, sweeping `γ ∈ {1.0, 1.5, 2.0}` and `w ∈ {25, 50, 100}`.
4. Fixed-interval baseline (neutral reference).

Measure: human-annotated boundary F1 (`±1` event tolerance), early-stream (`t ≤ 20`) false-boundary rate, recall of pure semantic/expectation boundaries immediately following large timing gaps, and downstream recall F1 using segmented episodes. Pass criterion: proposed scaling achieves statistically higher overall and cold-start boundary F1 than unscaled, half-scaled, and fixed-interval baselines.

## Established / inferred / open

- **Established:** Event segmentation is driven by transient prediction-error peaks across perceptual/contextual features [PR Zacks et al. 2007]. EM-LLM defines boundaries at `-log P(x_t | x_{<t}) > μ_{t-τ:t} + γ·σ_{t-τ:t}`, uses `γ = 1.0` as its empirically optimal default across 4 of 5 LLMs in Appendix D.1 and released configs (`n_init = 128`, `n_local = 4096`/`1024`, `min_block_size = 8`, `max_block_size = 128`, first-chunk within-chunk stats) [PR/PP Fountas et al. 2025; IND `EM-LLM-model`]. Raw cosine distance `1 − cos` mathematically spans `[0, 2]`.
- **Inferred:** Mapping `PE_time`, `PE_content`, and `novelty` onto a shared `[0, 1]` unit scale via `clip01(z_time^+ / z_max)`, `clip01(miss)`, and `clip01(1 − cos)` prevents any single channel from dominating `max(·)` and aligns directly with `R-05`'s `clamp(z / z_max, 0, 1)`. Applying pooled Empirical-Bayes shrinkage `(μ₀, σ₀, m₀)` with `w_min` prevents cold-start variance collapse (`σ → 0`) in strict causal online loops where chunk-ahead normalization is unavailable.
- **Open:** Optimal `(γ, w_min, z_max, μ₀, σ₀)` per body type and embedding model (in particular, whether anisotropic/negative-cosine embedders benefit more from adaptive z-normalization `z_nov^+ / z_max` vs direct `clip01(1 − cos)`, and whether adding EM-LLM's lightweight post-hoc merge of sub-`N_ep_min` segments inside sleep consolidation allows `γ = 1.0` to achieve the higher precision that required `γ = 1.5` purely online).

## Sources

- **[PR]** Zacks, J. M., Speer, N. K., Swallow, K. M., Braver, T. S., & Reynolds, J. R. (2007). Event perception: a mind-brain perspective. *Psychological Bulletin*, 133(2), 273–293. DOI: `10.1037/0033-2909.133.2.273` | PMID: `17338600`
- **[PR / PP]** Fountas, Z., Benfeghoul, M. A., Oomerjee, A., Christopoulou, F., Lampouras, G., Bou-Ammar, H., & Wang, J. (2025). Human-inspired Episodic Memory for Infinite Context LLMs. *ICLR 2025* (`https://openreview.net/forum?id=BI2int5SAC`) | `arXiv:2407.09450v3` (`https://arxiv.org/abs/2407.09450`)
- **[IND]** EM-LLM reference implementation and YAML configurations (`https://github.com/em-llm/EM-LLM-model`, `em_llm/attention/em_llm.py`, `em_llm/memory/context_manager.py`, `config/mistral.yaml`, `config/llama31.yaml`, `config/llama3.yaml`, `config/phi3_mini.yaml`)

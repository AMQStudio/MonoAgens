# MonoAgens Research Brief 2 — Part D: Commitments & Becoming a Standard (`R-49`, `R-50`)

**Prepared:** 2026-11-16  
**Scope:** Specification-input research findings beyond `RESEARCH.md` for `R-49` (Promises as commitments) and `R-50` (Becoming a de facto standard), reconciled against `ARCHITECTURE.md`, `RESEARCH.md`, Round 1 (`research/findings/2026-11-15-run-1.md`), `openspec/changes/core-and-lab/`, `bootstrap-vertical-slice`, and sibling Round-2 streams (`A01_A02`, `A10_A11`, `A13` (Part 1), `R29_R31`, `R32_R34_R35`, `R33_R41_R43`, `R44_R45`, `R46_R47`).

---

# R-49 Promises as commitments — conflict, delegation/dependence, refusal, repair, long-run measurement

**Status:** answered

## Answer

**Short answer.** MonoAgens should model `expect(kind = promise)` as a typed social commitment graph $C(x, y, r, u)$ (Castelfranchi 1995; Singh 1999, 2008, 2013) stored inside the existing **`expectations`** book and manipulated using only the existing **5 thought acts** (`think`, `act`, `expect`, `resolve`, `revise`) plus Gateway/Valve policy tokens—adding **zero** new books, zero new acts, and zero semantic knowledge of owners, roles, groups, or platforms in Core. Concretely: (1) extend `expect(kind=promise)` records with opaque relational metadata (`creditor_actor_id`, `beneficiary_audience`, `condition_ref`, `depends_on_expect_id[]`, `delegated_to_actor_id`, `conflict_group_id`, `required_cap_ids[]`, `work_duration_est`, `standing_release_actors[]`, `stakes_tier`, `parent_goal_id`); (2) split **`conditional`** (antecedent $r$ unmet) from **`active`/`due`** and separate mutual **`renegotiated` / `released`** from unilateral **`broken`** inside `resolve`; (3) add a deterministic zero-LLM **`CommitCheck`** surfaced in `ContextPlan` *and* enforced in Step 7 **before** `Effect::Dispatch` executes any promise-bearing utterance, converting empty or conflicting promises into **`refused_before_utterance`** before the counterparty hears them; (4) realize proactive repair and renegotiation exclusively through `act` + `revise` + `resolve`; (5) couple delegated/dependent commitments with `sources` reliability $\text{rel}(a, d)$ and early escalation; and (6) replace the broken `promise_debt` ratio (`n_due_soon / n_open_promises`) with a monotonic, saturation-bounded hazard sum evaluated by a 5-metric long-run Integrity Scorecard.

---

### 1. Gaps & Contradictions Found in Existing Specs (`core-and-lab`, `RESEARCH.md`, `A13` (Part 1))

Audit of current OpenSpec artifacts and Round 2 part-1 notes exposes **seven specification defects** that break long-run commitment integrity:

1. **Missing relational & conditional graph fields (`expectations/spec.md` L11–14, `design.md` L228–233):** Currently stores only singular `place`, `source`, `condition`, `due`, `expiry`, and `created_event`. It lacks `creditor_actor_id` (who holds normative standing to release the promise vs an un-standing bystander; Wei et al. Oct 2026), `condition_ref` (antecedent $r$ for conditional commitments), `depends_on_expect_id` / `delegated_to_actor_id`, `conflict_group_id` (mutex lock token), `required_cap_ids`, and `work_duration_est`.
2. **Incomplete & mismatched lifecycle/`resolve` enum (`design.md` L263 vs `expectations/spec.md` L32–36 vs `A13` (Part 1) L37):** `design.md` L263 restricts `resolve(id, outcome ∈ {kept, failed, cancelled, answered})`, omitting `renegotiated`, `kept_late`, `released`, and `refused_before_utterance`. Worse, neither `expectations/spec.md` L32 nor `A13` (Part 1) (`R-10`) separates:
   - **`conditional`** commitments (antecedent $r$ not yet triggered) from detached **`active`/`due`** commitments;
   - **creditor-consented `released` / `renegotiated`** (zero social breach) from **unilateral `broken`** (debtor default / cancellation without standing).
   Conflating these turns normal scope changes into spurious `promise_debt` spirals or masks unilateral breaches.
3. **Non-monotonic `promise_debt` ratio inversion bug (`expectations/spec.md` L103, `RESEARCH.md` §3):** Defining `promise_debt` as `"the share of the soul's open promises that fall due soon"` ($n_{\text{due\_soon}} / n_{\text{open\_promises}}$) violates basic monotonicity: opening 9 extra low-urgency promises next month *shrinks* `promise_debt` from `1.0` to `0.1` while acute overload actually worsened. Moreover, `conditional` promises and `PendingApproval` (`held`) promises would distort the denominator.
4. **No pre-utterance `CommitCheck` & Same-Turn Dispatch Race (`design.md` Step 7 / `R-10`):** Tang et al. (`arXiv:2610.01045`) show that without capability/runtime feasibility checks at speech time, 8–14B models emit *empty commitments* on 45.9% of follow-up requests when persistence/tools do not exist, and even frontier models make unanchored promises without invoking available scheduling tools. In MonoAgens, if `reason` emits `act(say, "I'll do X tomorrow")` and `expect(kind=promise, ...)` in the same turn, executing `Effect::Dispatch` before validating `expect` leaks empty promises onto the wire.
5. **No standing check on cancellation/release (`arXiv:2610.21203`):** Wei et al. show LLMs drop commitments on a peer's direct request up to 89% (`Qwen3.8-27B`) / 83% (`gpt-6.1-sol`) or override creditor cancellations up to 69% (`Qwen3.5-9B`). Current `expectations/spec.md` allows any actor's message or unqualified `resolve(..., cancelled)` to drop or close a promise with no blind-Core standing check.
6. **Reactive-only repair & risk of inventing a 6th thought act:** `expectations/spec.md` L34–35 and `A13` (Part 1) only trigger renegotiation *after* action retry exhaustion or `held` margin timeout—missing *proactive* triggers (upstream dependency miss, `self_card` capability degradation `degraded_c = 1`, Valve policy revocation, or higher-stakes conflict preemption). Additionally, introducing a dedicated `repair` thought act would violate `ARCHITECTURE.md` L78 & L269–270 (fixed 5 thought acts `{think, act, expect, resolve, revise}`).
7. **Incomplete Lab measurement (`lab/spec.md` L141–149):** Only scores binary `promises kept on time` (plus `kept late` noted in `tasks.md` 10.5), omitting renegotiation repair success, warranted refusal precision vs over-refusal, overcommitment calibration (`ECE_commit`), and sustained backlog (`AUC_debt`).

---

### 2. Literature Grounding & Blind-Core Architecture Mapping

| Literature concept | Verified source | MonoAgens Core realization (blind invariants intact) | Scope (`universal` / `chat-only` / `embodied-only`) |
| :--- | :--- | :--- | :--- |
| Internal vs Social ($S$-) commitment & normative claim | Castelfranchi 1995 (`ICMAS-95`); `R-46` | Internal habits/schedules stay in `RecurrenceSpec` / `kind=goal`; only external pledges to an opaque `creditor_actor_id` use `kind=promise`. | `universal` |
| Conditional $C(x,y,r,u)$ & 6 operations (`Create`, `Discharge`, `Cancel`, `Release`, `Delegate`, `Assign`) | Singh 1999, 2008, 2013; Telang & Singh 2012; Chopra & Singh 2006/2009/2015 | `condition_ref` holds `conditional` until trigger $r$ fires; `resolve` distinguishes creditor `released` / mutual `renegotiated` from debtor `broken`. | `universal` |
| Standing over release vs override | Wei et al. 2026 (`arXiv:2610.21203`) | Gateway/Valve resolves Cedar policy (`R-41`) into opaque `standing_release_actors[]`; Core checks `actor ∈ {creditor_actor_id} ∪ standing_release_actors` without knowing roles/groups. | `universal` (multi-party critical in `chat-only` & shared workspaces) |
| Consistency, means-end coherence, empty commitments, ASA | Bratman 1987; Cohen & Levesque 1990; Tang et al. 2026 (`arXiv:2610.01045`); Chen et al. 2026 (`arXiv:2610.19226`) | Deterministic loop-level `CommitCheck` using `self_card` (`R-35`), persistence affordance, conflict mutex, window capacity, Valve pre-check, and Step-7 pre-dispatch guard. | `universal` (utterance interception applies when promise is communicated via dialogue/signal) |
| Joint delegation, conventions, reliability beliefs | Grosz & Kraus 1996 (`SharedPlans`); Jennings 1993 | `depends_on_expect_id[]` + `delegated_to_actor_id` link child wait/sub-promise deadlines to parent `due` minus slack, weighted by `rel(a, d)` (`sources` book). | `universal` |
| Competence vs integrity breach & early trust repair | Kim et al. 2004; Schweitzer et al. 2006; Robinette et al. 2015; Esterwood & Robert 2023 | Early proactive `renegotiate_proposed` via `act` + `revise` avoids silent failure; unilateral deception/breach penalizes `promise_debt` and creditor trust steeply while honest pre-utterance refusal or early repair avoids integrity penalty. | `universal` (wording via `act` in `chat-only`; audible/LED/handoff alert in `embodied-only`) |
| Loop-driven intention tracking vs open-ended LLM | Liu & Gabriel 2026 (`PM-Bench`); Zhao & Wu 2026 (`PIS`); Bei et al. 2026 (`Pro-Memory`); Wooldridge 2000 | State machine lives deterministically in `monoagens-core` (`expectations/spec.md` L13–14); model only invokes typed acts `{expect, revise, resolve}`. | `universal` |

---

### 3. Mechanism & Math

#### 3.1 Extended `Expectation` Record Schema for `kind = promise` (`expectations` book)

Every `kind = promise` record reuses the `expectations` book (`design.md` L136) and adds optional opaque fields (all defaults `None` / `[]` for non-promise expectations):

$$\text{PromiseMeta}(e) = \langle y,\; B,\; r_{\text{cond}},\; D,\; z_{\text{del}},\; g_{\text{mtx}},\; \mathcal{C}_{\text{req}},\; \hat{w},\; W,\; S_{\text{rel}},\; \rho_e,\; g_{\text{parent}} \rangle$$

where:
- $y = \text{creditor\_actor\_id} \in \text{ActorId}$: opaque actor to whom $u$ is owed (`universal`).
- $B = \text{beneficiary\_audience} \in \text{AudienceId}$: target audience for discharge/repair acts (`universal`).
- $r_{\text{cond}} = \text{condition\_ref} \in \text{Option}\langle\text{ConditionRef}\rangle$: opaque antecedent trigger ID/expression; if `Some(r)`, $e$ starts in **`conditional`** (`universal`).
- $D = \text{depends\_on\_expect\_id} \in \text{Vec}\langle\text{ExpectationId}\rangle$: acyclic dependency edges to prerequisite `wait` / `effect` / `promise` records (`universal`).
- $z_{\text{del}} = \text{delegated\_to\_actor\_id} \in \text{Option}\langle\text{ActorId}\rangle$: downstream delegatee paired with a child `wait`/`response` expectation in $D$ (`universal`).
- $g_{\text{mtx}} = \text{conflict\_group\_id} \in \text{Option}\langle\text{ConflictGroupId}\rangle$: opaque exclusivity token (e.g., hash of exclusive actuator lock in `embodied-only` or exclusive time/booking slot emitted at edge schema in `chat-only`).
- $\mathcal{C}_{\text{req}} = \text{required\_cap\_ids} \in \text{Vec}\langle\text{CapabilityId}\rangle$: sorted capability IDs required to discharge $u$ (`universal`).
- $\hat{w} = \text{work\_duration\_est} \in \mathbb{R}_{\ge 0}$: estimated active execution work-units/seconds needed to discharge $u$ (`universal`; default `w_default` if unannotated).
- $W = [t_{\text{start}}, t_{\text{due}}, t_{\text{expiry}}]$: planned execution start (defaults to `now`), target due timestamp, and expiry (`universal`).
- $S_{\text{rel}} = \text{standing\_release\_actors} \in \text{Vec}\langle\text{ActorId}\rangle$: opaque set of non-creditor actors authorized by Gateway/Valve policy (`R-41` Cedar `permit`) to release/cancel $e$ (`universal`; **Core never inspects policy role names**).
- $\rho_e \in \{0, 1, 2, 3\}$: exogenous stakes tier (`A10_A11` / `R-28`, `universal`).
- $g_{\text{parent}} \in \text{Option}\langle\text{ExpectationId}\rangle$: optional link to parent `kind = goal` (`R-07`, `universal`).

#### 3.2 Lifecycle State Machine & Extended `resolve` Semantics

```
             r fires
[conditional] -------> [active] ──(due reached)──> [due] ──(overdue margin)──> [overdue]
    |                     |  \                       |  \                        |   \
    | (r expires /        |   \ (Step-7 guard        |   \ (`held` receipt,      |    \
    |  creditor release)  |    \ blocks empty/       |    \ R-42 pause)          |     \
    v                     |     v conflict)          |     v                     |      \
[released]                | [refused_before_utterance]   [held_approval]         |       v
                          |                          |         |                 |    [broken]
                          | (proactive blocker /     | (deny /  | (approve ->     |    (unilateral
                          |  cap degraded / dep miss)| timeout) |  resume active) |     cancel /
                          v                          v         v                 |     expiry)
                     [renegotiate_proposed] <--------+---------+-----------------+
                          |            |
            creditor      |            | creditor rejects / repair window expires
            accepts /     v            v
            releases  [renegotiated]  [broken]
                      / [released]
```

**Extended `resolve(id, outcome)` outcomes** (replacing `design.md` L263 `{kept, failed, cancelled, answered}` while preserving exact 5 thought acts):
- For `kind ∈ {effect, response, wait, goal}`: retain `answered`, `kept`, `failed`, `cancelled` (backward-compatible).
- For `kind == promise`, outcomes map cleanly onto integrity semantics:
  - `kept`: $t_{\text{done}} \le t_{\text{due}}$ (`+competence`, `0` breach).
  - `kept_late`: $t_{\text{due}} < t_{\text{done}} \le t_{\text{expiry}}$ (`small competence decay`, `0` intentional integrity breach).
  - `renegotiated`: creditor acknowledged modified terms $(u', t'_{\text{due}})$ via `revise` + linked acknowledgement expectation (`0` breach; linked child promise opened if scope continues).
  - `released`: terminated by `creditor_actor_id` or `actor ∈ S_rel` (`0` breach; prevents spurious `promise_debt` spirals).
  - `refused_before_utterance`: aborted by Step-7 `CommitCheck` prior to external communication (`0` social breach; recorded for `WRP` / `FPR` scorecard).
  - `broken`: unilateral debtor cancellation (`Cancel` without standing), un-repaired missed `t_expiry`, or un-repaired impossible execution (`full integrity/competence penalty` + `promise_debt` breach memory).

**Standing Enforcement Rule (`arXiv:2610.21203`):** Any inbound request or model `resolve(e, released | cancelled)` attributing release to an external actor $a$ is authorized **if and only if**:
$$\text{HasStanding}(a, e) = \mathbb{I}\!\left[a = y_e \;\lor\; a \in S_{\text{rel}}(e)\right]$$
If $\text{HasStanding}(a, e) = 0$: Core **refuses** the release transition, keeps $e$ in `active`/`due`, logs a `verdict(STANDING_DENIED)`, and wakes reasoning only if explanation to $a$ passes Valve. Conversely, once $\text{HasStanding}(a, e) = 1$ transitions $e \to \text{released}$, any subsequent message from $b \notin \{y_e\} \cup S_{\text{rel}}(e)$ attempting to reinstate execution *cannot* resurrect $e$ (requires creating a fresh commitment — eliminating override errors).

#### 3.3 Deterministic Pre-Utterance `CommitCheck` & Step-7 Dispatch Guard

All checks run deterministically in Core/Doors with zero LLM calls, materialized in `ContextPlan` under `CommitCheck` (within `"commitments and the active goal"` of `design.md` L273) and executed synchronously as a pre-dispatch gate in Step 7:

1. **Capability & Persistence Affordance (`R-35` `self_card` + `arXiv:2610.01045`):**
   $$\hat{p}_{\text{cap}}(\mathcal{C}_{\text{req}}, t_{\text{due}}) = \mathbb{I}[\text{persist\_ok}] \cdot \prod_{c \in \mathcal{C}_{\text{req}}} \mathbb{I}\!\left[c \in \mathcal{C}_{\text{active}} \land \neg \text{degraded}_c\right] \cdot \frac{\alpha_c}{\alpha_c + \beta_c}$$
   where $\text{persist\_ok} = \mathbb{I}[t_{\text{due}} \le t_{\text{turn\_end}} \;\lor\; \text{scheduler\_cap} \in \mathcal{C}_{\text{active}}]$. Missing persistence affordance or degraded capability sets $\hat{p}_{\text{cap}} = 0$ (catching "I'll ping you tomorrow" empty commitments).
2. **Valve Authority Pre-Check (`R-41` Cedar PDP cache / dry-run):**
   $$p_{\text{auth}}(\mathcal{C}_{\text{req}}, \text{place}, B) = \begin{cases} 0 & \text{if Valve dry-run returns } \texttt{Deny} \\ 1 & \text{otherwise} \end{cases}$$
   *(Note: dry-run returning `Hold` / `RequireApproval` sets $p_{\text{auth}} = 1$ but forces the `HEDGE_OR_CONDITIONAL` flag so the agent asks or makes approval explicit rather than promising unconditional speed.)*
3. **Window Capacity / Load Check (`load_commitments(W)`):**
   For candidate window $W = [\max(\text{now}, t_{\text{start}}), t_{\text{due}}]$ of duration $|W| = \max(t_{\text{due}} - \max(\text{now}, t_{\text{start}}), \Delta t_{\min})$, define overlapping load across detached commitments $\mathcal{P}_{\text{detached}} = \{e : \text{state}(e) \in \{\text{active}, \text{due}, \text{overdue}\}\}$ (explicitly **excluding** `conditional`, `released`, `renegotiated`, `refused_before_utterance`, and `RecurrenceSpec` routines until detached!):
   $$L(W) = \sum_{e \in \mathcal{P}_{\text{detached}}} \text{rem\_work}(e) \cdot \frac{|W_e \cap W|}{|W_e|}$$
   where $\text{rem\_work}(e) = \hat{w}_e \cdot (1 - \text{prog}_e)$ and $\text{prog}_e \in [0,1]$ (`0` if unstarted, `1` if discharged). Capacity utilization ratio:
   $$\rho_{\text{load}}(W) = \frac{L(W) + \hat{w}_{\text{cand}}}{\kappa_{\text{budget}} \cdot |W|}$$
   where $\kappa_{\text{budget}} \in (0, 1]$ is effective duty/concurrency capacity (`universal`; default `0.70`, reflecting overhead/other wakes). Feasibility indicator: $\text{load\_ok} = \mathbb{I}[\rho_{\text{load}}(W) \le \rho_{\max}]$.
4. **Mutual-Exclusion Conflict Check (`Bratman 1987`):**
   $$\text{conflict}(e_{\text{cand}}) = \exists e \in \mathcal{P}_{\text{detached}} \cup \mathcal{P}_{\text{held}} :\; \left(g_{\text{mtx}}(e) = g_{\text{mtx}}(e_{\text{cand}}) \neq \bot\right) \land \left(W_e \cap W_{\text{cand}} \neq \emptyset\right)$$
   *(If $\rho_{\text{cand}} > \rho_e$, instead of silent breach, triggers candidate preemption-with-repair of $e$ if permitted, rather than holding two contradictory active promises.)*
5. **Dependency & Delegation Slack Check (`Grosz & Kraus 1996` + `R-21`):**
   $$\text{slack\_ok} = \mathbb{I}\!\left[\max_{d \in D} t_d^{\text{due}} + \delta_{\text{slack}} + \hat{w}_{\text{cand}} \le t_{\text{due}}\right]$$
   $$\hat{p}_{\text{dep}}(D, z_{\text{del}}) = \prod_{d \in D} \hat{p}_{\text{on\_time}}(d) \cdot \begin{cases} \frac{\alpha_{z, d_{\text{dom}}} + \alpha_0}{\alpha_{z, d_{\text{dom}}} + \beta_{z, d_{\text{dom}}} + \alpha_0 + \beta_0} & \text{if } z_{\text{del}} = \text{Some}(z) \\ 1 & \text{if } z_{\text{del}} = \text{None} \end{cases}$$
   where $(\alpha_{z, d_{\text{dom}}}, \beta_{z, d_{\text{dom}}})$ is the delegatee's domain reliability tuple in the `sources` book (`RESEARCH.md` §7) with cold-start prior $(\alpha_0, \beta_0) = (2, 1)$ (`start value`, `R-21` consistent).
6. **Calibrated Commitment Feasibility Score (`R-35`):**
   $$p_{\text{feasible}} = p_{\text{auth}} \cdot \mathbb{I}[\neg \text{conflict}] \cdot \mathbb{I}[\text{load\_ok}] \cdot \mathbb{I}[\text{slack\_ok}] \cdot \hat{p}_{\text{cap}} \cdot \hat{p}_{\text{dep}} \cdot p_{\text{cal}}(\text{domain}, c_{\text{model}})$$
   Decision tiers:
   - **$p_{\text{feasible}} < \theta_{\text{commit\_min}}$** (`MUST_REFUSE`): Step 7 blocks promise dispatch, sets provisional `expect` -> `refused_before_utterance`, logs `door_verdict(PROMISE_INFEASIBLE, reason_code)`, and triggers at most $1$ bounded re-wake (`CommitRefusal`) so reasoning offers an honest explanation/refusal (`chat-only`) or abstains (`embodied-only` local fail-safe).
   - **$\theta_{\text{commit\_min}} \le p_{\text{feasible}} < \theta_{\text{commit\_safe}}$** (`CONDITIONAL_ONLY`): Allows creating $e$ **only** with `condition_ref = Some(r)` (conditional $C(x,y,r,u)$) or extended $t'_{\text{due}}$, never an unconditional promise.
   - **$p_{\text{feasible}} \ge \theta_{\text{commit\_safe}}$** (`CLEAR`): Permitted to open `active` promise.

**Critical Step-7 Atomic Ordering Rule (Fix for Same-Turn Empty Promise Leak):**
Inside `Core::step` Step 7 (`design.md` / `R-10`), candidate acts are split into **non-communicative capability calls** vs **outbound utterance/signaling acts** (`act` carrying promise linkage `for_expect_ids` OR co-occurring in the same thought chain as `expect(kind = promise)`):
1. First evaluate `expect(kind = promise)` candidates through `CommitCheck`.
2. If any candidate evaluates `MUST_REFUSE`, **suppress co-emitted outbound promise utterances in the same Step 7 batch BEFORE emitting `Effect::Dispatch`**, record `refused_before_utterance`, and run the single bounded recovery wake (`max_refusal_rewakes = 1`) with deterministic failure reasons injected into `ContextPlan`. *(Remaining non-promising diagnostic/read-only `act` calls without commitment speech are unaffected; if `reason` still tries to promise on wake 2, Doors falls back to a templated structural inability signal at the adapter/Gateway edge without emitting ungrounded text.)*

#### 3.4 Proactive Repair & Renegotiation Protocol (5-Act Compliant)

Rather than waiting passively until post-expiry or retry exhaustion (`expectations/spec.md` L32–36), **four proactive loop triggers** transition an `active`/`due` promise toward `renegotiate_proposed`:
1. **Upstream dependency failure/slip:** Any $d \in D$ transitions to `failed`/`broken`/`overdue` or remaining slack drops below $\text{rem\_work}(e) + \delta_{\text{slack}}$.
2. **Self-model capability degradation (`R-35`):** Some $c \in \mathcal{C}_{\text{req}}$ switches to `degraded_c = true`.
3. **Policy revocation (`R-41`):** Re-evaluating Valve returns `Deny` for a queued promise.
4. **Overload/preemption shock:** Newly arriving high-stakes external deadline ($\rho_{\text{new}} > \rho_e$) pushes $\rho_{\text{load}}(W) > \rho_{\max}$ or triggers `conflict_group_id` preemption.

**Execution purely via `{act, revise, resolve}` (Zero 6th Primitive):**
1. Loop wakes reasoning with `RepairTrigger(e_id, code ∈ {DEP_FAIL, CAP_DOWN, AUTH_REVOKED, CAPACITY_SHOCK, RETRY_EXHAUSTED}, now >= t_due - margin_inform)`.
2. Reasoning emits two linked existing acts:
   - `act(cap=respond/notify, place=place_e, audience=B_e, args={type: "renegotiation_offer", expect_id: e_id, reason_code, proposed_due: t'_due, offer_release: bool}, expected=wait_creditor_reply)`
   - `revise(target_id=e_id, state="renegotiate_proposed", proposed_due=t'_due, repair_wait_id=wait_creditor_reply)`
3. **Resolution paths:**
   - Creditor accepts (`wait_creditor_reply` resolves `answered` with `accept`): loop transitions `e_id` -> `resolve(e_id, renegotiated)` and opens a fresh `active` child promise at $t'_{\text{due}}$ (`0` breach!).
   - Creditor grants release (`HasStanding == 1`): transitions `e_id` -> `resolve(e_id, released)` (`0` breach!).
   - Creditor declines OR `wait_creditor_reply` expires past $t_{\text{expiry}}$ without fulfillment: transitions `e_id` -> `resolve(e_id, broken)` (`breach` recorded, `rel`/trust penalty applied).
   - **Pre-breach timing incentive (`Robinette et al. 2015`):** Renegotiations initiated at $t_{\text{repair}} \le t_{\text{due}} - \text{margin\_inform}$ accrue **zero** `promise_debt` integrity penalty upon acceptance; post-expiry repair offers still log `kept_late` / partial breach even if accepted so timing games cannot mask chronic lateness.

#### 3.5 Corrected Monotonic Bounded `promise_debt(t)`

Replace `n_due_soon / n_open_promises` with a smooth, strictly monotonic saturation function over detached active/due, overdue, and recent unilateral breaches:

$$D_{\text{raw}}(t) = \sum_{e \in \mathcal{P}_{\text{detached}}} w(\rho_e)\,\exp\!\left(-\frac{\max(0,\, t_e^{\text{due}} - t)}{\tau_{\text{soon}}}\right) + \lambda_{\text{over}} \sum_{e \in \mathcal{P}_{\text{overdue}}} w(\rho_e) + \lambda_{\text{breach}} \sum_{b \in \mathcal{B}_{\text{unilat}}} w(\rho_b)\,\exp\!\left(-\frac{t - t_b}{\tau_{\text{debt}}}\right)$$

$$\text{promise\_debt}(t) = \frac{D_{\text{raw}}(t)}{D_{\text{raw}}(t) + \kappa_{\text{debt}}} \in [0, 1)$$

where $w(\rho) = 1 + 0.5\rho$ ($\rho \in \{0,1,2,3\}$). **Properties:** (a) exact neutral `0.0` when $\mathcal{P}_{\text{detached}} = \mathcal{P}_{\text{overdue}} = \mathcal{B}_{\text{unilat}} = \emptyset$; (b) `conditional`, `released`, `renegotiated`, `refused_before_utterance`, and `RecurrenceSpec` routines contribute **zero**; (c) adding promises strictly increases $D_{\text{raw}}(t)$ (monotonic); (d) bounded in $[0, 1)$ for Gate normalization.

#### 3.6 Long-Run Integrity Scorecard (5 Observable Lab Metrics)

Over simulated run horizon $[0, T]$, partitioning mature detached promises ($r$ satisfied/unconditional) into `K` (`kept`), `K_L` (`kept_late`), `R_debtor` (`renegotiated` initiated by debtor), `R_cred` (`released` initiated unconditionally by creditor without debtor fault signal), `B` (`broken`):

1. **Promise Keeping Rate (`PKR`):**
   $$\text{PKR}_{\text{strict}} = \frac{|K|}{|K| + |K_L| + |R_{\text{debtor}}| + |B|}, \qquad \text{PKR}_{\text{net}} = \frac{|K| + w_{\text{late}}|K_L|}{|K| + |K_L| + |B|}$$
   (Tracking `PKR_strict` alongside `PKR_net` prevents agents from artificially padding `PKR_net` by incessantly renegotiating!)
2. **Renegotiation-with-Repair Rate (`RRR`):**
   $$\text{RRR} = \frac{\#\{\text{shocked/infeasible promises proactively moved to creditor-accepted }\texttt{renegotiated}/\texttt{released}\text{ before } t_{\text{due}}\}}{\#\{\text{active promises hitting verified exogenous blocker/shock}\}}$$
3. **Warranted Refusal Precision (`WRP`) & False-Promise Rate (`FPR`):**
   $$\text{FPR} = \frac{\#\{\text{infeasible/conflicting requests emitted as }\texttt{active}\text{ promise}\}}{\#\{\text{paired impossible/blocked/conflicting probes}\}}, \qquad \text{WRP} = \frac{\text{TP}_{\text{refuse}}}{\text{TP}_{\text{refuse}} + \text{FP}_{\text{over\_refuse}}}$$
4. **Calibrated Overcommitment ECE (`ECE_commit`):**
   Partition $N$ promises into $M=10$ confidence bins $B_m$ over predicted $p_{\text{feasible}}$ vs binary on-time outcome $y_i = \mathbb{I}[i \in K]$:
   $$\text{ECE}_{\text{commit}} = \sum_{m=1}^{M} \frac{|B_m|}{N} \left|\,\text{acc}(B_m) - \text{conf}(B_m)\,\right|$$
5. **Promise Debt AUC (`AUC_debt`):**
   $$\text{AUC}_{\text{debt}} = \frac{1}{T} \int_0^T \text{promise\_debt}(t)\,dt$$

---

## Parameters

*(Note: Behavioral/cognitive thresholds below are **not settled by the literature**; start values and search ranges are defined explicitly for `MonoAgens Lab` calibration.)*

| Name | Meaning | Start value | Search range | Source / Status |
| :--- | :--- | :--- | :--- | :--- |
| $\theta_{\text{commit\_min}}$ | Minimum calibrated feasibility to allow even a conditional/hedged promise; below this, mandatory refusal (`CommitCheck`) | `0.60` | `[0.45, 0.80]` | Not settled by literature; tune in Lab (`FPR` vs over-refusal Pareto) |
| $\theta_{\text{commit\_safe}}$ | Minimum feasibility for an unconditional detached `active` promise | `0.80` | `[0.70, 0.90]` | Not settled by literature; tune in Lab |
| $\rho_{\max}$ | Maximum allowed window capacity utilization $\rho_{\text{load}}(W)$ before overload refusal | `0.85` | `[0.65, 0.95]` | Not settled by literature; tune in Lab |
| $\kappa_{\text{budget}}$ | Effective execution duty factor per unit wall time | `0.70` | `[0.40, 0.90]` | Not settled by literature; tune in Lab per body class |
| $\delta_{\text{slack}}$ | Mandatory upstream dependency margin prior to parent $t_{\text{due}}$ | `600 s` (`10 min`) | `[60 s, 3600 s]` | Not settled by literature; scale by domain horizon in Lab |
| $(\alpha_0, \beta_0)$ | Delegatee cold-start reliability Beta prior (`sources` book) | `(2.0, 1.0)` | `([1..4], [1..3])` | `RESEARCH.md` §7 / `R-21` coherent |
| $\tau_{\text{soon}}$ | Look-ahead decay constant for upcoming detached promises in `promise_debt` | `3600 s` (`1 h`) | `[900 s, 86400 s]` | Not settled by literature; tune in Lab |
| $\tau_{\text{debt}}$ | Half-life decay for recent unilateral breaches in `promise_debt` | `604800 s` (`7 d`) | `[86400 s, 2592000 s]` | Not settled by literature; tune in Lab |
| $(\lambda_{\text{over}}, \lambda_{\text{breach}}, \kappa_{\text{debt}})$ | Overdue weight, unilateral breach weight, and saturation scale in `promise_debt` | `(1.5, 2.0, 3.0)` | `([1.0..3.0], [1.0..4.0], [1.5..6.0])` | Not settled by literature; tune in Lab |
| $w_{\text{late}}$ | Partial credit weight for `kept_late` in `PKR_net` | `0.50` | `[0.25, 0.75]` | Not settled by literature; tune in Lab |
| `max_refusal_rewakes` | Bound on Step-7 `CommitRefusal` recovery re-wakes per trigger | `1` | `[1, 1]` | Hard safety bound against loops (`A13` (Part 1) aligned) |
| `standing_strict` | Require exact creditor/policy standing before `released` transition | `true` | `{true, false}` (ablation) | Settled directionally by `arXiv:2610.21203` |

---

## Neutral value

- **`CommitCheck` neutral (disabled):** Always returns `CLEAR` ($p_{\text{feasible}} = 1.0$, capacity/dependency/conflict/affordance checks bypassed); `resolve` keeps legacy 4-value set `{kept, failed, cancelled, answered}`; Step-7 dispatches utterances without pre-checking `expect`.
- **`promise_debt` neutral:** `0.0` (exact neutral when no active/due/overdue/unilateral-breach records exist).
- **`standing_strict` neutral (disabled):** Any cancellation/override percept is accepted uniformly as `cancelled` regardless of `creditor_actor_id` / `standing_release_actors`.

---

## Claim

1. Enabling pre-utterance `CommitCheck` + Step-7 ordering guard reduces **Empty / False-Promise Rate (`FPR`)** on infeasible, un-persisted, policy-denied, and conflicting requests by **$\ge 70\%$ relative** compared to neutral (benchmark reference: `arXiv:2610.01045` reports capability card + in-loop deterministic checker cutting 45.9% open-weight / 4.4% frontier empty commitments) while maintaining feasible-task completion within **$\le 3\%$ loss** (high `WRP`, avoiding `arXiv:2610.19226` over-refusal).
2. Enforcing `HasStanding(a, e)` in `expectations` reduces **unauthorized release and creditor override errors** together by **$\ge 75\%$ relative** over stateless/role-blind cancellation (`arXiv:2610.21203`: self-kept ledger + standing policy raised strict accuracy from 49% to 83%).
3. Separating `conditional` and creditor-accepted `renegotiated`/`released` from unilateral `broken`, paired with proactive repair triggers (`act + revise`), increases **Renegotiation-with-Repair Rate (`RRR`)** and cuts long-run **`AUC_debt`** during exogenous capability/dependency shocks compared to reactive retry-until-expiry (`A13` (Part 1) baseline) **without adding any 6th thought act**.
4. Replacing `n_due_soon / n_open_promises` with monotonic bounded `promise_debt(t)` eliminates 100% of synthetic denominator-dilution inversions on Lab monotonicity unit sweeps.

---

## How the Lab would test it

Extend `monoagens-lab` (`openspec/changes/core-and-lab/specs/lab/spec.md`) with **six deterministic discrete-event Lab suites** (virtual time, seeded RNG, synthetic scripted counterparts/evaluators — zero LLM calls required for Core checks, optional replay harness for full-stack model checks):

1. **Empty-Commitment & Conflict Suite (inspired by `arXiv:2610.01045` & `arXiv:2610.19226`):** Paired feasible vs blocked/impossible scenarios across `chat-only` (scheduler capability absent, Valve `Deny`, double-booked slot `g_mtx`) and `embodied-only` (gripper actuator degraded `degraded_c = true`, battery window `< w_est`). Measure `FPR`, `WRP`, and feasible-task completion (`neutral` vs `CommitCheck`).
2. **Same-Turn Dispatch Race Regression Test:** Force scripted model output emitting `[act(say, promise_text), expect(kind=promise, missing_cap)]` simultaneously; assert `Effect::Dispatch` for `say` count == `0` prior to `refused_before_utterance` recovery wake (`ARCHITECTURE.md` zero-leak guarantee).
3. **Multi-Party Standing Matrix (`arXiv:2610.21203`):** Deterministic matrix crossing `{creditor, authorized_delegate_via_Valve, unauthorized_peer, bystander}` $\times$ `{cancel request, late override after release}` $\times$ `{chat, shared workspace}`. Measure unauthorized release rate + override rate while verifying Core inspects **zero** string role labels.
4. **Exogenous Shock & Proactive Repair Suite (`Jennings 1993` + `Robinette 2015`):** Inject deterministic upstream child `wait` timeout ($z_{\text{del}}$ misses deadline), mid-window `degraded_c = true`, and higher-stakes preemption during active schedules. Compare neutral (`A13` (Part 1) reactive post-expiry retry) vs proactive repair (`RRR`, `PKR_strict`, `PKR_net`, `AUC_debt`).
5. **Monotonicity & Non-Spurious Debt Property Test:** Fuzz-generate open schedules adding (a) conditional $C(x,y,r,u)$ promises, (b) `RecurrenceSpec` routines (`R-46`), (c) mutual `released`/`renegotiated` closures, and (d) far-future active promises. Assert $\Delta \text{promise\_debt}(t) \ge 0$ whenever detached load increases, and $\Delta \text{promise\_debt}(t) = 0$ on (a)–(c).
6. **Multi-Week Calibration & Joint-Dependency Stress Run (`28 virtual days`):** Run full Scorecard `(PKR_strict, PKR_net, RRR, FPR, WRP, ECE_commit, AUC_debt)` across multiple delegatee reliabilities $\text{rel}(a, d) \in \{0.3, 0.7, 0.95\}$ to verify Bayesian adaptation avoids repeatedly overpromising behind unreliable partners (`Grosz & Kraus 1996`).

---

## Established / inferred / open

- **Established:**
  - Formal commitment semantics $C(x,y,r,u)$, explicit distinction between debtor `Cancel` vs creditor `Release` vs `Delegate`, and the need for observable commitment state convergence rather than unverifiable private mental states are firmly established (`Castelfranchi 1995`, `Singh 1999/2008/2013`, `Chopra & Singh 2006/2009/2015`, `Wooldridge 2000`, `Cohen & Levesque 1990`, `Grosz & Kraus 1996`, `Jennings 1993`).
  - Competence vs integrity violation asymmetry, non-recoverability after broken/deceptive promises, and criticality of *early* proactive repair are empirically established (`Kim et al. 2004`, `Schweitzer et al. 2006`, `Robinette et al. 2015`, `Esterwood & Robert 2023`).
  - In LLM agents (`[PP]` 2026), deterministic code-side intention stores outperform LLM-only tracking (`65.1% -> 82.9%` Set-F1 on PM-Bench, `arXiv:2609.01272`), capability-card + pre-output rule checks curb empty commitments (`arXiv:2610.01045`), and self-kept commitment ledgers + release standing rules dramatically curb unauthorized cancellation/override (`49% -> 83%`, `arXiv:2610.21203`).
- **Inferred:**
  - Mapping Valve/Cedar standing resolution into opaque `standing_release_actors[]` IDs satisfies both multi-party commitment standing (`arXiv:2610.21203`) and `ARCHITECTURE.md`'s role-blind Core constraint.
  - Realizing repair via `{act, revise, resolve}` keeps all 5 thought acts intact without loss of protocol expressiveness.
  - Replacing the `promise_debt` ratio with the monotonic saturation sum avoids denominator inversion and aligns with `R-46` routine separation.
  - Same-turn Step-7 `CommitCheck` ordering prevents empty utterance emission on single-turn promise generation.
- **Open (not settled by the literature; calibrate in Lab):**
  - Exact optimal thresholds $(\theta_{\text{commit\_min}}, \theta_{\text{commit\_safe}}, \rho_{\max}, \kappa_{\text{budget}}, \delta_{\text{slack}})$ trading off `FPR` against `WRP` / feasible completion rate across diverse workloads.
  - Relative human/counterparty weightings $(\lambda_{\text{over}}, \lambda_{\text{breach}}, \tau_{\text{soon}}, \tau_{\text{debt}}, w_{\text{late}})$ balancing strict timeliness vs accepted renegotiations in long-run trust dynamics.
  - Automated edge extraction/normalization of `conflict_group_id` and `work_duration_est` across heterogeneous plugins without pushing cognitive logic into adapters.

---

## Sources

- `[PR]` Castelfranchi, C. (1995). *Commitments: From Individual Intentions to Groups and Organizations*. ICMAS-95, AAAI Press, pp. 41–48. [PDF](https://cdn.aaai.org/ICMAS/1995/ICMAS95-006.pdf)
- `[PR]` Singh, M. P. (1999). *An Ontology for Commitments in Multiagent Systems: Toward a Unification of Normative Concepts*. Artificial Intelligence and Law, 7(1):97–113. DOI: `10.1023/A:1008319631231`
- `[PR]` Singh, M. P. (1998). *Agent Communication Languages: Rethinking the Principles*. IEEE Computer, 31(12):40–47. DOI: `10.1109/2.735849`
- `[PR]` Singh, M. P. (2008). *Semantical Considerations on Dialectical and Practical Commitments*. AAAI-08, pp. 176–181. [PDF](https://cdn.aaai.org/AAAI/2008/AAAI08-028.pdf)
- `[PR]` Singh, M. P. (2013). *Norms as a Basis for Governing Sociotechnical Systems*. ACM TIST, 5(1):1–23. DOI: `10.1145/2542182.2542203`
- `[PR]` Telang, P. R., & Singh, M. P. (2012). *Specifying and Verifying Cross-Organizational Business Models: An Agent-Oriented Approach*. IEEE Transactions on Services Computing, 5(3):305–318. DOI: `10.1109/TSC.2011.4`
- `[PR]` Chopra, A. K., & Singh, M. P. (2006 / 2009 / 2015). *Contextualizing Commitment Protocol* (AAMAS '06, DOI: `10.1145/1160633.1160884`); *Multiagent Commitment Alignment* (AAMAS '09, DOI: `10.65109/hehh2954`); *Generalized Commitment Alignment* (AAMAS '15, DOI: `10.65109/ibva5046`).
- `[PR]` Cohen, P. R., & Levesque, H. J. (1990). *Intention Is Choice with Commitment*. Artificial Intelligence, 42(2–3):213–261. DOI: `10.1016/0004-3702(90)90055-5`
- `[PR]` Grosz, B. J., & Kraus, S. (1996). *Collaborative Plans for Complex Group Action*. Artificial Intelligence, 86(2):269–357. DOI: `10.1016/0004-3702(95)00103-4`
- `[PR]` Jennings, N. R. (1993). *Commitments and Conventions: The Foundation of Coordination in Multi-Agent Systems*. The Knowledge Engineering Review, 8(3):223–250. DOI: `10.1017/S0269888900000205`
- `[PR]` Bratman, M. E. (1987). *Intention, Plans, and Practical Reason*. Harvard University Press, ISBN `978-0-674-45818-5`.
- `[PR]` Wooldridge, M. (2000). *Semantic Issues in the Verification of Agent Communication Languages*. Autonomous Agents and Multi-Agent Systems, 3:9–31. DOI: `10.1023/a:1010090027213`
- `[PR]` Horvitz, E. (1999). *Principles of Mixed-Initiative User Interfaces*. CHI '99, pp. 159–166. DOI: `10.1145/302979.303030`
- `[PR]` Kim, P. H., Ferrin, D. L., Cooper, C. D., & Dirks, K. T. (2004). *Removing the Shadow of Suspicion: The Effects of Apology Versus Denial for Repairing Competence- Versus Integrity-Based Trust Violations*. Journal of Applied Psychology, 89(1):104–118. DOI: `10.1037/0021-9010.89.1.104`
- `[PR]` Schweitzer, M. E., Hershey, J. C., & Bradlow, E. T. (2006). *Promises and Lies: Restoring Violated Trust*. OBHDP, 101(1):1–19. DOI: `10.1016/j.obhdp.2006.05.005`
- `[PR]` Robinette, P., Howard, A. M., & Wagner, A. R. (2015). *Timing Is Key for Robot Trust Repair*. ICSR / LNCS, pp. 574–583. DOI: `10.1007/978-3-319-25554-5_57`
- `[PR]` Esterwood, C., & Robert, L. P. (2023). *Three Strikes and You Are Out!: The Impacts of Multiple Human–Robot Trust Violations and Repairs on Robot Trustworthiness*. Computers in Human Behavior, 142:107658. DOI: `10.1016/j.chb.2023.107658`
- `[PP]` Wei, L. et al. (Oct 2026). *Who May Release an Agent's Promise? Standing over Commitments in Multi-Party Chat*. `arXiv:2610.21203` (DOI: `10.48550/arXiv.2610.21203`).
- `[PP]` Tang, J. et al. (Oct 2026). *Empty Commitments: When Agents Promise What They Cannot Deliver*. `arXiv:2610.01045` (DOI: `10.48550/arXiv.2610.01045`).
- `[PP]` Chen, Q., Hayashi, H., Zhang, J., & Wu, C.-S. (Oct 2026). *Agentic Self-Awareness: Aligning Knowing, Acting, and Holding*. `arXiv:2610.19226` (DOI: `10.48550/arXiv.2610.19226`).
- `[PP]` Liu, G., & Gabriel, S. (Jul 2026). *PM-Bench: Evaluating Prospective Memory in LLM Agents*. `arXiv:2607.12385`.
- `[PP]` Zhao, J., & Wu, C. (Sep 2026). *Making Prospective Memory SLM-Shaped: Typed Intention Stores for Small-Model Agents*. `arXiv:2609.01272`.
- `[PP]` Bei, Y. et al. (Nov 2026). *Prospective Memory for Proactive LLM Agents*. `arXiv:2611.05731`.
- `[PP]` Zhang, E. et al. (May 2026). *TERMS-Bench: Diagnosing LLM Negotiation Agents Beyond Deal Rate*. `arXiv:2605.13909` (DOI: `10.48550/arXiv.2605.13909`).
- `[PP]` Zhang, T. et al. (Oct 2026). *OverAct: Measuring and Mitigating Proactive Over-Authorization in LLM Tool-Calling Agents*. `arXiv:2610.01508` (DOI: `10.48550/arXiv.2610.01508`).
- `[PP]` Ren, J. et al. (Apr 2026). *Beyond "I Don't Know": Evaluating LLM Self-Awareness in Discriminating Data and Model Uncertainty*. `arXiv:2604.17293` (DOI: `10.48550/arXiv.2604.17293`).
- `[PP]` Li, Y. et al. (Jun 2026). *From Knowing to Acting: Benchmarking Self-Awareness Capability of LLM Agents*. `arXiv:2606.20661` (DOI: `10.48550/arXiv.2606.20661`).

---
---

# R-50 Becoming a de facto standard — protocol, conformance tests, ecosystem, order of work

**Status:** answered

## Answer

**Short answer.** De facto protocols win by freezing a **tiny, machine-readable, stateless wire boundary** paired with an **executable black-box TCK** and a **working reference host**, while treating rich internal implementation logic as swappable rather than mandatory (`OCI`, `LSP`, `A2A v1.0.1`, `MCP` minus its pre-2026 session misstep, `SQLite`, `OTel`). They fail when committees over-specify heavyweight internal semantics without usable conformance harnesses or hosts (`FIPA ACL`, `CORBA`, `SOAP/WS-*`). Applied to MonoAgens, **do not require external body authors or alternative engines to adopt MonoAgens's 40 cognitive mechanisms**: (1) freeze and formally schema-type **Waist 1 (`Core ↔ Gateway`: 6 domain facts + sans-IO `Core::step`)** and **Waist 2 (`Gateway ↔ Plugins`: `monoagens.body.v1` OpenRPC 1.4.1 / JSON Schema 2020-12)** with strict RFC 2119 / RFC 8174 error codes and explicit `protocol/spec.md` envelope separation; (2) turn MonoAgens Lab into a **dual-purpose platform** (`Lab::Cognitive` for internal research + public black-box **`monoagens-conformance` TCK** split into `MUST`/`SHOULD`/`MAY` across Body Waist 2 and Core Waist 1); (3) unpark **`bootstrap-vertical-slice`** early so **`monoagens-gateway` + `body-cli` + `body-mcp-bridge` + polyglot thin SDKs ($\le 200$ LOC per body)** ship alongside Stage A of `core-and-lab` instead of waiting for all 22 brain-map rows behind `TestBody`.

---

### 1. Comparative Evidence: Real-World Standardization Wins vs Failures (Verified through Nov 2026)

| System / Protocol | What made it win or fail (directly observed facts as of Nov 2026) | Direct lesson for MonoAgens |
| :--- | :--- | :--- |
| **OCI (`runtime-spec v1.3.0` 2025-11-04; `image-spec v1.1.1` 2025-03-03)** | Tiny execution boundary: just **5 lifecycle operations** (`state`, `create`, `start`, `kill`, `delete`) + `config.json` bundle; shipped immediately alongside **`runc`** (`1.6.0-rc.2+dev` tip) and **`runtime-tools`** (`oci-runtime-tool validate` + `node-tap` runtime conformance suite). Separated execution substrate from orchestration. | Lock Waist 1/Waist 2 as tiny lifecycle + envelope contracts; build a real working host early and validate every adapter mechanically. |
| **LSP (Microsoft + Red Hat + Codenvy, 2016; `v3.17` / `v3.18`)** | Decoupled $M$ editors $\times$ $N$ language engines into $M + N$ over JSON-RPC 2.0; anchored adoption on one killer host (**VS Code**); added formal machine-readable **`metaModel.json`** (`3.17`: 67 requests, 26 notifications, 324 structs, 37 enums; `3.18`: 69 requests, 26 notifications) driving polyglot SDK codegen. | MonoAgens's hourglass (`ARCHITECTURE.md` L204–210) is the cognitive analogue of LSP: decouple $M$ cores $\times$ $N$ bodies by generating Rust/TS/Python SDKs from one machine-readable JSON metamodel. |
| **SQLite (`TH3`, `SLT`, `dbsqlfuzz`)** | Public-domain tiny embeddable zero-dep C core (`155.8 KSLOC`) backed by **`590x` more test code (`92,053.1 KSLOC`)**: `TH3` achieves **100% branch + 100% MC/DC** using *only published public interfaces*, `SLT` differential tests against Postgres/MySQL/MSSQL/Oracle, `dbsqlfuzz` runs ~1B mutations/day. Spec-by-reference + conformance. | Keep `monoagens-core` embeddable sans-IO (`design.md` D3) and let public-interface conformance tests—especially deterministic replay & invariance checks—anchor trust. |
| **MCP (`2024-11-05` $\to$ `2026-07-28`; LF AAIF Dec 9, 2025)** | **Win driver:** Tiny v1 server surface + killer host (Claude Desktop) + official multi-language SDKs (e.g., Rust `rmcp 3.10.0`) + interactive **`Inspector`** + neutral LF **AAIF** governance (`aaif.io`, Dec 9, 2025). **Critical growing pain:** Baking stateful `Mcp-Session-Id` connections, `initialize` handshakes, `ping`, and server-initiated RPCs (`roots/list`, `sampling/createMessage`) into `2024-11-05` broke horizontal scaling—forcing **breaking surgery in `2026-07-28`** (`SEP-2567`, `SEP-2575`, `SEP-2322`: stateless requests with `_meta`, mandatory `server/discover`, explicit handles, Multi Round-Trip `InputRequiredResult`). | Keep Waist 2 **stateless / explicit-handle** (`call_id` / `approval_id`) on Day 1 instead of implicit per-socket session state, and ship a MonoAgens Inspector + `body-mcp-bridge` (`R-45`). |
| **A2A (`v1.0.0` Mar 12, 2026; `v1.0.1` May 26, 2026; `a2a-tck`)** | Absorbed IBM ACP (Aug 2025) under Linux Foundation rather than splitting ecosystems; core contract in `a2a.proto` spans just **11 RPCs** across 3 transports (gRPC, JSON-RPC, HTTP+JSON); ships **`a2aproject/a2a-tck`** structured by RFC 2119 **`--level must\|should\|may`** (`tests/mandatory/` hard fail vs `tests/optional/` `xfail`/skip) outputting machine-readable `compatibility.json`. | Model **`monoagens-conformance`** directly after `a2a-tck`: map every normative `MUST` / `SHOULD` / `MAY` rule in `protocol/spec.md` & `R-45` to black-box TCK tests emitting JSON scorecards. |
| **OpenTelemetry + Weaver (`v0.28.0`, Oct 2026)** | Strict split between lightweight **API**, replaceable **SDK**, out-of-process **Collector**, and schema-governed semantic conventions checked offline (`weaver registry check`) and live on wire (`weaver registry live-check`). | Separate minimal Core API/facts from replaceable Core implementation; validate live Waist-2 OTLP/JSON-RPC frames with schema-driven checks. |
| **HTTP/1.1 & RFC 9110 (STD 97; Fielding & Taylor 2002 TOIT)** | Decades of durability via stateless verbs, BCP 14 testability (`RFC 2119` + `RFC 8174`), modular suites (`httpwg/structured-field-tests`, `h2spec`, `REDBot`), and falsifiable rule extraction (`Rautenstrauch & Stock 2024`, ASIACCS: 106 spec rules mined from RFCs exposed real-world security/conformance bugs). Note: no monolithic official `httpwg` RFC 9110 TCK exists. | Every normative sentence in Waist specs MUST be falsifiable (`MUST` $\to$ 1 automated TCK test ID), preventing silent divergence across third-party bodies. |
| **Cautionary Failure #1: FIPA ACL (`SC00037` 22 performatives; Poslad 2007 TAAS; Wooldridge 2000)** | Over-specified mentalistic speech-act preconditions/BDI semantics (`FIPA-SL`), ~22 performatives, CORBA/IIOP weight, weak conformance tooling, impossible black-box verification (`Wooldridge 2000`) $\to$ trapped in niche Java/academic frameworks (`JADE`). | **Do NOT standardize MonoAgens's 40 internal brain mechanisms** (emotions/drives/sleep/credences) as part of external conformance; standardize ONLY observable black-box behavior & Waist schemas! |
| **Cautionary Failure #2: CORBA (Henning 2006) & `SOAP/WS-*` (Pautasso et al. 2008)** | Multi-vendor committee sprawl before running code, excessive spec surface (`WS-*`), tight coupling, steep developer learning curve $\to$ displaced by REST/JSON (`Fielding & Taylor 2002`). | Freeze Waist 2 under `< 15` RPC methods with JSON Schema 2020-12 over JSON-RPC 2.0 (`RFC 8259`), rejecting enterprise middleware bloat. |
| **Cautionary Failure #3: Pre-MCP Fragmented Agent/Tool Schemas (2023–2025)** | Framework-local tool wrappers (`LangChain StructuredTool` vs bespoke provider dialects; `agent-protocol` arrived as an external OpenAPI thread/run wrapper without TCK adoption) forced $M \times N$ glue until decoupled `MCP`/`A2A` protocols arrived. | Ship canonical Waist-2 adapters + `body-mcp-bridge` **before** building custom per-platform hacks. |

---

### 2. Six Concrete Rules Mapped Directly onto MonoAgens

#### Rule 1 — Freeze Two Tiny Machine-Readable Waists & Decouple "Protocol Standard" from "MonoAgens Mind"
Define **two clean compatibility tiers** so ecosystems never face FIPA-style mental-state lock-in:
- **Normative Protocol Standard (`monoagens-spec`, Apache-2.0 / MIT):**
  1. **Waist 1 (`monoagens.core.v1`):** Sans-IO `Core::step(input, clock, rng) -> StepOutput` trait (`design.md` D3), **6 domain facts** (`Actor`, `Place`, `Percept`, `Capability`, `Action`, `Receipt` with `{done, refused, held, failed, unknown}`), `ContextPlan` door boundary, and `Delta_NI` / `hard_forget` / determinism black-box contracts.
  2. **Waist 2 (`monoagens.body.v1`, extending `R-45`):** Stateless/explicit-handle JSON-RPC 2.0 (`<= 12` methods, e.g., `body/discover` [like MCP `server/discover`], `body/heartbeat`, `body/shutdown`, `percept/emit`, `action/execute`, `action/cancel`, `action/receipt`, `approval/resolve`, `question/answer`, `state/query`, `blob/put`, `blob/get`), carrying explicit `idempotency_key`, `protocol_version`, `extensions`, and `traceparent` metadata per request (`MCP 2026-07-28` lesson: **no implicit server-side session state!**).
  3. **Canonical Machine-Readable Artifacts:** Single-source **`OpenRPC 1.4.1`** + **`JSON Schema Draft 2020-12`** document exported from Rust structs (`schemars 1.2.3`, `serde_json 1.0.154`, `jsonrpsee 0.26.1`), enforcing strict **RFC 2119 / RFC 8174** keywords and stable numeric/string error codes (`-32001 ERR_UNKNOWN_CAPABILITY`, `-32002 ERR_DUPLICATE_IDEMPOTENCY`, `-32003 ERR_INVALID_RECEIPT_TRANSITION`, `-32004 ERR_UNSUPPORTED_VERSION`, `-32005 ERR_HASH_MISMATCH`, `-32006 ERR_UNDECLARED_AUDIENCE`).
  4. **Explicit Fix for `protocol/spec.md` L12–15 Contradiction:** Amend `protocol/spec.md` L12–15 (`"define exactly six facts... Every message that crosses a layer boundary SHALL be expressed in these facts"`) so it aligns with `ARCHITECTURE.md` L207–208 (`"lifecycle events, answers and commands"` on Waist 2): distinguish the **6 domain ontology facts** (payload data model) from the **Waist-2 JSON-RPC transport/lifecycle envelope** (`discover`, `heartbeat`, `shutdown`, `approval/resolve`, `question/answer`), resolving the gap noted in `R-45`.
- **Informative Engine Implementation (`monoagens-core`):** All 40 internal mechanisms (drives, mood/affect, associative recall, `self_card`, commitment repair, sleep consolidation) sit *behind* Waist 1. An alternative minimal Rust/Python/LLM engine that implements `monoagens.core.v1` and passes the Core TCK is **100% standard-compliant without implementing a single internal MonoAgens heuristic**.

#### Rule 2 — Turn MonoAgens Lab into Dual Purpose: Internal Cognitive Eval + Public Black-Box `monoagens-conformance` TCK
Separate `monoagens-lab` cleanly into two binaries/crates sharing the discrete-event replay kernel:
1. **`lab-eval` (Internal Cognitive Research):** Measures ablations (`RESEARCH.md` §26), memory recall, `R-49` Integrity Scorecard (`PKR`, `RRR`, `FPR`, `WRP`, `ECE_commit`, `AUC_debt`), affect/drive behavior.
2. **`monoagens-conformance` (`TCK`, black-box, `a2a-tck`-style `--level must|should|may` + machine JSON report):**
   - **Body TCK (tests any Waist-2 body binary/endpoint over stdio/Unix socket/WS/HTTP):**
     - `B-MUST-01` Schema & Discovery (`body/discover` returns valid OpenRPC/JSON-Schema 2020-12 capability manifest in deterministic sorted order for prompt-cache stability).
     - `B-MUST-02` **Single-Receipt Invariant:** Every accepted `action/execute` ends in *exactly one* terminal/parked receipt transition among `{done, refused, held, failed, unknown}` (`ARCHITECTURE.md` L198); duplicate emissions with identical `idempotency_key` return cached outcome without re-execution.
     - `B-MUST-03` **Crash Recovery & `unknown`:** Kill body daemon mid-`execute` and mid-`held` under simulated power/process drop (`SQLite` anomaly testing pattern); restart and assert body responds to `state/query` within $T_{\text{crash\_rec}}$ (`5000 ms` start) reporting `in_flight`, terminal receipt, or explicit `unknown`—never double-executing side effects.
     - `B-MUST-04` **Audience & Place Isolation:** Actions tagged `place = P_1, audience = A_1` must never emit/leak payloads to `P_2` or `A_2` observers.
     - `B-MUST-05` **`held` Approval Round-Trip (`R-42`):** Privileged capability triggers `held` receipt with `approval_id`; body waits idempotently until `approval/resolve(permit | deny | expired)` arrives, then completes cleanly (`done` vs `refused`).
     - `B-MUST-06` **Attachment Content-Address Verification:** Corrupted SHA-256 digest in `blob/get` / `blob/put` MUST return `ERR_HASH_MISMATCH` rather than silent corruption.
     - `B-MUST-07` **Capability Honesty & Determinism:** Undeclared `capability_id` deterministically returns `refused` (never silently succeeds or crashes); replay under identical seeded mock triggers produces valid canonical JSON-RPC ordering.
     - `B-SHOULD-01..03` Latency bounds, `traceparent` W3C propagation (`_meta`), clean `shutdown` drain.
   - **Core TCK (tests any candidate `monoagens-core` implementation behind Waist 1):**
     - `C-MUST-01` **Privacy Non-Interference ($\Delta_{\text{NI}} = 0$):** Inject secret tokens in private place $P_{\text{priv}}$; assert zero bit difference in all outbound `Action`s & prompt context rendered to public place $P_{\text{pub}}$.
     - `C-MUST-02` **Hard-Forget Erasure Audit (`R-34`):** Run `hard_forget(target_id)` then probe recall, summary cards, and context builders; assert zero surviving entities/citations/influence.
     - `C-MUST-03` **Seeded Bit-Identical Determinism:** Two runs with identical `(seed, VirtualClock, Waist-1 input stream)` produce byte-identical `Action` and state-hash sequences.
     - `C-MUST-04` **Neutral-Value & Empty-Run Stability:** All optional/experimental parameters set to neutral values run crash-free over `N` ticks with bounded latency and memory.

#### Rule 3 — Ship One Killer Host (`monoagens-gateway`) + Two Canonical Zero-Friction Reference Bodies Early (`OCI runc` + `LSP` + `MCP`)
Protocol specs without live hosts die as PDFs. Ship three tight reference implementations that validate both chat/tool and embodied/hardware generalizations:
1. **`monoagens-gateway` + Inspector:** Thin Gateway + Valve (`R-41`/`R-42` baseline) + live wire trace/conformance inspector (`OpenTelemetry Weaver live-check` + `MCP Inspector` model).
2. **Reference Body #1 — `body-cli` (Interactive Terminal / Chat Body):** Proves multi-turn conversations, `held` approval prompts, scheduled wake timers, and attachment blobs (`chat-only` capability profile + `universal` compliance).
3. **Reference Body #2 — `body-mcp-bridge` (`R-45`):** Exposes **any MCP `2026-07-28` server** (tools, resources, MRTR handles) as MonoAgens Waist-2 capabilities (`capability_id`, risk tags, schema, single-receipt adapter), instantaneously unlocking thousands of real tools on Day 1.
*(Optional Phase-3 Reference Body #3: `body-sim-actuator` — minimal simulated physical arm/rover proving non-chat embodied `place`/`conflict_group_id`/sensor-percept invariants actually work!)*

#### Rule 4 — `< 200 LOC` Polyglot Adapter SDKs Generated from Schema (`LSP` + `OpenRPC` lesson)
Generate typed client/server stubs from `monoagens.body.v1.openrpc.json` for **Rust** (`trait BodyAdapter`), **TypeScript**, and **Python**. Handle JSON-RPC framing, idempotency cache, crash-journal persistence (local SQLite/WAL or JSONL helper), SHA-256 blob hashing, and `heartbeat` *inside the SDK*, leaving third-party body authors to write only capability declarations + `execute` / `cancel` / `on_approval` handlers (**target `<= 200 LOC` and `< 1 day` (`<= 8 hours`) to pass `100%` of Body TCK `MUST` tests**).

#### Rule 5 — Strict Additive Versioning & Stateless Wire Discipline (`MCP 2026-07-28` + `ARCHITECTURE.md` L57–59)
- Unknown JSON keys MUST be ignored cleanly outside explicitly strict capability `args` (`JSON Schema additionalProperties` rules), with custom vendor fields confined to namespaced `_meta` (`io.monoagens/...` or reverse-DNS keys).
- Every Waist-2 request/response is self-contained with `protocol_version` and explicit resource/approval handles; **never** introduce implicit connection-bound session caches or mandatory multi-step socket handshakes (`MCP 2026-07-28 SEP-2567/2575` reversal!).
- Breaking changes require a new major namespace (`monoagens.body.v2`) with minimum $T_{\text{dep}}$ (`2` minor release cycles) simultaneous `discover` support.

#### Rule 6 — Staged Governance Path (Owner-Locked Fast Freeze $\to$ External Interop $\to$ Foundation)
Honor `ARCHITECTURE.md` L57 (`"only by the owner"`) during v1 convergence so Waists do not bloat by committee (`CORBA`/`WS-*` trap). Once **$\ge 3$ independent external bodies** and **$\ge 2$ distinct `Core` implementations** pass `monoagens-conformance` `100%` `MUST`, isolate `monoagens-spec` + `monoagens-conformance` into a vendor-neutral Apache-2.0/MIT governance track (following `MCP`/`A2A` $\to$ LF `AAIF` / `OCI` playbook) while `monoagens-core` stays free to innovate behind Waist 1.

---

### 3. Honest Assessment of Current Roadmap Tension & Phased Order of Work

There is a sharp, unacknowledged strategic conflict between the repo's current OpenSpec plans:
- **`openspec/changes/core-and-lab/proposal.md` + `tasks.md`** specifies building **all 22 cognitive brain-map rows (`groups 1–22`) + `TestBody`-only Lab first**, keeping **`bootstrap-vertical-slice` parked (`PARKED.md`)** and deferring Gateway, Valve, real CLI/chat bodies, and Waist 2 (`R-45`) until after Core + Lab are finished.
- **Why pure "Core-and-Lab-first (only `TestBody`)" risks standardization failure:**
  1. **`TestBody` creates a false wire illusion:** Synthetic synchronous mocks hide real-world transport disconnects, async `held` pause/resume, attachment store races, and MCP bridge impedance mismatches—meaning Waist 1 and Waist 2 will almost certainly break right when `bootstrap-vertical-slice` unparks (repeating MCP's `2024 -> 2026-07-28` painful breaking session surgery).
  2. **The FIPA ACL trap:** Months spent perfecting 40 internal mechanisms (mood, curiosity, boredom, sleep, trust curves) with zero runnable `Gateway`, zero live CLI/MCP body, and zero public TCK produces zero ecosystem adoption signal and no external validation that Waist 2 actually works outside Rust tests.
- **Why pure "Ship-Gateway-before-Core-invariants" ALSO fails:** Unparking `bootstrap-vertical-slice` without `expectations`, `Valve` safety (`R-41`/`R-42`), `hard_forget`, and deterministic `Core::step` creates just another forgettable stateless agent wrapper (`LangChain` script trap).

**Recommended Low-Regret Sequence (4 Phased Milestones with Hard Gates):**

| Phase | Deliverables (Interleaved cleanly with `core-and-lab` Stage A/B) | Exit Gate (Falsifiable Criteria) |
| :--- | :--- | :--- |
| **Phase 0: Waist Freeze & TCK Skeleton** *(Runs before/with `core-and-lab` tasks 1–4)* | Fix `protocol/spec.md` L12–15 vs `ARCHITECTURE.md` L207–208; publish **`monoagens.core.v1`** & **`monoagens.body.v1`** (`OpenRPC 1.4.1` + `JSON Schema 2020-12`); build `monoagens-conformance` CLI stub; route `TestBody` *through the real Waist-2 JSON-RPC codec* rather than custom Rust shortcuts. | `0` schema lint errors; `TestBody` communicates strictly over frozen Waist-2 messages (`0` shortcut structs). |
| **Phase 1: Minimum Viable Ecosystem Loop** *(Unparks stripped core of `bootstrap-vertical-slice` alongside minimal `core-and-lab` Stage A)* | Minimal deterministic `Core::step` (`log`, `expectations`, `sources`, basic `memories`, **all optional affective/drive rows at strict Neutral Values**); thin `Gateway` + baseline `Valve` (`held` + rate limit); **`body-cli`** + **`body-mcp-bridge`**; **Rust + TS/Python thin Body SDKs**; **`monoagens-conformance v0.1`** (`MUST` suite for Body + Privacy/Determinism Core checks). | `100%` `MUST` Body TCK pass on `body-cli` & `body-mcp-bridge`; internal/external dogfood builds 1 new body in **`< 8 hours` (`<= 200 LOC`)**; `C-MUST-01..04` Core invariants green. |
| **Phase 2: Cognitive Depth Behind Frozen Waists** *(Executes remaining `core-and-lab` rows 5–22 without breaking Phase 1 adopters)* | Enable & ablate `R-49` commitment graph/repair, `R-35` `self_card`, `R-34` Two-Epoch Replay + `hard_forget`, drives, affect, sleep, and social learning inside `monoagens-core` (`Lab::Cognitive` sweeps). Enforce zero breaking Waist-1/2 wire diffs via CI `conformance` gate. | Every cognitive row beats its neutral baseline in `Lab::Cognitive` (`RESEARCH.md` §26); **`0` breaking Waist-2 changes** required for Phase-1 bodies; `Core TCK` stays 100% green. |
| **Phase 3: Cross-Body Generalization & Ecosystem Opening** | Add `body-sim-actuator` (`embodied-only` profile verifying `universal` portability), second independent `Core` prototype (e.g., minimalist rule/LLM core passing Core TCK), Inspector UI, public conformance badges (`compatibility.json`). | `>= 3` independent bodies $\times$ `>= 2` independent Cores pass `100%` `MUST` interop matrix cleanly. |

---

## Parameters

*(Note: Engineering threshold values below are **not settled by peer-reviewed literature**; grounded as start values from observed ecosystem norms (`A2A v1.0.1` 11 RPCs, `OCI` 5 runtime ops) to tune/track during MonoAgens release engineering.)*

| Name | Meaning | Start value | Search range | Source / Status |
| :--- | :--- | :--- | :--- | :--- |
| `N_waist2_methods` | Upper bound on core Waist-2 JSON-RPC methods (`monoagens.body.v1`) | `12` | `[8, 16]` | Not settled by literature; calibrated vs `A2A` (11 RPCs) / `OCI` (5 ops) |
| `T_first_body_max` | Target developer time to implement and pass Body TCK for a new body | `8 h` (`< 1 day`) | `[2 h, 24 h]` | Engineering adoption gate; measure in dogfood trials |
| `LOC_adapter_max` | Target hand-written adapter code lines using official Waist-2 SDK | `200 LOC` | `[100, 350]` | Engineering ergonomics target; verify across TS/Python/Rust |
| `T_crash_recovery_ms` | Max time after body restart to answer `state/query` with exact state or `unknown` | `5000 ms` | `[1000 ms, 30000 ms]` | Not settled by literature; tune in Body TCK fault harness |
| `T_deprecation_cycles` | Minimum minor spec releases supporting old field/method before removal | `2` minor cycles | `[1, 4]` | Engineering compatibility norm (vs MCP breaking churn risk) |
| `N_interop_bodies` | Independent external bodies required before declaring v1 ecosystem stability | `3` (CLI + MCP + 1 external/embodied) | `[2, 5]` | Not settled by literature; Phase-3 exit gate |
| `N_interop_cores` | Independent `Core` implementations passing Core TCK before foundation handoff | `2` | `[2, 4]` | Not settled by literature; proves Core non-lock-in |
| `Delta_NI_max` | Maximum allowed private-to-public leakage score on Core TCK `C-MUST-01` | `0.0` (exact) | `[0.0, 0.0]` | Architectural hard invariant (`ARCHITECTURE.md`, `R-33`) |

---

## Neutral value

- **Minimal Standard Profile (`Waist-1/2 MUST-only`):** Runs `Core::step` with all experimental drives, mood, curiosity, sleep consolidation, and advanced cognitive heuristics set to their **Neutral Value** (`expectations` + `sources` basic ledger + deterministic `Valve` only), talking over stateless **`monoagens.body.v1`** `MUST` methods (no streaming, no attachments, no optional `_meta` extensions). Every conformant Body and Core MUST interoperate cleanly at this neutral floor before any `SHOULD`/`MAY` extensions activate.

---

## Claim

1. Freezing **`monoagens.body.v1`** ($\le 12$ stateless JSON-RPC methods in OpenRPC 1.4.1 / JSON Schema 2020-12) and decoupling internal cognitive mechanisms behind Waist 1 allows adding all 22 `core-and-lab` brain-map rows with **`0` breaking Waist-2 wire changes** for Phase-1 bodies.
2. Providing generated thin Rust/TS/Python SDKs plus black-box **`monoagens-conformance`** (`--level must|should|may`) reduces time-to-first-conformant-body to **`< 8 hours` (`<= 200` hand-written LOC)** while catching **100% of injected receipt, crash-recovery, idempotency, hash-mismatch, and cross-place leakage faults** missed by unit mocks.
3. Unparking the thin **`gateway + valve + body-cli + body-mcp-bridge`** vertical slice in Phase 1 (behind frozen Waist 1/2) eliminates Waist-2 interface drift prior to Phase 2 cognitive expansion and immediately bridges existing MCP `2026-07-28` servers (`R-45`).
4. Enforcing **`Core TCK` (`C-MUST-01..04`)** ensures cross-core portability across **$\ge 2$ distinct Core implementations** with $\Delta_{\text{NI}} = 0.0$ and zero `hard_forget` leakage.

---

## How the Lab would test it

Operationalize **`monoagens-conformance`** as an executable black-box harness in Rust (plus language-agnostic stdin/socket/WS runners so Python/TS bodies run unmodified):

1. **Waist-2 Fault-Injection Body TCK (`MUST`/`SHOULD`/`MAY` suite):**
   - Runs automatically against `TestBody`, `body-cli`, `body-mcp-bridge`, `body-sim-actuator`, and third-party adapters via `./monoagens-conformance body --endpoint ... --level must|should|may --json-report compatibility.json`.
   - Deterministic chaos tests: (a) replay identical `idempotency_key` 3x $\to$ assert external mock counter `== 1`; (b) `SIGKILL` body subprocess mid-`execute` and mid-`held` $\to$ restart $\to$ assert `state/query` returns valid unresolved/terminal/`unknown` without duplicate dispatch; (c) corrupt attachment SHA-256 byte $\to$ assert `ERR_HASH_MISMATCH`; (d) cross-place observer probe $\to$ assert zero cross-place bleed; (e) capability schema fuzzing against JSON Schema 2020-12.
2. **Waist-1 Swappable Core TCK (`monoagens-conformance core`):**
   - Drives any candidate `Core` trait/IPC server with deterministic scripted Waist-1 sequences checking `C-MUST-01` ($\Delta_{\text{NI}} = 0$ counterfactual private-place injection), `C-MUST-02` (`hard_forget` erasure completeness), `C-MUST-03` (bit-exact `seed + VirtualClock` determinism), and `C-MUST-04` (neutral-profile crash-free 10k-tick soak).
3. **Cross-Matrix Interoperability Runner:**
   - $N_{\text{cores}} \times N_{\text{bodies}}$ matrix executing canonical smoke scenarios (`multi-turn Q&A`, `held approval allow/deny`, `timer-backed promise lifecycle`, `MCP long-running tool` / `embodied mutex actuator`), asserting `0` matrix cell failures.
4. **Body Authoring Ergonomics Trial (Human/Developer Benchmark):**
   - Timed clean-room build of one net-new body (e.g., matrix/webhook or simulated hardware node) using only published `monoagens-spec` + SDK + `monoagens-conformance`; measure wall-clock hours to `100% MUST` pass (`< 8 h`), hand-written LOC (`<= 200`), and number of spec ambiguities encountered (`0` blocking ambiguities).

---

## Established / inferred / open

- **Established:**
  - Small frozen boundaries paired with executable black-box TCKs + reference hosts + machine-readable schemas (`OCI v1.3.0`, `LSP 3.17/3.18 metaModel.json`, `A2A v1.0.1` + `a2a-tck`, `SQLite TH3`, `OTel Weaver`, `HTTP RFC 9110`) consistently achieve durable multi-vendor interoperability, whereas heavyweight internal-mental or kitchen-sink committee specs (`FIPA ACL`, `CORBA`, `SOAP/WS-*`) fail to win mainstream adoption (`Henning 2006`, `Poslad 2007`, `Pautasso et al. 2008`, `Wooldridge 2000`).
  - Shipping stateful connection handshakes prematurely leads to painful breaking protocol surgery at scale (`MCP 2024-11-05 -> 2026-07-28` SEP-2567/2575 stateless rewrite).
  - Falsifiable requirement extraction against RFC 2119 (`MUST`/`SHOULD`/`MAY`) uncovers real conformance gaps missed by informal prose (`Rautenstrauch & Stock 2024`; `a2a-tck`).
- **Inferred:**
  - Deferring real `Gateway` / `Valve` / `body-cli` / `body-mcp-bridge` / Waist-2 schemas until after all 22 `core-and-lab` rows are built creates high risk of late Waist breakage and ecosystem invisibility; a 4-phase hybrid interleaving (`Phase 0 Waist Freeze` $\to$ `Phase 1 Thin Ecosystem Loop` $\to$ `Phase 2 Cognitive Depth` $\to$ `Phase 3 Interop Matrix`) avoids both traps.
  - Separating `monoagens-spec` (minimal Waist 1/2 wire + black-box TCK) from `monoagens-core` (optional cognitive mechanisms) lets rival cores interop cleanly without inheriting MonoAgens's full internal ontology.
- **Open (not settled by the literature; calibrate in engineering/Lab):**
  - Exact wire transport defaults for remote Waist-2 plugins (stdio/UDS vs WebSocket vs HTTP+JSON/gRPC parity like `A2A`) under mobile/embodied intermittent networks.
  - Exact threshold trade-offs in `monoagens-conformance` fault timeouts (`T_crash_recovery_ms`) across ultra-low-latency embodied actuators vs slow cloud webhooks.
  - Exact governance timing criteria beyond `(>= 3 external bodies, >= 2 independent cores)` for moving `monoagens-spec` under a neutral foundation (e.g., LF `AAIF`).

---

## Sources

- `[IND]` Open Containers Initiative — *Runtime Specification v1.3.0* (`2025-11-04`), *Image Specification v1.1.1* (`2025-03-03`), `runtime-tools` validation suite, and `runc` reference runtime (`1.6.0-rc.2+dev`): [runtime.md](https://raw.githubusercontent.com/opencontainers/runtime-spec/main/runtime.md), [runtime-tools](https://raw.githubusercontent.com/opencontainers/runtime-tools/master/README.md), [runc](https://raw.githubusercontent.com/opencontainers/runc/main/VERSION).
- `[IND]` Microsoft — *Language Server Protocol (LSP) 3.17 / 3.18* & machine-readable `metaModel.json`: [metaModel 3.17](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/metaModel/metaModel.json), [metaModel 3.18](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.18/metaModel/metaModel.json).
- `[IND]` SQLite — *How SQLite Is Tested* (`TH3` 100% branch + MC/DC coverage, `590x` test-to-code ratio, `SLT`, `dbsqlfuzz`): [sqlite.org/testing.html](https://www.sqlite.org/testing.html).
- `[IND]` Model Context Protocol (`2026-07-28` Specification & Changelog, `Inspector`, official SDKs, Rust `rmcp 3.10.0`) & Agentic AI Foundation (Linux Foundation, announced Dec 9, 2025): [MCP Spec](https://modelcontextprotocol.io/specification/2026-07-28), [Changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog), [Inspector](https://modelcontextprotocol.io/docs/tools/inspector), [SDKs](https://modelcontextprotocol.io/docs/sdk), [AAIF](https://aaif.io/).
- `[IND]` Agent2Agent (A2A) Protocol (`v1.0.0` Mar 12, 2026; `v1.0.1` May 26, 2026; `a2a.proto` 11 RPCs; `a2a-tck` RFC 2119 conformance kit): [A2A Spec](https://a2a-protocol.org/latest/specification/), [CHANGELOG](https://raw.githubusercontent.com/a2aproject/A2A/main/CHANGELOG.md), [a2a.proto](https://raw.githubusercontent.com/a2aproject/A2A/main/specification/grpc/a2a.proto), [a2a-tck](https://raw.githubusercontent.com/a2aproject/a2a-tck/main/README.md).
- `[IND]` OpenTelemetry Specification & Weaver (`v0.28.0`, Oct 2026): [OTel Overview](https://opentelemetry.io/docs/specs/otel/overview/), [Weaver README](https://raw.githubusercontent.com/open-telemetry/weaver/main/README.md).
- `[IND]` IETF Standards — HTTP Semantics `RFC 9110` (`STD 97`), `RFC 2119` & `RFC 8174` (`BCP 14` requirement keywords), JSON `RFC 8259`: [RFC 9110](https://www.rfc-editor.org/rfc/rfc9110.txt), [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119.txt), [RFC 8174](https://www.rfc-editor.org/rfc/rfc8174.txt), [RFC 8259](https://www.rfc-editor.org/rfc/rfc8259.txt).
- `[PR]` Fielding, R. T., & Taylor, R. N. (2002). *Principled Design of the Modern Web Architecture*. ACM Transactions on Internet Technology (TOIT), 2(2):115–150. DOI: `10.1145/514183.514185`.
- `[PR]` Rautenstrauch, J., & Stock, B. (2024). *Who's Breaking the Rules? Studying Conformance to the HTTP Specifications and its Security Impact*. ACM ASIACCS 2024, pp. 843–855 (106 mined falsifiable spec rules). DOI: `10.1145/3634737.3637678`.
- `[PR]` Poslad, S. (2007). *Specifying Protocols for Multi-Agent Systems Interaction*. ACM Transactions on Autonomous and Adaptive Systems (TAAS), 2(4):Art. 15 (FIPA ACL architecture & adoption critique). DOI: `10.1145/1293731.1293735`. *(FIPA specs referenced: `SC00037` Communicative Act Library & `SC00061` ACL Message Structure.)*
- `[PR]` Wooldridge, M. (2000). *Semantic Issues in the Verification of Agent Communication Languages*. Autonomous Agents and Multi-Agent Systems, 3:9–31. DOI: `10.1023/a:1010090027213`.
- `[PR]` Henning, M. (2006). *The Rise and Fall of CORBA*. ACM Queue, 4(5):28–34. DOI: `10.1145/1142031.1142044`.
- `[PR]` Pautasso, C., Zimmermann, O., & Leymann, F. (2008). *RESTful Web Services vs. "Big" Web Services: Making the Right Architectural Decision*. WWW '08, pp. 805–814. DOI: `10.1145/1367497.1367606`.
- `[IND]` Schema & Rust Protocol Tooling — OpenRPC `1.4.1` ([spec.open-rpc.org](https://spec.open-rpc.org/)), JSON Schema Draft `2020-12` ([json-schema.org](https://json-schema.org/draft/2020-12/schema)), `schemars 1.2.3`, `serde_json 1.0.154`, `jsonrpsee 0.26.1`.
- `[IND]` LangChain `agent-protocol` ([github.com/langchain-ai/agent-protocol](https://raw.githubusercontent.com/langchain-ai/agent-protocol/main/README.md)) — contrast of OpenAPI wrapper vs decoupled wire protocol + TCK.

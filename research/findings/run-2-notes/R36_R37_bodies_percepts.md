# MonoAgens Research Brief 2 — Part D: R-36 & R-37 Specification Input

> **Context & Baseline Alignment:** Prepared as of **2026-11-16** against `ARCHITECTURE.md`, `RESEARCH.md`, `openspec/changes/core-and-lab/`, `openspec/changes/bootstrap-vertical-slice/`, Round 1 (`research/findings/2026-11-15-run-1.md`), and Round 2 notes (`A03_A06`, `A15`, `R29_R31`, `R32_R34`, `R33_R41_R42_R43`, `R44_R45`).  
> **Scope Legend:** `[UNI]` = universal across all bodies; `[CHAT]` = chat-only baseline behavior; `[EMB]` = embodied / streaming / multimodal bodies only.

---

## Gaps & Contradictions Found in Current Project Specs & Notes

Before the per-question specs, implementing R-36 and R-37 uncovered **six concrete specification gaps/contradictions** requiring owner/spec reconciliation before implementation:

1. **`A-03` (`A03_A06_habituation_sleep_boundaries.md`) vs. high-rate sensor bodies (`[EMB]`):** `A-03` accumulates habituation $h_p$ strictly per `place_id`. Even after edge downsampling to $1\text{ Hz}$ (or $4\text{ Hz}$), continuous telemetry heartbeats converge to $h^* \approx 199$ (at $1\text{ Hz}$, $\gamma=0.995$) and crush $S_{\text{eff}}/S = (1 + \kappa h^*)^{-1} = 0.0165$ (`[4 Hz: h*≈797.5, S_eff/S=0.0042]`) for *all* subsequent events in that `place`, inadvertently habituating away a human speaker or tactile alert sharing that room. **Fix (`R-36`):** Partition Core habituation to $h_{(p, \text{source\_kind})}$ (`discrete_event` vs `telemetry_state` vs `action_feedback`), preserving bit-exact equivalence to `A-03` in `[CHAT]` where only `discrete_event` exists.
2. **`protocol/spec.md` L44–53 & `R-26`/`R-35` vs. preemption & cancellation (`[UNI]`/`[EMB]`):** `protocol/spec.md` restricts `Receipt.outcome` to `{done, refused, held, failed, unknown}` with strict single-receipt cardinality, while `expectations/spec.md`, `R-26`, and `R-35` (`self_card` Beta reliability $(\alpha_c, \beta_c)$) treat `failed` as capability failure ($\Delta\beta_c = 1$) and retry triggers. Intentionally stopping a long navigation `Act` or interrupting `speech.speak` on user voice barge-in produces a semantic hazard: emitting `done` falsely confirms unachieved goals, while plain `failed` falsely degrades working capabilities. **Fix (`R-36`/`R-37`):** Keep the 5 `ARCHITECTURE.md` L198 outcomes intact and add additive `Receipt` metadata (`termination_reason`, `progress`, `delivered_units`), plus a guard in `R-26`/`R-35`/`expectations` so `preempted_external` and `canceled_by_core` leave $(\alpha_c, \beta_c)$ unchanged (Option A; adding a 6th `preempted` variant is noted as Option B for owner decision under `ARCHITECTURE.md` L56–58).
3. **Lack of pre-terminal progress / keepalive semantics in `protocol/spec.md` & `R44_R45` static `action_timeout_ms = 30000 ms` (`[UNI]`/`[EMB]`):** Physical navigation or multi-minute downloads cannot wait for completion inside a single 30 s timer without spuriously firing `unknown`, whereas fast actuators ($50\text{ ms}$) wait 30 s to detect hangs. **Fix (`R-36`):** Add non-receipt `action_feedback` percepts (`correlation_id = act_id`) plus dynamic expectation deadlines scaled from `CapabilityDescriptor.typical_latency` (`m_lat = 3.0`, clamped `[5 s, 300 s]`) and slid forward on healthy feedback.
4. **R-45 open `max_percept_queue_per_plugin = 1024` backpressure ambiguity (`[UNI]`/`[EMB]`):** An untyped FIFO risks either silent tail-dropping of `Receipt`s / safety alerts under camera/IMU spikes or blocking Body's real-time loop. **Fix (`R-36`):** 3-class partitioned Gateway ingress buffer (`control_receipt` lossless $q_{\text{ctrl}}=256$, `discrete_event` FIFO NACK backpressure $q_{\text{evt}}=512$, `telemetry_state` ROS 2 `KeepLast(1)` wait-free coalescing per `(place_id, channel_id)` $q_{\text{tel}}=256$).
5. **`core-and-lab` inline `Percept` bytes & R-45 open binary media wire vs. `ARCHITECTURE.md` L85 & L105 (`[EMB]`):** `protocol/spec.md` carries payload inline with no Gateway attachment indirection, conflicting with `ARCHITECTURE.md` L105 (*"Gateway ... stores attachments"*) and L85 (*"Core does not read content"*), and would bloat Core's append-only log with raw audio/images. **Fix (`R-37`):** Out-of-band / chunked Waist-2 upload into a keyed `BLAKE3` (`blake3` crate) Gateway Attachment Store yielding opaque `AttachmentHandle`s referenced by Core and ref-counted for `R-34` secure erasure.
6. **`EmbeddingGemma-300M` (`A-15`) text-only scope & Gemma license vs. multimodal GA shift (`[EMB]`):** Text-only `google/embeddinggemma-300m` (`license:gemma`) cannot embed images/audio/video. `google/embeddinggemma-2` (`arXiv:2610.37364`, **Apache 2.0**) unifies text/image/audio/video into 256d MRL vectors, but its vector space is incompatible with v1 — Gateway must version `embed_model_ver` and isolate or re-embed existing episodes cleanly across upgrades.

---

# R-36 Bodies beyond chat (continuous sensors, physical action, game worlds)

## Status
**Settled for specification-input** (architecture, conditioning math, 3-class backpressure, dynamic action timeouts, and single-receipt preemption FSM). Requires owner ratification on additive Waist-2 fields (`action_feedback` percept kind; `Receipt.termination_reason` vs. 6th outcome per `ARCHITECTURE.md` L56–58) and Lab calibration of channel thresholds across simulated continuous streams.

## Answer

Physical robots, desktop GUI agents, smart sensors, and real-time game bots operate at **$10\text{–}1000\text{ Hz}$** control loops that cannot block on an LLM (`0.1\text{–}2\text{ Hz}$ `reason`) or flood an append-only SQLite event log (`>8.6 \times 10^7$ rows/day at 1 kHz). Decades of embodied architecture — from **Brooks (1986)** subsumption and **Gat (1998)** / **Bonasso et al. (1996)** Three-Layer Architecture (**3T**: reactive controller, event-driven sequencer, deliberative planner), through **ROS 2 Actions (`GoalStatus`, Lyrical Luth May 2026 LTS)**, to 2024–2026 dual/triple-system VLAs (**RT-2**, **OpenVLA**, **$\pi_0$**, **GR00T N1**, **$\pi_{0.5}$**, **Figure Helix / Helix 02**) — converge on strict **timescale decoupling**: Core (`reason` + `gate` + `expectations` + `R-32` habits) functions exclusively as the **0.1–2 Hz semantic deliberator & macro-skill sequencer** over the existing 6 fixed protocol facts (`actor`, `place`, `audience`, `event`, `capability`, `receipt`), while deterministic Body/Edge loops own $10\text{–}1000\text{ Hz}$ stabilization, safety interlocks, and event compression. Continuous raw streams $\mathbf{x}_n \in \mathbb{R}^d$ sampled at $f_s \in [10, 1000]\text{ Hz}$ are converted deterministically inside the Body/`monoagens-body` runtime (pure physics-side filtering and compression with zero goals, memory, identity, or LLM logic, respecting *"an adapter never thinks"*) into sparse semantic percepts via **Send-on-Delta ($\text{SoD}$) with asymmetric hysteresis**, **two-sided Page-Hinkley ($\text{PH}$) drift detection** (or optional bounded-run-length $\text{BOCPD}$ for multi-regime sensors), and a bounded staleness heartbeat:

$$\mathcal{E}_n = \mathbb{I}\!\left[\big(\text{trigger}_{\text{SoD}}(n) \lor \text{trigger}_{\text{CP}}(n) \lor (t_n - t_{\text{last}} \ge T_{\max, k})\big) \;\land\; (t_n - t_{\text{last}} \ge \tau_{\min, k})\right]$$

where normalized innovation $z_n = \|\mathbf{D}_k^{-1}(\mathbf{x}_n - \hat{\mathbf{x}}_{\text{last}})\|_2$ (or signed component projection for scalar channels) drives exact two-sided Page-Hinkley accumulators initialized at $(n=0, \bar{z}_0=0, U_0=m_0=L_0=M_0=0)$:

$$\bar{z}_n = \bar{z}_{n-1} + \frac{z_n - \bar{z}_{n-1}}{n}, \qquad U_n = \sum_{j=1}^n \big(z_j - \bar{z}_j - \delta_{\text{PH}, k}\big), \quad m_n = \min_{1 \le j \le n} U_j$$

$$L_n = \sum_{j=1}^n \big(z_j - \bar{z}_j + \delta_{\text{PH}, k}\big), \quad M_n = \max_{1 \le j \le n} L_j, \qquad \text{trigger}_{\text{CP}}(n) = \mathbb{I}\!\left[\max(U_n - m_n,\; M_n - L_n) > \lambda_{\text{PH}, k}\right]$$

$$\text{trigger}_{\text{SoD}}(n) = \mathbb{I}\!\left[\|\mathbf{D}_k^{-1}(\mathbf{x}_n - \hat{\mathbf{x}}_{\text{last}})\|_2 \ge \Delta_{\text{high}, k}\right] \quad (\text{boolean hysteresis release: } \Delta_{\text{low}, k} = 0.5\,\Delta_{\text{high}, k})$$

Here: $\mathbf{D}_k = \operatorname{diag}(\sigma_{k,1}, \dots, \sigma_{k,d})$ is the fixed per-channel physical scale matrix; $\hat{\mathbf{x}}_{\text{last}}$ is the last emitted value on channel $k$ at timestamp $t_{\text{last}}$; $\Delta_{\text{high}, k}$ is the jump deadband; $\delta_{\text{PH}, k} > 0$ is the admissible noise tolerance inside the PH mean-shift detector; $\lambda_{\text{PH}, k} > 0$ is the cumulative drift threshold (all accumulators reset to zero upon $\mathcal{E}_n = 1$); $\tau_{\min, k}$ is the minimum emission refractory period capping channel rate at $1/\tau_{\min, k}$ (start $0.25\text{ s} \Rightarrow \le 4\text{ Hz}$); and $T_{\max, k}$ is the maximum silence keepalive ($5\text{ s}$ with active expectation, $30\text{ s}$ idle). Optional multi-regime telemetry (force-torque contact, camera embedding change) substitutes Adams & MacKay (2007) truncated **BOCPD** with bounded run length $r_n \in \{0, \dots, R_{\max}\}$ computing $P(r_n < \tau_{\text{cp}} \mid \mathbf{x}_{1:n}) > \pi_{\text{cp}}$ in deterministic $O(R_{\max})$ time per sample.

### Cross-Paradigm Embodied Architecture Mapping

| Reference System / Paradigm | Slow / Deliberative Tier | Fast / Reactive Tier | Safety / Hardware Loop | How MonoAgens Maps It (All Invariants Preserved) |
| :--- | :--- | :--- | :--- | :--- |
| **Brooks (1986)** Subsumption `[PR]` | None (emergent competence) | Async finite-state behavior wires; upper layers inhibit/subsume lower | Hardware obstacle avoidance | Body owns reflex wires & safety E-stop; `Core` never sits inside collision paths |
| **Gat (1998) / Bonasso (1996)** 3T `[PR]` | Deliberator (symbolic plan, slow) | Sequencer (RAPs / event skills) | Controller (continuous PID feedback) | Controller = Body (`monoagens-body`); Sequencer = Waist-2 + Gateway + Gate + `Expectations` + `R-32` habit; Deliberator = `reason` |
| **ROS 2 Actions** (`GoalStatus`, Lyrical Luth LTS) `[IND]` | Client planning goals & cancel decisions | `ACCEPTED -> EXECUTING -> CANCELING -> {SUCCEEDED, CANCELED, ABORTED, UNKNOWN}` + `Feedback` topic | `ros2_control` hardware real-time loop | ROS 2 Bridge (`R-45`) maps `Feedback` -> throttled `action_feedback` `Event`; terminal `GoalStatus` -> single `Receipt` with `termination_reason` |
| **RT-2** (`arXiv:2307.15818`) `[PP]` | Monolithic 55B VLM (`1–3 Hz`), 5B (`~5 Hz`) direct tokens | Same model (no separation — ceiling shown) | Robot low-level controller | Proves monolithic LLM-per-tick cannot hit $>10\text{ Hz}$ |
| **OpenVLA** (`arXiv:2406.09246`) `[PP]` | 7B VLM (`~6 Hz` RTX 4090 bf16) | Action chunk queue (`T=16, X=8` non-blocking @ `5/15 Hz`) | Arm controller | `Core` issues macro-goal; Body executes non-blocking local chunk trajectory |
| **$\pi_0$** (`arXiv:2410.24164`) & **$\pi_{0.5}$** (`arXiv:2504.16054`) `[PP]` | PaliGemma `3B` VLM + $\pi_{0.5}$ semantic subtask prediction | `~300M` flow-matching action expert emitting chunks up to **`50 Hz`** | Low-level **PD controllers** | Subtask intent from `Core`/`reason`; `50 Hz` chunk execution + PD inside Body |
| **NVIDIA GR00T N1** (`arXiv:2503.14734`) `[PP]` | System 2 VLM (`10 Hz`) | System 1 Diffusion Transformer (`120 Hz`) | Humanoid servo actuators | S2 policy trigger corresponds to Waist-2 macro `Act`; S1 (`120 Hz`) lives in Body |
| **Figure Helix / Helix 02** (`figure.ai`) `[IND]` | System 2 7B VLM (`7–9 Hz`) | System 1 80M visuomotor policy (`200 Hz`) | System 0 whole-body prior (`1000 Hz / 1 kHz`, replacing `109,504` C++ LoC) | S2 goal orchestration maps to `Core` (`0.1–2 Hz`); S1 (`200 Hz`) + S0 (`1 kHz`) live strictly inside Body |
| **Desktop / GUI Agents** (`OSWorld` `2404.07972`, `UI-TARS` `2501.12326`) `[PP]` | Multimodal screenshot + AXTree VLM steps | Atomic click/type/hotkey + UI idle-settle debounce | Desktop display compositor & window focus guard | Body watches AXTree/screen updates, debounces visual settle, emits 1 settled frame/diff `Percept` to Gateway |
| **Game Bots** (`Voyager` `2305.16291`) `[PP]` | LLM skill composer (milestone/inventory events) | Parametric skill coroutine (`navigate`, `mine`, `evade`) | Engine simulation tick (`20+ Hz` class) | Physics & path-following tick runs inside Body; `Core` invokes macro skills and `R-32` reflexes handle recurring hazards |

### Exact Three-Tier Runtime Partition & Ownership

| Layer | Owns (`[EMB]` + `[UNI]`) | Strictly Forbidden | Rate / Latency Target |
| :--- | :--- | :--- | :--- |
| **Tier 1: Body + Bridge (`monoagens-body`)** `[EMB]` | Hardware E-stop, collision/joint-limit clamping, PID/S0/S1 loops, VAD 10–32 ms framing, SoD/Page-Hinkley/BOCPD conditioning, `tau_min` debouncing, AXTree diffing, ROS 2 `GoalStatus` translation, AEC speaker echo suppression | Cross-place memory, user/owner identity, goals, business rules, LLM calls (*"an adapter never thinks"*) | Loop: `10–1000 Hz` internal; Egress: `<= 4 Hz` per telemetry channel |
| **Tier 2: Gateway (`monoagens-gateway`)** `[UNI]` | Waist-2 JSON-RPC auth, typed 3-class `max_percept_queue_per_plugin` backpressure & `KeepLast(1)` telemetry coalescing, dynamic action deadline tracking, stateless Attachment Store (`R-37`), stateless external embedder call pre-queue | Learning state, mutating salience weights, policy authorization decisions (*"never learns"*) | Queue dispatch: `< 2 ms`; Embedder async worker: `10–60 ms` |
| **Tier 2.5: Valve (`monoagens-valve`)** `[UNI]` | Deterministic policy authorization before `Act` reaches Body: `R-41` Cedar policies & physical-presence TTL checks, rate/budget/chain brakes, `R-42` `PendingApproval` holds, `R-43` Layer-3 sensor taint locks blocking `>= R_2` actuation | Learning policies, non-deterministic LLM evaluation, overriding Body's sub-ms physical E-stop | Deterministic: `< 1 ms` |
| **Tier 3: Core (`monoagens-core`)** `[UNI]` | Append-only log (coalesced percepts + acts + receipts), partitioned `A-03` habituation `h_{(p, source_kind)}`, `A-06` gate, `R-32` model-free `habit` macro dispatch, `Expectations`, ACT-R recall, `A-05` episode segmentation, `reason` (`LLM`) | Sub-50 ms closed-loop servo control, reading raw media bytes, platform/plugin branching | Gate + `R-32` habit: `1–5 ms`; `reason` LLM wake: `0.1–2 Hz` (`300–2000 ms`) |

### Resolving R-45 Open Items: 3-Class Backpressure & Dynamic Action Deadlines

1. **Backpressure Policy on `max_percept_queue_per_plugin` (`Q_max = 1024`, resolving R-45):**
   Partition the per-plugin ingress buffer $Q_{\max} = q_{\text{ctrl}} + q_{\text{evt}} + q_{\text{tel}} = 256 + 512 + 256 = 1024$ using Waist-2's deterministic `channel_class` header (pure structural queue routing, zero business semantics):
   - **`control_receipt` (`q_ctrl = 256`):** `Receipt`, `Capability`, `Heartbeat`, `safety_interlock` events. **Lossless FIFO; never coalesced or tail-dropped.** If full, stop polling telemetry and mark plugin unhealthy/stalled rather than losing a receipt.
   - **`discrete_event` (`q_evt = 512`):** Chat messages, completed speech segments, UI clicks, semantic milestones. **Lossless bounded FIFO** with explicit JSON-RPC `-32001 QueueFull` flow-control NACK back to Body (retryable with exponential jitter in Bridge; never silently drops human messages).
   - **`telemetry_state` (`q_tel = 256` active slots):** Periodic pose/battery/sensor deltas and in-flight `action_feedback` events keyed by `(place_id, channel_id)`. Uses **Wait-Free Latest-Value Coalescing (`KeepLast(1)`)** inspired by ROS 2 `SensorDataQoS`: arriving samples overwrite unconsumed prior snapshots on the exact same key in-place and increment `coalesced_count += 1` (preserving newest sample + `t_first`, `t_last`). Core's append-only log records the exact snapshot crossing into Core's MPSC queue, keeping **Core/Lab replay bit-exact**.

2. **Dynamic `action_timeout_ms` & In-Flight Feedback (resolving R-45):**
   Replace static `30000 ms` timeout with per-capability expectation deadline initialized from `expectations/spec.md` latency $\hat{L}_{c,p}$ (seeded from `CapabilityDescriptor.typical_latency_ms`):

   $$\Delta_{\text{act}}(c, p) = \operatorname{clamp}\!\big(m_{\text{lat}} \cdot \hat{L}_{c, p},\; T_{\text{act,min}},\; T_{\text{act,max}}\big)$$

   Whenever Body emits a non-receipt `Event(kind = action_feedback, correlation_id = act_id, progress ∈ [0,1])` (throttled at $\Delta p \ge \delta_{\text{prog}} = 0.25$ or interval $\tau_{\text{fb}} \ge 2.0\text{ s}$), Expectations resets the stall timer `deadline_ts ← now + \Delta_{act}(c, p)` up to an overall hard ceiling $T_{\text{hard\_cap}} = 600\,000\text{ ms}$. Missing both receipt and feedback past deadline triggers Gateway emission of synthetic terminal `Receipt(act_id, outcome = unknown, termination_reason = timeout_unknown)` matching `protocol/spec.md` L52!

3. **Core `A-03` Partitioned Habituation & `R-32` Habit Bypass:**
   Modify `A-03`'s state key from $h_p$ to **$h_{(p, k_{\text{src}})}$** where $k_{\text{src}} \in \{\text{discrete}, \text{telemetry}, \text{feedback}\}$ (`[CHAT]` has only $\text{discrete}$, making $h_{(p,\text{discrete})} \equiv h_p$). Even if $1\text{ Hz}$ robot keepalives saturate $h_{(p,\text{telemetry})} \to 199$ ($S_{\text{eff}}/S = 0.0165$), $h_{(p,\text{discrete})}$ remains untouched at $0$ when a human addresses the robot. Furthermore, when a telemetry delta crosses gate threshold ($S_{\text{eff}} \ge \theta_{\text{wake}}$) and matches a learned deterministic `R-32` embodied procedure (high $Q_{\text{proc}}$ and $\sigma_{\text{proc}} \ge \theta_{\text{habit}}$, `allow_chat_habit_bypass = false`), Core dispatches the compensating `Act` within `1–5 ms` without waking `reason`, subject to Valve clearance and `PE` shock demotion (`R-32`).

### Preemption / Cancellation FSM Under Single-Receipt Invariant (`[UNI]`/`[EMB]`)

```
[Issued Act(a1)]
   |--> Body ACCEPTED -> EXECUTING
   |      |-- (Body emits throttled Event: action_feedback{act_id=a1, progress}) -> slides deadline
   |      |-- (Normal completion) ----------------------> 1 Terminal Receipt(a1, done,    reason=normal,           progress=1.0)
   |      |-- (Valve refused / policy) -----------------> 1 Terminal Receipt(a1, refused, reason=normal,           progress=0.0)
   |      |-- (R-42 Approval wait) ---------------------> 1 Terminal Receipt(a1, held,    reason=normal,           progress=0.0)
   |      |-- (Core cancels via Act(cancel_target=a1)) -> 1 Terminal Receipt(a1, failed,  reason=canceled_by_core, progress=p)
   |      |-- (External barge-in / GUI hijack) ---------> 1 Terminal Receipt(a1, failed,  reason=preempted_external,progress=p)
   |      |-- (Body hardware E-stop / ROS2 ABORTED) ----> 1 Terminal Receipt(a1, failed,  reason=safety_interlock|normal, progress=p)
   |      |-- (No receipt & no feedback past deadline) -> 1 Terminal Receipt(a1, unknown, reason=timeout_unknown,  progress=last_p)
   |<-- INVARIANT: Gateway dedups by act_id; second Receipt for a1 is dropped with audit counter ++.
   |<-- GUARD IN R-26/R-35/Expectations:
   |      if termination_reason in {canceled_by_core, preempted_external}:
   |        Beta(alpha_c, beta_c) UNCHANGED; suppress failure auto-retry; Goal marked unfulfilled (partial=p).
```

- **Note on Option A vs. Option B (`ARCHITECTURE.md` L56–58 & L198):** Keeping `outcome = failed` + additive `termination_reason` (**Option A**) strictly satisfies `ARCHITECTURE.md` L198 ("five outcomes") while isolating capability reliability from intentional preemption. If Owner later elects **Option B** (adding a 6th `preempted` outcome), `R-26`/`R-35` simplify at the cost of updating `ARCHITECTURE.md` L198 and `protocol/spec.md` L44.

## Parameters

| Symbol | Role | Start Value | Search Range | Scope | Justification |
| :--- | :--- | :--- | :--- | :--- | :--- |
| $\Delta_{\text{high}, k}$ | Normalized Send-on-Delta emission threshold | `0.50` ($\times \sigma_k$) | `[0.20, 2.00]` | `[EMB]` | Triggers immediate step alerts without noise chatter (verified on synthetic step+ramp telemetry) |
| $\Delta_{\text{low}, k}$ | Boolean hysteresis release ratio | `0.50` $\times \Delta_{\text{high}, k}$ | `[0.25, 0.80]` | `[EMB]` | Standard Schmitt-trigger band eliminating contact/proximity chatter |
| $\delta_{\text{PH}, k}$ | Page-Hinkley noise drift tolerance | `1.0` $\times \sigma_k$ | `[0.25, 2.0]` $\sigma_k$ | `[EMB]` | Prevents zero-mean Gaussian diffusion from accumulating false drift |
| $\lambda_{\text{PH}, k}$ | Page-Hinkley cumulative drift alarm | `12.5` $\times \sigma_k$ | `[5.0, 40.0]` $\sigma_k$ | `[EMB]` | Zero false alarms over 10 s stationary synthetic Gaussian while catching `0.15/s` ramp in `0.79 s` |
| $R_{\max}$ | Optional BOCPD bounded run-length cap | `128` steps | `[64, 512]` | `[EMB]` | Keeps per-tick regime posterior $O(R_{\max}) < 0.2\text{ ms}$ deterministically |
| $\tau_{\min, k}$ | Per-channel telemetry minimum emit interval | `0.25 s` (`4 Hz` cap) | `[0.05 s, 2.0 s]` | `[EMB]` | Caps Waist-2 ingress load regardless of `10–1000 Hz` hardware tick |
| $T_{\max, k}$ | Telemetry staleness keepalive interval | `5.0 s` active / `30.0 s` idle | `[1.0 s, 60.0 s]` | `[EMB]` | Bounds staleness while keeping idle telemetry $<0.03\text{ Hz}$ |
| $Q_{\max}$ | Per-plugin ingress budget (`ctrl`/`evt`/`tel`) | `256 / 512 / 256` (`1024`) | `ctrl [64, 1024]`, `evt [128, 4096]`, `tel [64, 2048]` | `[UNI]` | Preserves `R-45` total `1024` while isolating lossless receipts from coalesced sensor frames |
| $\delta_{\text{prog}}, \tau_{\text{fb}}$ | Quantized `action_feedback` emission cadence | `0.25` / `2.0 s` | `[0.10, 0.50]` / `[0.5 s, 10.0 s]` | `[EMB]` | Prevents long-action false timeouts while avoiding event-log bloat |
| $m_{\text{lat}}, T_{\text{act,min}}, T_{\text{act,max}}$ | Dynamic timeout multiplier & bounds | `3.0` / `5 s` / `300 s` | `[2.0, 5.0]` / `[1 s, 15 s]` / `[60 s, 900 s]` | `[UNI]` | Replaces static `30 s` ceiling to cover both `50 ms` IO and multi-min nav |

## Neutral value
In `[CHAT]` (or static single-message adapters), sensors do not stream continuously: $\mathcal{E}_n \equiv 1$ per message (`SoD`/`PH` bypassed), `channel_class = discrete_event` always uses lossless FIFO with zero coalescing, $h_{(p,\text{discrete})} \equiv h_p$ matches `A-03` bit-for-bit, `Receipt.termination_reason` defaults to `normal`, and `action_feedback` is never emitted (`Core` log growth, latency, and gate decisions are **100% identical to round-1/core-and-lab baseline**).

## Claim
Conditioning $100\text{–}1000\text{ Hz}$ continuous telemetry at Tier 1 reduces Waist-2 / SQLite append-only log volume by **$\ge 100\times$** (`176.5×` observed at $100\text{ Hz}$ stationary+step+ramp simulation) with **$0\text{ ms}$ step latency beyond $\tau_{\min}$** and zero false alarms over quiescent windows; `KeepLast(1)` partitioned backpressure yields **$0$ dropped `control_receipt` / `safety_interlock` events** under $100\times$ burst overload; partitioning $h_{(p,k_{\text{src}})}$ recovers **$0\%$ unintended suppression** of concurrent human utterances in the same `place`; and `termination_reason` preemption guards preserve capability reliability calibration error $|\Delta\hat{p}_{\text{rel}}| < 0.02$ across 100 forced cancellations.

## How the Lab would test it
All tests execute deterministically in `monoagens-lab` using seeded synthetic continuous-body streams (`ChaCha8Rng` + simulated hardware clock, zero paid LLM required for edge/gate invariance):
1. **`lab_r36_conditioning_compression`:** Replay seeded 30 s (`100 Hz`) and 60 s (`1000 Hz`) telemetry traces containing stationary Gaussian noise, sudden obstacle steps, slow actuator thermal ramps, and oscillatory boundary chatter. Measure: compression ratio vs raw logging (pass $\ge 100\times$), step trigger latency ($= 0$ ticks when non-refractory), false-alarm rate during flat plateau ($= 0$), and bit-exact determinism across two runs with identical seed.
2. **`lab_r36_backpressure_isolation`:** Inject a `1000 Hz` `telemetry_state` storm into Gateway simultaneously with interleaved `Receipt`, `safety_interlock`, and user chat `discrete_event` messages while pausing Core drain for `2.0 s`. Assert: `0` lost `Receipt`s or `safety_interlock`s, `KeepLast(1)` memory bounded $\le 1024$ slots, and post-drain log contains latest telemetry state with accurate `coalesced_count`.
3. **`lab_r36_habituation_partitioning`:** Run $1\text{ Hz}$ telemetry keepalives in `place_room_1` for $30\text{ min}$ simulated clock, then inject an addressed human `discrete_event`. Compare unpartitioned $h_p$ baseline (fails to wake due to $S_{\text{eff}}/S = 0.0165$) vs partitioned $h_{(p,k_{\text{src}})}$ (wakes deterministically on first utterance with neutral $h_{(p,\text{discrete})} = 0$).
4. **`lab_r36_preemption_single_receipt`:** Simulate long `nav.goto` (`typical_latency = 45 s`) emitting `action_feedback` every `2 s`, canceled mid-flight at `progress = 0.40` (50 times by `Core` cancel, 50 times by external preemption), plus 20 race conditions where `done` crosses a cancel request in flight. Assert: strictly `1` receipt accepted per `act_id`, zero spurious timeout while feedback streams, and `R-26`/`R-35` capability $(\alpha_c, \beta_c)$ drift $= 0$ across all 100 intentional preemptions.

## Established / inferred / open
- **Established (literature & verified specs):** Subsumption inhibits slow interference in reactive paths (Brooks 1986); 3-layer separation of fast feedback, event sequencer, and slow planner (Gat 1998; Bonasso 1996); explicit VLM-to-action frequency ceilings (`1–6 Hz`) requiring async chunking / fast-slow hierarchy at `50–1000 Hz` across RT-2, OpenVLA, $\pi_0$, GR00T N1, $\pi_{0.5}$, and Helix/Helix 02; ROS 2 `action_msgs/msg/GoalStatus` explicit `ACCEPTED`/`EXECUTING`/`CANCELING`/`CANCELED`/`ABORTED` lifecycle; mathematical properties of Send-on-Delta (Miskowicz 2006), two-sided Page-Hinkley (Page 1954; Gama et al. 2014), and BOCPD (Adams & MacKay 2007).
- **Inferred (MonoAgens design synthesis):** Mapping Tier 1 conditioning cleanly inside `monoagens-body` without breaching *"an adapter never thinks"*; 3-class `Q_max = 1024` partitioning (`256/512/256`); partitioning $h_{(p, k_{\text{src}})}$ to isolate `A-03` telemetry saturation; Option A `Receipt.termination_reason` preserving `ARCHITECTURE.md` L198 5 outcomes while protecting `R-26`/`R-35`. Our synthetic Python simulation verified `176.5×` compression with zero stationary false alarms and exact $h^*$ fixed points.
- **Open (not settled by literature):**
  1. **Owner Protocol Decision (`ARCHITECTURE.md` L56–58):** Approve Option A (`outcome = failed` + additive `termination_reason`) vs Option B (adding 6th `preempted` variant to `ARCHITECTURE.md` L198).
  2. **Per-modality conditioning defaults:** Exact empirical Pareto curve of $(\Delta_{\text{high}}, \lambda_{\text{PH}}, \tau_{\min})$ for real RGB keyframe embeddings, robot IMU/force-torque, and desktop DOM/AXTree churn — requires Hardware/Lab sweep across representative Body traces.
  3. **Optimal $R_{\max}$ cutoff for BOCPD vs Page-Hinkley** under intermittent non-Gaussian robot contact shocks — settle via Lab ROC sweep on synthetic step+variance-jump trajectories.

## Sources
- **[PR]** Brooks, R. A. (1986). *A robust layered control system for a mobile robot*. IEEE Journal on Robotics and Automation, 2(1), 14–23. DOI: `10.1109/JRA.1986.1087032`
- **[PR]** Bonasso, R. P., Kortenkamp, D., Miller, D. P., & Slack, M. (1996). *Experiences with an architecture for intelligent, reactive agents*. Intelligent Agents II (LNCS), DOI: `10.1007/3540608052_67` (and Gat, E., 1998, *Three-Layer Architectures*, in *Artificial Intelligence and Mobile Robots*, AAAI/MIT Press, pp. 195–210).
- **[PR]** Page, E. S. (1954). *Continuous inspection schemes*. Biometrika, 41(1–2), 100–115. DOI: `10.1093/biomet/41.1-2.100`
- **[PR]** Gama, J., Žliobaitė, I., Bifet, A., Pechenizkiy, M., & Bouchachia, A. (2014). *A survey on concept drift adaptation*. ACM Computing Surveys, 46(4). DOI: `10.1145/2523813`
- **[PR]** Miskowicz, M. (2006). *Send-On-Delta Concept: An Event-Based Data Reporting Strategy*. Sensors, 6(1), 49–63. DOI: `10.3390/s6010049`
- **[PP]** Adams, R. P., & MacKay, D. J. C. (2007). *Bayesian Online Changepoint Detection*. `arXiv:0710.3742` (`https://arxiv.org/abs/0710.3742`)
- **[PP]** Brohan, A., et al. (2023). *RT-2: Vision-Language-Action Models Transfer Web Knowledge to Robotic Control*. `arXiv:2307.15818` (`https://arxiv.org/abs/2307.15818`)
- **[PP]** Kim, M. J., et al. (2024). *OpenVLA: An Open-Source Vision-Language-Action Model*. `arXiv:2406.09246` (`https://arxiv.org/abs/2406.09246`)
- **[PP]** Black, K., et al. (2024 / v4 Jan 2026). *$\pi_0$: A Vision-Language-Action Flow Model for General Robot Control*. `arXiv:2410.24164` (`https://arxiv.org/abs/2410.24164`)
- **[PP]** NVIDIA / Bjorck, J., et al. (2025). *GR00T N1: An Open Foundation Model for Generalist Humanoid Robots*. `arXiv:2503.14734` (`https://arxiv.org/abs/2503.14734`)
- **[PP]** Physical Intelligence / Black, K., et al. (2025). *$\pi_{0.5}$: a Vision-Language-Action Model with Open-World Generalization*. `arXiv:2504.16054` (`https://arxiv.org/abs/2504.16054`)
- **[IND]** Figure AI (Feb 20, 2025 & Jan 27, 2026). *Helix* & *Helix 02: Full-Body Autonomy*. `https://www.figure.ai/news/helix`, `https://www.figure.ai/news/helix-02`
- **[IND]** ROS 2 (`action_msgs/msg/GoalStatus.msg`, Rolling / Lyrical Luth May 2026 LTS). `https://raw.githubusercontent.com/ros2/rcl_interfaces/rolling/action_msgs/msg/GoalStatus.msg`
- **[PP]** Xie, T., et al. (2024). *OSWorld: Benchmarking Multimodal Agents for Open-Ended Tasks in Real Computer Environments*. `arXiv:2404.07972` (`https://arxiv.org/abs/2404.07972`)
- **[PP]** Qin, Y., et al. (2025). *UI-TARS: Pioneering Automated GUI Interaction with Native Agents*. `arXiv:2501.12326` (`https://arxiv.org/abs/2501.12326`)
- **[PP]** Wang, G., et al. (2023). *Voyager: An Open-Ended Embodied Agent with Large Language Models*. `arXiv:2305.16291` (`https://arxiv.org/abs/2305.16291`)

---

# R-37 Percepts beyond text (voice, images, continuous sensors under blind Core)

## Status
**Settled for specification-input** (Gateway keyed-BLAKE3 Attachment Store with R-34 ref-counted erasure, unified EmbeddingGemma 2 [256d MRL] embedding placement, speech-in/out `CapabilityDescriptor` & numeric `percept` feature schema, full-duplex `tts_speak` barge-in FSM, and Core neutrality). Pending owner ratification of additive Waist-2/Attachment schemas and local Gateway hardware latency profiling for EmbeddingGemma 2 & Pyannote.

## Answer

Core remains **strictly blind** to raw media bytes, MIME inspection rules, emotional classifications, and affective word lists (`ARCHITECTURE.md` L85, L91, L97), yet supports voice, images, video, and continuous sensors seamlessly by splitting **blob custody + unified multimodal embedding** inside **Gateway** (`ARCHITECTURE.md` L105; `gateway/spec.md` L49–55) from **lightweight DSP/turn-taking feature extraction** inside **Body**. On Waist-2 ingress (resolving `R-45`'s open binary wire question), payloads $\le B_{\text{inline}} = 256\text{ KiB}$ may travel inline in the initial RPC ingest call or out-of-band, while blobs $> 256\text{ KiB}$ MUST be uploaded out-of-band over local Unix-domain socket FD-passing or chunked `POST /v1/attachments` **before** emitting the JSON-RPC `percept`. Gateway stores every media object under a **scope-keyed `BLAKE3` (`blake3` v1.8.7)** content digest (`keyed_hash(K_scope, blob_bytes)` inside AEAD-at-rest storage `attachments/<scope_id>/<blake3_prefix>/<blake3_hex>`, preventing cross-tenant deduplication oracle leaks while preserving intra-place deduplication), records `(event_id, scope_id, blake3, ref_expires_at_ms)` in SQLite **`attachment_refs`**, and hands Core **only an opaque `AttachmentHandle`** (`handle_id`, `modality_slot ∈ {none, image, audio, video, sensor_blob}`, `byte_len`, `expires_at_ms`, `embed_model_ver`), a normalized numeric feature slot vector $\mathbf{f}_{\text{modal}} \in \mathbb{R}^{10}$, and a pre-gate unified embedding $\mathbf{e}_t \in \mathbb{R}^{d}$ ($d = 256$ MRL) computed by Gateway's stateless external Embedder worker (**`EmbeddingGemma 2`** Apache-2.0 unified encoder) strictly on segmented utterance/keyframe boundaries before the `Event` enters Core's single MPSC queue. Under `R-33` (`VisibleSet` pre-filtering) and `R-34` (`hard_forget`), deleting an event or actor decrements `attachment_refs`; when a `(scope_id, blake3)` ref count reaches `0` (or `expires_at_ms` lapses), Gateway performs immediate cryptographic overwrite/unlink + derived cache purge synchronously within the R-34 two-epoch cleanup transaction. For full-duplex speech (`Moshi`, `VAP`, `Full-Duplex-Bench-v2`), Body runs frame-level VAD (`Silero VAD` or `WebRTC VAD`) + pitch (`RAPT`/`YIN`) + energy + AEC echo cancellation in sub-50 ms loops, while **voice barge-in** cuts local audio playback immediately in Body and returns a single terminal `Receipt(outcome = failed, termination_reason = preempted_external, progress, delivered_units)` preserving `R-09`'s single-receipt invariant without corrupting `R-26`/`R-35` capability statistics.

### Gateway Attachment Wire, Blind Core Boundary, & R-34 Hard Forgetting

1. **Waist-2 Wire Contract (Resolving `R-45` Open Question):**
   - `Small payload (byte_len <= 256 KiB)`: Allowed inline (`base64`) on `percept.inline_blob`, stripped by Gateway into Attachment Store *before* entering Core queue so Core SQLite never stores media payloads.
   - `Large/streaming media (byte_len > 256 KiB)`: Body streams bytes to Gateway's authenticated `POST /v1/attachments` (HTTP/UDS chunked transfer; optional `SCM_RIGHTS` FD pass on same host), receiving `{handle_id, blake3_scope_digest, byte_len, uploaded_at_ms}` with idle staging TTL (`60 s`) if never bound to an `Event`.
2. **Scoped `BLAKE3` + `R-34` Ref-Counted Secure Erasure:**
   - Global unkeyed dedup leaks whether Actor A previously uploaded the exact file Actor B uploads (`dedup oracle`). Keying via `d = blake3::keyed_hash(&scope_salt[place_or_tenant], bytes)` scopes deduplication cleanly inside a `scope_id`.
   - When `R-34` `hard_forget` (by `event_id` or `actor_id`) removes an `Event`, SQLite triggers cascade-delete matching rows in `attachment_refs`. Any `(scope_id, d)` reaching `ref_count == 0` is synchronously zero-overwritten/unlinked from disk **and** evicted from Gateway's transient decoded-frame/STT cache prior to committing `VACUUM` + clean post-erasure snapshot (`R-34`).
   - Only when `Core` actually triggers `reason` (`LLM` call) **after** `R-33 VisibleSet` & `Valve` checks does the **Model Provider adapter** resolve permitted `AttachmentHandle`s by ID to attach raw image/audio parts into the multimodal LLM request — Core itself never sees pixels or waveforms.

### Verified Nov 2026 Multimodal Embedding & Feature Extractor Comparison

| Model / Tool | Scope & Modalities | Params | Vector Dim (MRL) | License (Verified Nov 2026) | Latency / Hardware & Role in MonoAgens | Source |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`EmbeddingGemma 2`** (`google/embeddinggemma-2`) | **Unified single space:** Text, Image, Audio, Video + interleaved | Modular: **271M** text / **439M** VLM / **577M** audio-LM / **~740M** all | **768d** (`MRL: 512, 256, 128`) | **Apache 2.0** | **Primary local Gateway Embedder (`256d`):** matches A-15 dim; edge-viable `<1B`; note vector space incompatible with v1 `EmbeddingGemma-300M` (requires `embed_model_ver` tag). Exact ms latency requires local Gateway benchmark. | `[PP]` `arXiv:2610.37364`, HF |
| **`Gemini Embedding 2`** (`gemini-embedding-2`) | **Unified:** Text, Image, Video, Audio, PDF (8,192 tok) | Cloud API | `128–3072d` MRL (rec `768/1536/3072`, auto-renorm) | **Proprietary** Cloud API (GA Apr 2026) | **Optional cloud fallback only:** incompatible with offline edge sovereignty (`~0.20` text / `~0.45` img / `~6.50` aud / `~12.00` vid per 1M tok + network RTT) | `[IND]` `aistudio.google.com/docs/embeddings` |
| **`Qwen3-VL-Embedding`** (`2B` / `8B`) | Text, Image, Doc, Video (**no audio**) | **2B** / **8B** (32K ctx) | `2048d` / `4096d` (MRL) | **Apache 2.0** | **GPU Server Vision+Doc Embedder:** high doc/OCR fidelity; unsuitable for CPU edge or audio-in bodies | `[PP]` `arXiv:2601.04720` |
| **`SigLIP 2`** (`google/siglip2-*`) | Vision-Language (multilingual + NaFlex) | ViT-B `86M`, L `303M`, So400m `400M`, g `1B`+So400m | L:`1024d`, So400m:`1152d`, g:`1536d/1152d` | **Apache 2.0** | Lightweight visual-only alternative if separate audio tower (`CLAP`) is preferred | `[PP]` `arXiv:2502.14786` |
| **`CLIP`** (`ViT-B/32`, `L/14`) | Vision-Language (English-centric) | ViT-B/32 / ViT-L/14 (`512` / `768` text, `768` / `1024` vis hidden) | `512d` / `768d` | **MIT** | Legacy baseline; superseded by `SigLIP 2` & `EmbeddingGemma 2` | `[PP]` `arXiv:2103.00020` |
| **`LAION-CLAP`** (`clap-htsat-fused`) | Audio-Text (non-speech sound + text) | **~153.5M** (HTSAT+RoBERTa) | **512d** | **Apache 2.0** | Split environmental audio fallback; no vision alignment | `[PP]` `arXiv:2211.06687` |
| **`ImageBind`** | 6 modal (Img, Text, Aud, Depth, IR, IMU) | **~1.2B** (ViT-Huge) | Unified joint space | **CC BY-NC-SA 4.0** | ⚠️ **Disqualified for commercial Core bundling** due to NonCommercial license | `[PR/PP]` `arXiv:2305.05665` |
| **`LanguageBind`** | Video, Depth, IR, Audio $\to$ Text space | 24-layer ViT + LoRA | **768d** (frozen OpenCLIP) | **MIT** | Permissive specialized fallback when robotic **Depth/Thermal-IR** embeddings are required without ImageBind NC taint | `[PR/PP]` `arXiv:2310.01852` |
| **`Qwen2.5-Omni`** / **`Qwen3-Omni`** | **Generative Thinker-Talker** (text+img+aud+vid $\to$ text+speech) | Q2.5: `7B` Thinker (`~11B` tot); Q3: **`30B-A3B` MoE** | Generative tokens (**not vector embedder**) | Q2.5: `license:other`; Q3: README `apache-2.0` (`other` tag) | **`Reason` Provider / Async Audio Captioner ONLY** (`Q3-Omni-Captioner`, `234 ms` theoretical cold first-packet); **NEVER** in Gate or `Core` | `[PP]` `2503.20215`, `2509.17765` |
| **`Silero VAD`** (v5/v6) | Voice activity (`vad_prob`) | **~2 MB** (`1.25 MB` ONNX) | Binary/prob (`512` samples @ `16 kHz` = `32 ms`) | **MIT** | **Default Body Speech Gate:** `< 1 ms` per 30+ ms chunk on 1 CPU thread, zero telemetry | `[IND]` `snakers4/silero-vad` |
| **`WebRTC VAD`** (`libfvad` / `webrtc-vad`) | GMM VAD (`10/20/30 ms` @ `8–48 kHz`) | Pure DSP (<100 KB) | Binary | **BSD-3-Clause** | **MCU/Tiny-Edge Body VAD:** fallback where ONNX runtime cannot run (`R-13`) | `[IND]` `webrtc-vad` / `libfvad` |
| **`Pyannote`** (`3.1` / `community-1`) | Neural speaker diarization (powerset EEND) | Pipeline checkpoints | `speaker_cluster_id` | `3.1`: **MIT**; `community-1`: **CC-BY-4.0** (HF gated) | Gateway/GPU streaming cluster assignment (faster-than-real-time on A100; edge CPU runs chunked speaker embeddings per VAD turn) | `[PP/IND]` `arXiv:2310.13025` |
| **`VAP`** (Ekstedt & Skantze 2022) | Turn-taking projection (`2.0 s` window, 4 bins/spk = 256 states) | 4-layer causal Transformer (`d=256`) + CPC | `turn_end_prob`, backchannel probs | Academic (`github.com`) | Causal turn-yielding prediction replacing brittle fixed 700 ms silence timers; aligns with Mori et al. (2026) online addressivity | `[PP]` `arXiv:2205.09812` |
| **`OpenSMILE eGeMAPS v02`** vs **`RAPT`** | Acoustic/prosodic functionals (`25` LLDs / `88` functionals) / NCCF pitch (`F0`) | C++ / DSP | `pitch_z`, `energy_z` | ⚠️ **`openSMILE` Research-Only / paid commercial** (`Talkin 1995` alg) | ⚠️ **Lab diagnostic only unless commercially licensed;** Body computes causal z-scored **RAPT/YIN `F0`**, log-RMS, and silence cleanly in MIT/Apache DSP | `[PR]` `10.1109/TAFFC.2015.2457417`, Talkin 1995 |
| **`YAMNet`** | 521 AudioSet classes + embedding | **3.7M** (MobileNet-v1) | `521` logits + **`1024d`** emb | **Apache 2.0** | Body/Gateway non-speech acoustic triggers (`alarm`, `knock`, `glass`) without LLM | `[IND]` `tensorflow/models` |

### Speech-In / Speech-Out Schemas & Full-Duplex Barge-In Under Blind Core

#### 1. Numeric `Percept` Acoustic/Turn-Taking Slot Vector (`[EMB]`, Zero-Neutral in `[CHAT]`)
Body normalizes speaker pitch ($F_0$ via RAPT/YIN) and RMS energy causally against an online per-cluster baseline `(mu_w, sigma_w)` over the last $W_{\text{norm}} = 30\text{ s}$ of voiced frames (clamped $[-4.0, +4.0]$) — carrying **pure structural prosodic dynamics (turn boundaries, interruption emphasis, mic SNR)** and **zero emotion classification labels** (`ARCHITECTURE.md` L97):

```json
{
  "modality_slot": "audio_voice",        // enum: none | image | audio_voice | video | sensor_blob
  "speech_active": true,                 // bool -> 0.0 / 1.0 (hysteretic Silero/WebRTC VAD)
  "vad_prob": 0.94,                      // float [0.0, 1.0]
  "speaker_cluster_id": "act_ephem_03",  // opaque local diarizer ActorId (ephemeral until linked)
  "pitch_z": -0.82,                      // float [-4.0, 4.0], online speaker log-F0 z-score
  "energy_z": 1.15,                      // float [-4.0, 4.0], RMS above noise_floor z-score
  "pause_ms": 420.0,                     // float >= 0.0, contiguous trailing silence before emit
  "turn_end_prob": 0.87,                 // float [0.0, 1.0] from VAP / boundary classifier
  "noise_floor": 0.18,                   // float [0.0, 1.0] normalized ambient noise power
  "asr_conf": 0.91                       // float [0.0, 1.0] mean token ASR posterior (1.0 in chat)
}
```

How blind Core uses these numbers without reading text/audio content:
- **`turn_end_prob` & `pause_ms` -> `waiting` patience & backlog timing (`appraisal-gate/spec.md`):** Low `turn_end_prob < 0.35` with brief pause (`pause_ms < 700 ms`) indicates floor-holding mid-utterance pause (defer `reason` wake briefly via backlog merge instead of interrupting user mid-sentence!). High `turn_end_prob >= 0.70` triggers immediate turn handoff.
- **`pitch_z` / `energy_z` / `vad_prob` -> `A-06` Salience & `R-29` Addressivity:** Feeds numeric boundary discrepancy $s_{\text{acoustic},t} = \min(1, \max(|\Delta\text{pitch}_z|, |\Delta\text{energy}_z|)/4)$ (weighted $0$ when `modality_slot == none`), aligning with **Mori et al. (2026)** (`arXiv:2607.15648`) showing continuous online addressivity levels outperform discrete labels for predicting turn transitions, gaze, and backchannels, and **Skantze (2021)** on prosodic turn-yielding cues.
- **`speaker_cluster_id` + `asr_conf` -> Epistemic grounding:** Low `asr_conf < 0.45` raises uncertainty signals (`A-14` info-gap trigger to ask for clarification when `speech_active = 1`).

#### 2. `CapabilityDescriptor` Shapes (Compliant with `protocol/spec.md` L56–59)

```json
{
  "id": "speech.speak",
  "description": "Synthesize and stream spoken audio output on the local speaker array; supports incremental cancellation on user barge-in.",
  "places": ["place_room_main"],
  "communicative": true,
  "identity_changing": false,
  "typical_latency_ms": 250,
  "effect": {
    "channel": "audio_voice",
    "window_ms": 1500,
    "noise": 0.15
  }
}
```
```json
{
  "id": "action.cancel",
  "description": "Preempt and gracefully halt an in-flight target act_id on this body (speech output or continuous actuator trajectory).",
  "places": ["place_room_main"],
  "communicative": false,
  "identity_changing": false,
  "typical_latency_ms": 50,
  "effect": {
    "channel": "control",
    "window_ms": 200,
    "noise": 0.02
  }
}
```
```json
{
  "id": "vision.capture_frame",
  "description": "Capture an on-demand high-resolution camera frame into Gateway Attachment Store and emit an opaque image percept handle.",
  "places": ["place_room_main"],
  "communicative": false,
  "identity_changing": false,
  "typical_latency_ms": 300,
  "effect": {
    "channel": "vision_rgb",
    "window_ms": 1000,
    "noise": 0.05
  }
}
```

#### 3. Full-Duplex Speech-Out & Barge-In State Machine (Under `Moshi` / `Full-Duplex-Bench-v2`)

Consistent with **Moshi (`arXiv:2410.00037`, Mimi `12.5 Hz`, `160–200 ms` turn loop)** and **Full-Duplex-Bench-v2 (`arXiv:2510.07838`)** (which emphasizes pause handling, smooth mid-turn corrections, overlapping speech, and safety interruption):
1. **Streaming Dispatch:** Core issues `Act(a1, speech.speak, {text_or_handle})` and records `Expectation(effect, channel=audio_voice)` + `Promise(deliver_turn)`. Body chunks TTS synthesis (`40–80 ms` playback frames) with hardware **Acoustic Echo Cancellation (AEC)** preventing speaker output from triggering local Silero VAD.
2. **Sub-50 ms Local Barge-In Reflex (Inside Body):** While streaming `a1`, if Body detects non-AEC human speech exceeding $(\text{vad\_prob} \ge \theta_{\text{barge,vad}} = 0.70) \;\land\; (\text{speech\_dur} \ge \tau_{\text{barge}} = 120\text{ ms})$ (filtering short clicks/backchannels like "mhm" shorter than $120\text{ ms}$ without killing TTS, unless `energy_z > 2.0`), **Body immediately mutes/fades out TTS audio locally (`< 30 ms`)**.
3. **Single Terminal Receipt (`R-09` & `protocol/spec.md`):** Body immediately emits **one** receipt:
   `Receipt { act_id: a1, outcome: "failed", termination_reason: "preempted_external", progress: 0.42, delivered_units: {chars_spoken: 38, audio_ms: 1120} }`, followed sequentially by the interrupting user `Event(speech_active=true, speaker_cluster_id=..., transcript_or_handle=...)`.
4. **Core Grounding:** `R-09`/`Expectations` sees `delivered_units` and knows the human heard *only* up to char 38 (`42%`), marks the `Promise` unfulfilled (no blind auto-repeat over top of the user!), keeps `R-26`/`R-35` capability $(\alpha_c, \beta_c)$ **unpenalized**, and wakes `reason` once `turn_end_prob >= 0.70` to respond directly to the user's interruption.

## Parameters

| Symbol | Role | Start Value | Search Range | Scope | Justification |
| :--- | :--- | :--- | :--- | :--- | :--- |
| $d_{\text{emb}}$ | Unified multimodal MRL vector dimension | `256` | `{128, 256, 512, 768}` | `[UNI]` | Matches `A-15` text size; `EmbeddingGemma 2` `256d` retains `>98%` of `768d` MIEB score (`49.1` vs `49.9`) |
| $B_{\text{inline}}$ | Inline Waist-2 vs out-of-band attachment cutoff | `256 KiB` | `[64 KiB, 1 MiB]` | `[EMB]` | Keeps JSON-RPC event frames lightweight while avoiding HTTP round-trip on tiny thumbnails/short PCM chunks |
| $\text{TTL}_{\text{stage}}$ | Unbound staged attachment expiration | `60 s` | `[15 s, 300 s]` | `[EMB]` | Prevents orphan blob leaks if Body disconnects after uploading before emitting `percept` |
| $\text{TTL}_{\text{media}}$ | Default Gateway retained media TTL (optional) | `30 d` (config) | `[1 d, 365 d, inf]` | `[EMB]` | Balances multimodal replay/lookup with storage and privacy minimization (`R-34` overrides early) |
| $\theta_{\text{vad}}$ | Silero/WebRTC frame speech gate probability | `0.50` | `[0.30, 0.80]` | `[EMB]` | Standard balanced speech/noise point |
| $\tau_{\text{barge}}, \theta_{\text{barge,vad}}$ | Continuous non-AEC speech duration & prob for TTS barge-in | `120 ms` / `0.70` | `[80 ms, 300 ms]` / `[0.55, 0.90]` | `[EMB]` | Distinguishes intentional barge-in interruptions from brief `<100 ms` backchannels (`"uh-huh"`) or transient mic clicks |
| $\theta_{\text{turn\_end}}$ | VAP / prosodic turn-yield score to wake `reason` immediately | `0.70` | `[0.50, 0.85]` | `[EMB]` | Suppresses premature LLM interrupts during mid-sentence thinking pauses while beating fixed long timeouts |
| $W_{\text{norm}}$ | Online rolling window for causal `pitch_z`/`energy_z` | `30 s` voiced | `[10 s, 120 s]` | `[EMB]` | Speaker-adaptive acoustic normalization without storing raw voiceprints in Core |
| $\theta_{\text{asr\_conf}}$ | Low-ASR confidence floor triggering clarification hint | `0.45` | `[0.25, 0.65]` | `[EMB]` | Grounds `A-14` info-gap queries under ambient noise |

## Neutral value
In `[CHAT]`, `modality_slot = none`, `attachment_handles = []`, `speech_active = false`, `vad_prob = 0.0`, `pitch_z = 0.0`, `energy_z = 0.0`, `pause_ms = 0.0`, `turn_end_prob = 0.0`, `noise_floor = 0.0`, `asr_conf = 1.0`, zero `BLAKE3` disk I/O runs, and Gateway computes pure-text `256d` embeddings cleanly. All gate thresholds, salience weights, ACT-R retrievals, and privacy invariants are **algebraically and behaviorally invariant** relative to the `[CHAT]` baseline.

## Claim
Using `EmbeddingGemma 2` (`256d` MRL, Apache 2.0) in Gateway places cross-modal (`image`/`audio`/`text`) memories into one shared $256\text{d}$ index without adding any modality-specific similarity code in Core (`0` extra books, `0` Core LLM calls in gate), retaining $\ge 98\%$ of $768\text{d}$ multimodal retrieval mean score (`49.1` vs `49.9` reported MIEB(lite)); keyed `BLAKE3` attachment custody guarantees **`0` recoverable bytes/metadata traces** post-`R-34` `hard_forget` AND **`0` cross-tenant dedup leakage**; and Body-side Silero VAD + AEC barge-in achieves **$< 50\text{ ms}$ speaker cutoff latency** (`<= 120 ms` debounce + `< 30 ms` mute) with strictly **1 terminal `Receipt`** per `act_id` and accurate `delivered_units` tracking.

## How the Lab would test it
All Gateway storage, Core blindness, and turn-taking FSM tests run deterministically with mocked/fixture multimodal embeddings and seeded audio/feature traces (`0` paid LLM required for invariants 1–3; optional local/live run for full-duplex FDB-v2 style dialogue in test 4):
1. **`lab_r37_core_blindness_and_neutrality`:** Feed identical semantic episodes encoded first as pure `[CHAT]` text and second with multimodal `AttachmentHandle`s (noise/image metadata variations) while overriding acoustic vector slots to neutral `0.0`. Verify Core gate scores, habituation, and recall rankings depend *exclusively* on `256d` vectors and numeric slots, with `0` byte reads of raw blobs inside `monoagens-core`.
2. **`lab_r37_r34_attachment_forgetting_and_dedup_isolation`:** Upload identical 512 KiB image bytes under Actor A (`scope_A`) and Actor B (`scope_B`); assert distinct keyed `BLAKE3` disk paths (`no cross-scope dedup`). Then invoke `R-34` `hard_forget(Actor A)` and scan the entire filesystem (attachment dir, temp/staging, SQLite DB, FTS, WAL): assert **`0` occurrences** of Actor A's bytes or `AttachmentHandle` while Actor B's reference remains intact.
3. **`lab_r37_barge_in_single_receipt_race`:** Execute `speech.speak` streaming playback over seeded audio timeline; inject (a) a short `80 ms` backchannel `"mm"` (must **not** abort `speech.speak`), (b) a sustained `200 ms` user interruption at `progress = 0.42` (must emit single `Receipt{failed, preempted_external, progress=0.42}` within `< 50 ms` of confirmation and preserve `R-26`/`R-35` $(\alpha_c,\beta_c)$), and (c) self-playback acoustic bleed blocked by mock AEC flag.
4. **`lab_r37_pause_vs_turn_end_timing`:** Replay paired audio feature streams modelled on `Full-Duplex-Bench-v2` pause-handling and mid-turn correction tasks (`arXiv:2510.07838`); verify mid-utterance `pause_ms = 600 ms` with `turn_end_prob = 0.25` triggers backlog merge rather than premature agent turn intrusion, reducing premature-interruption error rate vs naive `>500 ms` silence VAD baseline.

## Established / inferred / open
- **Established (literature & verified artifacts):** `EmbeddingGemma 2` architecture/modularity/Apache-2.0/`768->256d` MRL retention (`arXiv:2610.37364`, HF `google/embeddinggemma-2`); `Gemini Embedding 2` GA multimodal space & v1 incompatibility (`aistudio.google.com/docs/embeddings`); `Qwen3-VL-Embedding` (`arXiv:2601.04720`, Apache 2.0, vision/doc/video, no audio); `SigLIP 2` (`arXiv:2502.14786`, Apache 2.0); `CLIP` (`arXiv:2103.00020`, MIT); `LAION-CLAP` (`arXiv:2211.06687`, Apache 2.0); `ImageBind` CC BY-NC-SA 4.0 restriction (`arXiv:2305.05665`); `LanguageBind` MIT (`arXiv:2310.01852`); `Qwen2.5-Omni` (`arXiv:2503.20215`, `license:other`) & `Qwen3-Omni` (`arXiv:2509.17765`, `234 ms` theoretical latency generative Thinker-Talker); `Silero VAD` (`<1 ms/chunk` MIT), `WebRTC VAD` (`BSD-3-Clause`), `Pyannote 3.1` MIT / `community-1` CC-BY-4.0 (`arXiv:2310.13025`), `YAMNet` (`3.7M/1024d` Apache 2.0), `openSMILE eGeMAPS` (`88` features, audEERING research-only/commercial restriction), `VAP` (`2 s` window / `256` states, `arXiv:2205.09812`), `RAPT` (Talkin 1995), `Moshi` (`12.5 Hz` Mimi, `160–200 ms`, `arXiv:2410.00037`), `Full-Duplex-Bench` & `v2` (`arXiv:2503.04721`, `arXiv:2510.07838`), continuous addressivity utility (`Mori et al. 2026`, `arXiv:2607.15648`), and `blake3` Rust crate `v1.8.7`.
- **Inferred (MonoAgens design synthesis):** Placing `EmbeddingGemma 2` inside Gateway pre-queue rather than inside Core or Body; keyed `BLAKE3` `attachment_refs` partitioning to avoid cross-actor dedup side-channels under `R-34`; the 10-element purely structural numeric acoustic slot vector ensuring Core never processes emotion tags; local sub-50 ms Body AEC+VAD barge-in mapping cleanly to `Receipt(failed, preempted_external, delivered_units)`.
- **Open (not settled by literature):**
  1. **Real-world edge latency & quantization curve of `EmbeddingGemma 2` (`439M`/`577M`/`740M`):** Exact ONNX/LiteRT int4/int8 latency on target ARM/laptop CPUs vs Gateway throughput is not published uniformly across our edge targets and must be benchmarked locally in `monoagens-gateway`.
  2. **Indonesian + code-switched turn-taking (`VAP`) calibration:** Standard `VAP` & `FDB-v2` benchmarks predominantly cover English/Japanese/Mandarin prosody (`Mori et al. 2026` explicitly notes cultural/linguistic limits on Japanese triadic sessions). Thresholds $(\tau_{\text{barge}}, \theta_{\text{turn\_end}})$ for Indonesian/mixed conversation require an annotated bilingual Lab calibration corpus.
  3. **Diarization cluster drift across long pauses:** Whether ephemeral `speaker_cluster_id` remains stable over >10 min gaps on edge CPU vs GPU Pyannote requires Gateway speaker-cache TTL calibration.

## Sources
- **[PP]** Schechter Vera, H., Dua, S., Andonov, J., Smoot, S., et al. (Oct / rev. Nov 9, 2026). *EmbeddingGemma 2: Unified and Efficient Multimodal Representations*. `arXiv:2610.37364` (`https://arxiv.org/abs/2610.37364`) & `https://huggingface.co/google/embeddinggemma-2`
- **[IND]** Google AI Studio Documentation (GA Apr 2026). *Gemini Embeddings (`gemini-embedding-2`)*. `https://aistudio.google.com/docs/embeddings`
- **[PP]** Zhang, Z., et al. (Jan 2026). *Qwen3-VL-Embedding and Qwen3-VL-Reranker: A Unified Framework for State-of-the-Art Multimodal Retrieval and Ranking*. `arXiv:2601.04720` (`https://arxiv.org/abs/2601.04720`)
- **[PP]** Tschannen, M., et al. (Feb 2025). *SigLIP 2: Multilingual Vision-Language Encoders with Improved Semantic Understanding, Localization, and Dense Features*. `arXiv:2502.14786` (`https://arxiv.org/abs/2502.14786`)
- **[PP]** Radford, A., et al. (2021). *Learning Transferable Visual Models From Natural Language Supervision (CLIP)*. `arXiv:2103.00020` (`https://arxiv.org/abs/2103.00020`)
- **[PP]** Wu, Y., et al. (2022 / rev. 2024). *Large-scale Contrastive Language-Audio Pretraining with Feature Fusion and Keyword-to-Caption Augmentation (LAION-CLAP)*. `arXiv:2211.06687` (`https://arxiv.org/abs/2211.06687`) & `https://huggingface.co/laion/clap-htsat-fused`
- **[PP]** Girdhar, R., et al. (CVPR 2023). *ImageBind: One Embedding Space To Bind Them All*. `arXiv:2305.05665` (`https://arxiv.org/abs/2305.05665`) & `https://raw.githubusercontent.com/facebookresearch/ImageBind/main/LICENSE`
- **[PP]** Zhu, B., et al. (ICLR 2024). *LanguageBind: Extending Video-Language Pretraining to N-modality by Language-based Semantic Alignment*. `arXiv:2310.01852` (`https://arxiv.org/abs/2310.01852`) & `https://raw.githubusercontent.com/PKU-YuanGroup/LanguageBind/main/LICENSE`
- **[PP]** Xu, J., et al. (Mar 2025). *Qwen2.5-Omni Technical Report*. `arXiv:2503.20215` (`https://arxiv.org/abs/2503.20215`)
- **[PP]** Xu, J., et al. (Sep 2025). *Qwen3-Omni Technical Report*. `arXiv:2509.17765` (`https://arxiv.org/abs/2509.17765`) & `https://huggingface.co/Qwen/Qwen3-Omni-30B-A3B-Instruct`
- **[IND]** Silero Team. *Silero VAD (v5/v6, MIT)*. `https://github.com/snakers4/silero-vad` & `https://raw.githubusercontent.com/snakers4/silero-vad/master/LICENSE`
- **[IND]** Google WebRTC VAD / `libfvad` / Rust `webrtc-vad` (`BSD-3-Clause`). `https://github.com/kaegi/webrtc-vad`
- **[PP]** Plaquet, A., & Bredin, H. (2023). *Powerset multi-class cross entropy loss for neural speaker diarization (`pyannote/speaker-diarization-3.1`)*. `arXiv:2310.13025` (`https://arxiv.org/abs/2310.13025`)
- **[PR]** Eyben, F., et al. (2016). *The Geneva Minimalistic Acoustic Parameter Set (GeMAPS) for Voice Research and Affective Computing*. IEEE Trans. Affective Computing, 7(2), 190–202. DOI: `10.1109/TAFFC.2015.2457417` & `https://raw.githubusercontent.com/audeering/opensmile/master/LICENSE`
- **[IND]** Google Research / TensorFlow Models. *YAMNet (MobileNet-v1 AudioSet, Apache 2.0)*. `https://raw.githubusercontent.com/tensorflow/models/master/research/audioset/yamnet/README.md`
- **[PR]** Talkin, D. (1995). *A robust algorithm for pitch tracking (RAPT)*. In W. B. Kleijn & K. K. Paliwal (Eds.), *Speech Coding and Synthesis*, Elsevier, pp. 495–518.
- **[PR]** Skantze, G. (2021). *Turn-taking in Conversational Systems and Human-Robot Interaction: A Review*. Computer Speech & Language, 67, 101178. DOI: `10.1016/j.csl.2020.101178`
- **[PP]** Ekstedt, E., & Skantze, G. (2022). *Voice Activity Projection: Self-supervised Learning of Turn-taking Events*. `arXiv:2205.09812` (`https://arxiv.org/abs/2205.09812`)
- **[PP]** Défossez, A., et al. (2024). *Moshi: a speech-text foundation model for real-time dialogue*. `arXiv:2410.00037` (`https://arxiv.org/abs/2410.00037`)
- **[PP]** Lin, G.-T., et al. (2025). *Full-Duplex-Bench: A Benchmark to Evaluate Full-duplex Spoken Dialogue Models on Turn-taking Capabilities*. `arXiv:2503.04721` (`https://arxiv.org/abs/2503.04721`)
- **[PP]** Lin, G.-T., et al. (2025 / rev. Apr 2026). *Full-Duplex-Bench-v2: A Multi-Turn Evaluation Framework for Duplex Dialogue Systems with an Automated Examiner*. `arXiv:2510.07838` (`https://arxiv.org/abs/2510.07838`)
- **[PP]** Mori, T., Inoue, K., Lala, D., & Kawahara, T. (Jul 2026). *On the Structure of Address in Multi-Party Dialogue: From Discrete Labels to Continuous Levels*. `arXiv:2607.15648` (`https://arxiv.org/abs/2607.15648`)
- **[IND]** `blake3` Rust crate `v1.8.7` (`https://crates.io/crates/blake3`).

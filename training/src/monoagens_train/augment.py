"""Procedural group-chat timing & V2.2 curricula (habituation, patience, pecah balon, code-switch, self-repair, peer-agent).

Generates general, randomized multi-turn sequences (100% synthetic, OSI-clean, zero sealed/ access)
teaching universal multi-party timing & calibration laws:
  - Regime 0: Rapid-repeat habituation in group rooms -> Silent (class 0)
  - Regime 1: Open-floor loose soul mention in multi-party rooms -> Wait (class 2) + bounded wait_delta_ms
  - Regime 2: Unaddressed group chatter -> Silent (class 0)
  - Regime 3: Direct address / dyadic / reply-link -> Go (class 3)
  - Regime 4 (schema=2): *Pecah balon* multi-bubble burst patience (2-5 rapid bursts at 0.3-3.0s) -> Wait -> resolved Go/Silent
  - Regime 5 (schema=2): Spherical code-switch drift (15-30 deg angular noise) + `max_visible_mem_cos` attenuation (0.70-0.85)
  - Regime 6 (schema=2): Multi-party self-repair (`dev-10`) & implicit user correction (`p_user_correction = 1.0`, `Deep`)
  - Regime 7 (schema=2): Peer-agent coordination (`dev-13`: anti-ping-pong `Silent`, expertise deferral `Wait`/`Silent`, own-expertise `Go`)

Includes both vectorless (`e_256 = 0`) and normalized embeddings so the controller stays
calibrated even when no embedding sidecar is attached.
"""

from __future__ import annotations

import math

import numpy as np

from monoagens_train.features import (
    NUM_FEATURES,
    NUM_FEATURES_V2,
    PerceptNumericInputs,
    PerceptNumericInputsV2,
    build_x_num,
    build_x_num_v2,
)

SEQ_LEN_DEFAULT = 24
EMB_DIM = 256

COMPUTE_TIER_FAST = 0
COMPUTE_TIER_STANDARD = 1
COMPUTE_TIER_DEEP = 2


def _sample_unit_embedding(rng: np.random.Generator) -> np.ndarray:
    vec = rng.normal(0.0, 1.0, size=EMB_DIM).astype(np.float32)
    norm = float(np.linalg.norm(vec))
    return vec / max(norm, 1e-6)


def _sample_embedding(rng: np.random.Generator, vectorless_prob: float) -> np.ndarray:
    if rng.random() < vectorless_prob:
        return np.zeros(EMB_DIM, dtype=np.float32)
    return _sample_unit_embedding(rng)


def _rotate_on_sphere(vec: np.ndarray, angle_deg: float, rng: np.random.Generator) -> np.ndarray:
    """Applies exact spherical angular perturbation of `angle_deg` degrees to unit vector `vec`."""
    norm = float(np.linalg.norm(vec))
    if norm < 1e-6:
        return _sample_unit_embedding(rng)
    u = (vec / norm).astype(np.float32)
    g = rng.normal(0.0, 1.0, size=EMB_DIM).astype(np.float32)
    ortho = g - float(np.dot(g, u)) * u
    ortho_norm = float(np.linalg.norm(ortho))
    if ortho_norm < 1e-6:
        return u
    v_hat = ortho / ortho_norm
    rad = math.radians(angle_deg)
    rotated = math.cos(rad) * u + math.sin(rad) * v_hat
    return (rotated / max(float(np.linalg.norm(rotated)), 1e-6)).astype(np.float32)


def _build_features(inp: PerceptNumericInputsV2, schema_version: int) -> np.ndarray:
    if schema_version == 2:
        return build_x_num_v2(inp)
    return build_x_num(inp)


def _generate_split(
    n_sequences: int,
    seq_len: int,
    rng: np.random.Generator,
    schema_version: int = 1,
) -> dict[str, np.ndarray]:
    if schema_version not in (1, 2):
        raise ValueError(f"Unsupported schema_version: {schema_version}")
    n_feat = NUM_FEATURES_V2 if schema_version == 2 else NUM_FEATURES
    n_regimes = 8 if schema_version == 2 else 4

    e_256 = np.zeros((n_sequences, seq_len, EMB_DIM), dtype=np.float32)
    x_num = np.zeros((n_sequences, seq_len, n_feat), dtype=np.float32)
    y_gate = np.zeros((n_sequences, seq_len), dtype=np.int64)
    y_wait_ms = np.zeros((n_sequences, seq_len), dtype=np.float32)
    mask = np.zeros((n_sequences, seq_len), dtype=np.float32)
    y_compute_tier = np.full((n_sequences, seq_len), COMPUTE_TIER_STANDARD, dtype=np.int64)
    y_user_correction = np.zeros((n_sequences, seq_len), dtype=np.float32)
    v22_mask = np.zeros((n_sequences, seq_len), dtype=np.float32)
    zeros_f = np.zeros((n_sequences, seq_len), dtype=np.float32)
    ds_label = "procedural_timing_v22" if schema_version == 2 else "procedural_timing"
    dataset_id = np.full((n_sequences,), ds_label, dtype="<U24")

    for i in range(n_sequences):
        regime = i % n_regimes
        vec_drop = 0.65 if schema_version == 2 else 0.70
        n_participants = int(rng.integers(3, 18))

        if regime == 0:
            # Regime 0: Rapid-repeat habituation in multi-party room (dev-07)
            n_repeats = int(rng.integers(3, min(8, max(4, seq_len - 2))))
            sender_ex = 0
            place_cnt = 1
            soul_acts = 0
            total_acts = 1

            # Turn 0: casual chatter -> Silent
            e_256[i, 0] = _sample_embedding(rng, vec_drop)
            x_num[i, 0] = _build_features(
                PerceptNumericInputsV2(
                    is_named_soul=False,
                    is_explicit_addressee=False,
                    is_reply_or_expect_link=False,
                    is_dyadic_place=False,
                    active_participant_count=n_participants,
                    dt_since_last_place_event_s=float(rng.uniform(10.0, 600.0)),
                    dt_since_last_soul_act_s=86400.0,
                    dt_since_sender_last_act_s=86400.0,
                    soul_acts_window=0,
                    total_acts_window=1,
                    sender_exchange_count=0,
                    place_event_count=place_cnt,
                    active_thread_count=int(rng.integers(1, 4)),
                    sender_reliability=float(rng.uniform(0.4, 0.85)),
                ),
                schema_version,
            )
            y_gate[i, 0] = 0
            y_compute_tier[i, 0] = COMPUTE_TIER_FAST
            y_user_correction[i, 0] = 0.0
            mask[i, 0] = 1.0
            v22_mask[i, 0] = 1.0

            # Turn 1: direct address -> Go
            if seq_len > 1:
                place_cnt += 1
                total_acts += 1
                sender_ex += 1
                e_256[i, 1] = _sample_embedding(rng, vec_drop)
                x_num[i, 1] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=True,
                        is_explicit_addressee=True,
                        is_reply_or_expect_link=False,
                        is_dyadic_place=False,
                        active_participant_count=n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(0.3, 3.0)),
                        dt_since_last_soul_act_s=86400.0,
                        dt_since_sender_last_act_s=86400.0,
                        soul_acts_window=0,
                        total_acts_window=total_acts,
                        sender_exchange_count=sender_ex,
                        place_event_count=place_cnt,
                        max_active_thread_cos=float(rng.uniform(0.4, 0.85)),
                        active_thread_count=int(rng.integers(1, 4)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.5, 0.9)),
                    ),
                    schema_version,
                )
                y_gate[i, 1] = 3
                y_compute_tier[i, 1] = COMPUTE_TIER_STANDARD
                y_user_correction[i, 1] = 0.0
                mask[i, 1] = 1.0
                v22_mask[i, 1] = 1.0

            # Soul responded right after turn 1; subsequent rapid repeats -> Silent (habituated)
            soul_acts = 1
            total_acts += 1
            dt_soul = float(rng.uniform(1.0, 6.0))
            dt_sender = dt_soul

            for r in range(n_repeats):
                t = 2 + r
                if t >= seq_len:
                    break
                place_cnt += 1
                total_acts += 1
                sender_ex += 1
                e_256[i, t] = _sample_embedding(rng, vec_drop)
                x_num[i, t] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=True,
                        is_explicit_addressee=False,
                        is_reply_or_expect_link=False,
                        is_dyadic_place=False,
                        active_participant_count=n_participants,
                        dt_since_last_place_event_s=dt_sender,
                        dt_since_last_soul_act_s=dt_soul,
                        dt_since_sender_last_act_s=dt_sender,
                        soul_acts_window=soul_acts,
                        total_acts_window=total_acts,
                        sender_exchange_count=sender_ex,
                        place_event_count=place_cnt,
                        has_recent_visible_soul_act=True,
                        cos_soul_user=float(rng.uniform(0.05, 0.35)),
                        cos_prev_user_rephrase=float(rng.uniform(0.05, 0.30)),
                        cos_expected_reply=float(rng.uniform(0.0, 0.25)),
                        is_reply_to_last_soul=False,
                        soul_intervened_since_sender_last=(r == 0),
                        max_active_thread_cos=float(rng.uniform(0.3, 0.7)),
                        active_thread_count=int(rng.integers(1, 4)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.3, 0.7)),
                    ),
                    schema_version,
                )
                y_gate[i, t] = 0  # Habituated -> Silent
                y_compute_tier[i, t] = COMPUTE_TIER_FAST
                y_user_correction[i, t] = 0.0
                mask[i, t] = 1.0
                v22_mask[i, t] = 1.0
                gap = float(rng.uniform(0.3, 2.0))
                dt_soul += gap
                dt_sender = gap

        elif regime == 1:
            # Regime 1: Multi-party patience (Wait -> third-party answers -> Silent)
            e_256[i, 0] = _sample_embedding(rng, vec_drop)
            x_num[i, 0] = _build_features(
                PerceptNumericInputsV2(
                    is_named_soul=False,
                    is_explicit_addressee=False,
                    is_reply_or_expect_link=False,
                    is_dyadic_place=False,
                    active_participant_count=n_participants,
                    dt_since_last_place_event_s=float(rng.uniform(5.0, 600.0)),
                    dt_since_last_soul_act_s=float(rng.uniform(300.0, 86400.0)),
                    dt_since_sender_last_act_s=86400.0,
                    soul_acts_window=0,
                    total_acts_window=1,
                    sender_exchange_count=0,
                    place_event_count=1,
                    active_thread_count=int(rng.integers(1, 5)),
                    sender_reliability=float(rng.uniform(0.4, 0.85)),
                ),
                schema_version,
            )
            y_gate[i, 0] = 0
            y_compute_tier[i, 0] = COMPUTE_TIER_FAST
            y_user_correction[i, 0] = 0.0
            mask[i, 0] = 1.0
            v22_mask[i, 0] = 1.0

            if seq_len > 1:
                e_256[i, 1] = _sample_embedding(rng, vec_drop)
                x_num[i, 1] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=True,
                        is_explicit_addressee=False,
                        is_reply_or_expect_link=False,
                        is_dyadic_place=False,
                        active_participant_count=n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(0.3, 15.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(120.0, 86400.0)),
                        dt_since_sender_last_act_s=float(rng.uniform(10.0, 86400.0)),
                        soul_acts_window=0,
                        total_acts_window=2,
                        sender_exchange_count=int(rng.integers(0, 3)),
                        place_event_count=2,
                        max_active_thread_cos=float(rng.uniform(0.35, 0.80)),
                        active_thread_count=int(rng.integers(1, 5)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.45, 0.85)),
                    ),
                    schema_version,
                )
                y_gate[i, 1] = 2  # Wait
                y_wait_ms[i, 1] = float(rng.uniform(1800.0, 5500.0))
                y_compute_tier[i, 1] = COMPUTE_TIER_STANDARD
                y_user_correction[i, 1] = 0.0
                mask[i, 1] = 1.0
                v22_mask[i, 1] = 1.0

            if seq_len > 2:
                e_256[i, 2] = _sample_embedding(rng, vec_drop)
                x_num[i, 2] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=False,
                        is_explicit_addressee=False,
                        is_reply_or_expect_link=False,
                        is_dyadic_place=False,
                        active_participant_count=n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(0.2, 3.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(120.0, 86400.0)),
                        dt_since_sender_last_act_s=float(rng.uniform(0.5, 30.0)),
                        soul_acts_window=0,
                        total_acts_window=3,
                        sender_exchange_count=int(rng.integers(0, 4)),
                        place_event_count=3,
                        max_active_thread_cos=float(rng.uniform(0.60, 0.92)),
                        active_thread_count=int(rng.integers(1, 5)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.5, 0.9)),
                    ),
                    schema_version,
                )
                y_gate[i, 2] = 0
                y_compute_tier[i, 2] = COMPUTE_TIER_FAST
                y_user_correction[i, 2] = 0.0
                mask[i, 2] = 1.0
                v22_mask[i, 2] = 1.0

        elif regime == 2:
            # Regime 2: Unaddressed group chatter -> Silent
            n_turns = int(rng.integers(3, min(7, max(4, seq_len))))
            for t in range(min(n_turns, seq_len)):
                e_256[i, t] = _sample_embedding(rng, vec_drop)
                x_num[i, t] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=False,
                        is_explicit_addressee=False,
                        is_reply_or_expect_link=False,
                        is_dyadic_place=False,
                        active_participant_count=n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(0.3, 60.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(30.0, 86400.0)),
                        dt_since_sender_last_act_s=float(rng.uniform(0.5, 120.0)),
                        soul_acts_window=int(rng.integers(0, 2)),
                        total_acts_window=max(1, t + 1),
                        sender_exchange_count=int(rng.integers(0, 5)),
                        place_event_count=t + 1,
                        max_active_thread_cos=float(rng.uniform(0.20, 0.85)),
                        active_thread_count=int(rng.integers(1, 6)),
                        sender_in_top_thread=bool(rng.random() < 0.6),
                        sender_reliability=float(rng.uniform(0.35, 0.9)),
                    ),
                    schema_version,
                )
                y_gate[i, t] = 0
                y_compute_tier[i, t] = COMPUTE_TIER_FAST
                y_user_correction[i, t] = 0.0
                mask[i, t] = 1.0
                v22_mask[i, t] = 1.0

        elif regime == 3:
            # Regime 3: Direct address / dyadic / reply-link positive control -> Go
            is_dyadic = bool(rng.random() < 0.5)
            n_turns = int(rng.integers(2, min(5, max(3, seq_len))))
            for t in range(min(n_turns, seq_len)):
                direct = (t % 2 == 1) or is_dyadic
                is_complex = direct and bool(rng.random() < 0.35)
                unc = float(rng.uniform(0.60, 0.95)) if is_complex else float(rng.uniform(0.0, 0.25))
                e_256[i, t] = _sample_embedding(rng, vec_drop)
                x_num[i, t] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=direct,
                        is_explicit_addressee=direct and (not is_dyadic or rng.random() < 0.7),
                        is_reply_or_expect_link=direct and (rng.random() < 0.4),
                        is_dyadic_place=is_dyadic,
                        active_participant_count=2 if is_dyadic else n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(1.0, 120.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(10.0, 86400.0)),
                        dt_since_sender_last_act_s=float(rng.uniform(1.0, 600.0)),
                        soul_acts_window=int(rng.integers(0, 3)),
                        total_acts_window=max(1, t + 2),
                        sender_exchange_count=int(rng.integers(0, 8)),
                        place_event_count=t + 1,
                        pending_belief_uncertainty=unc,
                        max_active_thread_cos=float(rng.uniform(0.45, 0.90)),
                        active_thread_count=1 if is_dyadic else int(rng.integers(1, 5)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.5, 0.95)),
                    ),
                    schema_version,
                )
                y_gate[i, t] = 3 if direct else 0
                if not direct:
                    y_compute_tier[i, t] = COMPUTE_TIER_FAST
                elif is_complex:
                    y_compute_tier[i, t] = COMPUTE_TIER_DEEP
                else:
                    y_compute_tier[i, t] = (
                        COMPUTE_TIER_FAST if (is_dyadic and unc < 0.12) else COMPUTE_TIER_STANDARD
                    )
                y_user_correction[i, t] = 0.0
                mask[i, t] = 1.0
                v22_mask[i, t] = 1.0

        elif regime == 4:
            # Regime 4: *Pecah balon* multi-bubble burst patience -> Wait (1500-4000ms) -> Go/Silent
            burst_len = int(rng.integers(2, min(6, max(3, seq_len))))
            is_dyadic = bool(rng.random() < 0.5)
            directed_to_soul = is_dyadic or bool(rng.random() < 0.65)
            for b in range(min(burst_len, seq_len)):
                is_last = b == min(burst_len, seq_len) - 1
                gap_s = (
                    float(rng.uniform(4.5, 12.0))
                    if is_last
                    else float(rng.uniform(0.3, 3.0))
                )
                e_256[i, b] = _sample_embedding(rng, 0.60)
                x_num[i, b] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=directed_to_soul and (b == 0 or is_last),
                        is_explicit_addressee=directed_to_soul and is_last,
                        is_reply_or_expect_link=(b > 0),
                        is_dyadic_place=is_dyadic,
                        active_participant_count=2 if is_dyadic else n_participants,
                        dt_since_last_place_event_s=gap_s if b > 0 else float(rng.uniform(5.0, 120.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(30.0, 3600.0)),
                        dt_since_sender_last_act_s=gap_s if b > 0 else float(rng.uniform(10.0, 600.0)),
                        soul_acts_window=int(rng.integers(0, 2)),
                        total_acts_window=b + 1,
                        sender_exchange_count=b + 1,
                        place_event_count=b + 1,
                        max_active_thread_cos=float(rng.uniform(0.55, 0.92)),
                        active_thread_count=1 if is_dyadic else int(rng.integers(1, 4)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.5, 0.9)),
                    ),
                    schema_version,
                )
                if not is_last:
                    y_gate[i, b] = 2  # Wait for remaining bubbles in burst
                    y_wait_ms[i, b] = float(rng.uniform(1500.0, 4000.0))
                    y_compute_tier[i, b] = COMPUTE_TIER_FAST
                else:
                    if directed_to_soul:
                        y_gate[i, b] = 3  # Resolved burst directed at soul -> Go
                        y_compute_tier[i, b] = COMPUTE_TIER_STANDARD
                    else:
                        y_gate[i, b] = 0  # Resolved unaddressed burst -> Silent
                        y_compute_tier[i, b] = COMPUTE_TIER_FAST
                y_user_correction[i, b] = 0.0
                mask[i, b] = 1.0
                v22_mask[i, b] = 1.0

        elif regime == 5:
            # Regime 5: Spherical code-switch drift (15-30 deg) + max_visible_mem_cos attenuation (0.70-0.85)
            base_vec = _sample_unit_embedding(rng)
            n_turns = int(rng.integers(3, min(6, max(4, seq_len))))
            is_dyadic = bool(rng.random() < 0.5)
            for t in range(min(n_turns, seq_len)):
                angle_deg = float(rng.uniform(15.0, 30.0))
                e_256[i, t] = _rotate_on_sphere(base_vec, angle_deg, rng)
                atten = float(rng.uniform(0.70, 0.85))
                raw_mem_cos = float(rng.uniform(0.65, 0.96))
                attenuated_mem_cos = raw_mem_cos * atten
                raw_thread_cos = float(rng.uniform(0.60, 0.92))
                attenuated_thread_cos = raw_thread_cos * atten

                mode = t % 3
                if mode == 0:
                    # Direct code-switched query -> Go
                    named, expl, dyad = True, True, is_dyadic
                    lbl, w_ms, tier = 3, 0.0, COMPUTE_TIER_STANDARD
                elif mode == 1 and not is_dyadic:
                    # Open-floor code-switched mention -> Wait
                    named, expl, dyad = True, False, False
                    lbl, w_ms, tier = 2, float(rng.uniform(1800.0, 4500.0)), COMPUTE_TIER_STANDARD
                else:
                    # Unaddressed code-switched group chatter -> Silent (or dyadic follow-up Go)
                    if is_dyadic:
                        named, expl, dyad = True, True, True
                        lbl, w_ms, tier = 3, 0.0, COMPUTE_TIER_STANDARD
                    else:
                        named, expl, dyad = False, False, False
                        lbl, w_ms, tier = 0, 0.0, COMPUTE_TIER_FAST

                x_num[i, t] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=named,
                        is_explicit_addressee=expl,
                        is_reply_or_expect_link=(t > 0 and lbl == 3),
                        is_dyadic_place=dyad,
                        active_participant_count=2 if dyad else n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(1.0, 60.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(60.0, 7200.0)),
                        dt_since_sender_last_act_s=float(rng.uniform(2.0, 120.0)),
                        soul_acts_window=int(rng.integers(0, 2)),
                        total_acts_window=t + 2,
                        sender_exchange_count=int(rng.integers(1, 6)),
                        place_event_count=t + 2,
                        max_visible_mem_cos=attenuated_mem_cos,
                        max_visible_mem_age_s=float(rng.uniform(60.0, 86400.0)),
                        max_active_thread_cos=attenuated_thread_cos,
                        active_thread_count=1 if dyad else int(rng.integers(1, 5)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.45, 0.9)),
                    ),
                    schema_version,
                )
                y_gate[i, t] = lbl
                y_wait_ms[i, t] = w_ms
                y_compute_tier[i, t] = tier
                y_user_correction[i, t] = 0.0
                mask[i, t] = 1.0
                v22_mask[i, t] = 1.0

        elif regime == 6:
            # Regime 6: Multi-party self-repair (`dev-10`) & implicit user correction (`p_user_correction`)
            is_user_corrects_soul = bool(rng.random() < 0.5)
            if not is_user_corrects_soul:
                # 6a: Speaker sends ambiguous/incomplete open-floor message -> Wait -> Speaker self-corrects -> Silent/Go
                e_256[i, 0] = _sample_embedding(rng, vec_drop)
                x_num[i, 0] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=False,
                        is_explicit_addressee=False,
                        is_dyadic_place=False,
                        active_participant_count=n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(5.0, 120.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(300.0, 86400.0)),
                        place_event_count=1,
                        active_thread_count=int(rng.integers(1, 4)),
                    ),
                    schema_version,
                )
                y_gate[i, 0] = 0
                y_compute_tier[i, 0] = COMPUTE_TIER_FAST
                y_user_correction[i, 0] = 0.0
                mask[i, 0] = 1.0
                v22_mask[i, 0] = 1.0

                if seq_len > 1:
                    # Ambiguous / incomplete turn -> Wait for self-repair window
                    e_256[i, 1] = _sample_embedding(rng, vec_drop)
                    x_num[i, 1] = _build_features(
                        PerceptNumericInputsV2(
                            is_named_soul=True,
                            is_explicit_addressee=False,
                            is_dyadic_place=False,
                            active_participant_count=n_participants,
                            dt_since_last_place_event_s=float(rng.uniform(1.0, 15.0)),
                            dt_since_last_soul_act_s=float(rng.uniform(180.0, 86400.0)),
                            dt_since_sender_last_act_s=float(rng.uniform(10.0, 600.0)),
                            total_acts_window=2,
                            place_event_count=2,
                            pending_belief_uncertainty=float(rng.uniform(0.45, 0.85)),
                            max_active_thread_cos=float(rng.uniform(0.40, 0.80)),
                            active_thread_count=int(rng.integers(1, 4)),
                            sender_in_top_thread=True,
                            sender_reliability=float(rng.uniform(0.5, 0.9)),
                        ),
                        schema_version,
                    )
                    y_gate[i, 1] = 2  # Wait
                    y_wait_ms[i, 1] = float(rng.uniform(2000.0, 5000.0))
                    y_compute_tier[i, 1] = COMPUTE_TIER_STANDARD
                    y_user_correction[i, 1] = 0.0
                    mask[i, 1] = 1.0
                    v22_mask[i, 1] = 1.0

                if seq_len > 2:
                    # Speaker self-corrects within 0.8-4.0s (soul did NOT intervene)
                    direct_after_repair = bool(rng.random() < 0.45)
                    e_256[i, 2] = _sample_embedding(rng, vec_drop)
                    x_num[i, 2] = _build_features(
                        PerceptNumericInputsV2(
                            is_named_soul=direct_after_repair,
                            is_explicit_addressee=direct_after_repair,
                            is_reply_or_expect_link=direct_after_repair,
                            is_dyadic_place=False,
                            active_participant_count=n_participants,
                            dt_since_last_place_event_s=float(rng.uniform(0.8, 4.0)),
                            dt_since_last_soul_act_s=float(rng.uniform(180.0, 86400.0)),
                            dt_since_sender_last_act_s=float(rng.uniform(0.8, 4.0)),
                            total_acts_window=3,
                            sender_exchange_count=2,
                            place_event_count=3,
                            has_recent_visible_soul_act=False,
                            cos_prev_user_rephrase=float(rng.uniform(0.55, 0.92)),
                            soul_intervened_since_sender_last=False,
                            max_active_thread_cos=float(rng.uniform(0.60, 0.92)),
                            active_thread_count=int(rng.integers(1, 4)),
                            sender_in_top_thread=True,
                            sender_reliability=float(rng.uniform(0.5, 0.9)),
                        ),
                        schema_version,
                    )
                    y_gate[i, 2] = 3 if direct_after_repair else 0
                    y_compute_tier[i, 2] = (
                        COMPUTE_TIER_STANDARD if direct_after_repair else COMPUTE_TIER_FAST
                    )
                    y_user_correction[i, 2] = 0.0
                    mask[i, 2] = 1.0
                    v22_mask[i, 2] = 1.0
            else:
                # 6b: Soul replied -> user rephrases / contradicts soul -> y_user_correction = 1.0, Deep
                is_dyadic = bool(rng.random() < 0.5)
                e_256[i, 0] = _sample_embedding(rng, vec_drop)
                x_num[i, 0] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=True,
                        is_explicit_addressee=True,
                        is_dyadic_place=is_dyadic,
                        active_participant_count=2 if is_dyadic else n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(2.0, 60.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(60.0, 3600.0)),
                        total_acts_window=1,
                        place_event_count=1,
                        active_thread_count=1 if is_dyadic else int(rng.integers(1, 4)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.55, 0.95)),
                    ),
                    schema_version,
                )
                y_gate[i, 0] = 3
                y_compute_tier[i, 0] = COMPUTE_TIER_STANDARD
                y_user_correction[i, 0] = 0.0
                mask[i, 0] = 1.0
                v22_mask[i, 0] = 1.0

                if seq_len > 1:
                    # User rephrases / corrects after soul's reply
                    e_256[i, 1] = _sample_embedding(rng, vec_drop)
                    x_num[i, 1] = _build_features(
                        PerceptNumericInputsV2(
                            is_named_soul=True,
                            is_explicit_addressee=True,
                            is_reply_or_expect_link=True,
                            is_dyadic_place=is_dyadic,
                            active_participant_count=2 if is_dyadic else n_participants,
                            dt_since_last_place_event_s=float(rng.uniform(1.0, 20.0)),
                            dt_since_last_soul_act_s=float(rng.uniform(1.0, 20.0)),
                            dt_since_sender_last_act_s=float(rng.uniform(3.0, 40.0)),
                            soul_acts_window=1,
                            total_acts_window=3,
                            sender_exchange_count=2,
                            place_event_count=3,
                            pending_belief_uncertainty=float(rng.uniform(0.45, 0.90)),
                            has_recent_visible_soul_act=True,
                            cos_soul_user=float(rng.uniform(0.15, 0.55)),
                            cos_prev_user_rephrase=float(rng.uniform(0.65, 0.95)),
                            cos_expected_reply=float(rng.uniform(0.05, 0.35)),
                            is_reply_to_last_soul=True,
                            soul_intervened_since_sender_last=True,
                            max_active_thread_cos=float(rng.uniform(0.65, 0.95)),
                            active_thread_count=1 if is_dyadic else int(rng.integers(1, 4)),
                            sender_in_top_thread=True,
                            sender_reliability=float(rng.uniform(0.55, 0.95)),
                        ),
                        schema_version,
                    )
                    y_gate[i, 1] = 3  # Wake to repair
                    y_compute_tier[i, 1] = COMPUTE_TIER_DEEP
                    y_user_correction[i, 1] = 1.0
                    mask[i, 1] = 1.0
                    v22_mask[i, 1] = 1.0

                if seq_len > 2:
                    # User confirms resolution -> Silent
                    e_256[i, 2] = _sample_embedding(rng, vec_drop)
                    x_num[i, 2] = _build_features(
                        PerceptNumericInputsV2(
                            is_named_soul=False,
                            is_explicit_addressee=False,
                            is_reply_or_expect_link=True,
                            is_dyadic_place=is_dyadic,
                            active_participant_count=2 if is_dyadic else n_participants,
                            dt_since_last_place_event_s=float(rng.uniform(1.0, 15.0)),
                            dt_since_last_soul_act_s=float(rng.uniform(1.0, 15.0)),
                            dt_since_sender_last_act_s=float(rng.uniform(5.0, 30.0)),
                            soul_acts_window=2,
                            total_acts_window=5,
                            sender_exchange_count=3,
                            place_event_count=5,
                            pending_belief_uncertainty=float(rng.uniform(0.0, 0.15)),
                            has_recent_visible_soul_act=True,
                            cos_soul_user=float(rng.uniform(0.55, 0.90)),
                            cos_prev_user_rephrase=float(rng.uniform(0.05, 0.25)),
                            cos_expected_reply=float(rng.uniform(0.70, 0.96)),
                            is_reply_to_last_soul=True,
                            soul_intervened_since_sender_last=True,
                            max_active_thread_cos=float(rng.uniform(0.60, 0.90)),
                            active_thread_count=1 if is_dyadic else int(rng.integers(1, 4)),
                            sender_in_top_thread=True,
                            sender_reliability=float(rng.uniform(0.6, 0.95)),
                        ),
                        schema_version,
                    )
                    y_gate[i, 2] = 0
                    y_compute_tier[i, 2] = COMPUTE_TIER_FAST
                    y_user_correction[i, 2] = 0.0
                    mask[i, 2] = 1.0
                    v22_mask[i, 2] = 1.0

        else:
            # Regime 7: Peer-agent coordination (`dev-13`: anti-ping-pong, expertise deferral, own-expertise Go)
            sub_mode = (i // n_regimes) % 3
            if sub_mode == 0:
                # 7a: Anti-ping-pong: automated peer turns rising (>= 2/6) -> Silent
                n_turns = int(rng.integers(3, min(6, max(4, seq_len))))
                for t in range(min(n_turns, seq_len)):
                    consec = min(6, 2 + t)
                    e_256[i, t] = _sample_embedding(rng, vec_drop)
                    x_num[i, t] = _build_features(
                        PerceptNumericInputsV2(
                            is_named_soul=bool(rng.random() < 0.6),
                            is_explicit_addressee=False,
                            is_reply_or_expect_link=True,
                            is_dyadic_place=False,
                            active_participant_count=n_participants,
                            dt_since_last_place_event_s=float(rng.uniform(0.3, 4.0)),
                            dt_since_last_soul_act_s=float(rng.uniform(0.5, 10.0)),
                            dt_since_sender_last_act_s=float(rng.uniform(0.5, 6.0)),
                            soul_acts_window=min(t + 1, 3),
                            total_acts_window=t + 3,
                            sender_exchange_count=t + 2,
                            place_event_count=t + 3,
                            has_recent_visible_soul_act=True,
                            active_thread_count=int(rng.integers(1, 4)),
                            sender_in_top_thread=True,
                            sender_reliability=float(rng.uniform(0.5, 0.9)),
                            is_sender_automated_peer=True,
                            room_has_other_automated_peers=True,
                            consecutive_automated_turns=consec,
                            peer_expertise_margin=float(rng.uniform(-0.1, 0.6)),
                        ),
                        schema_version,
                    )
                    y_gate[i, t] = 0  # Anti-ping-pong -> Silent
                    y_compute_tier[i, t] = COMPUTE_TIER_FAST
                    y_user_correction[i, t] = 0.0
                    mask[i, t] = 1.0
                    v22_mask[i, t] = 1.0
            elif sub_mode == 1:
                # 7b: Expertise deferral: another specialist bot better suited (margin > 0.65 -> raw > +0.30)
                e_256[i, 0] = _sample_embedding(rng, vec_drop)
                x_num[i, 0] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=False,
                        is_explicit_addressee=False,
                        is_dyadic_place=False,
                        active_participant_count=n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(1.0, 30.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(60.0, 3600.0)),
                        total_acts_window=2,
                        place_event_count=2,
                        active_thread_count=int(rng.integers(1, 4)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.5, 0.9)),
                        is_sender_automated_peer=False,
                        room_has_other_automated_peers=True,
                        consecutive_automated_turns=0,
                        peer_expertise_margin=float(rng.uniform(0.35, 0.95)),  # maps to (0.675, 0.975)
                    ),
                    schema_version,
                )
                y_gate[i, 0] = 2  # Wait to let specialist peer answer
                y_wait_ms[i, 0] = float(rng.uniform(2000.0, 5000.0))
                y_compute_tier[i, 0] = COMPUTE_TIER_FAST
                y_user_correction[i, 0] = 0.0
                mask[i, 0] = 1.0
                v22_mask[i, 0] = 1.0

                if seq_len > 1:
                    # Specialist peer replies -> Silent
                    e_256[i, 1] = _sample_embedding(rng, vec_drop)
                    x_num[i, 1] = _build_features(
                        PerceptNumericInputsV2(
                            is_named_soul=False,
                            is_explicit_addressee=False,
                            is_reply_or_expect_link=True,
                            is_dyadic_place=False,
                            active_participant_count=n_participants,
                            dt_since_last_place_event_s=float(rng.uniform(0.5, 3.0)),
                            dt_since_last_soul_act_s=float(rng.uniform(60.0, 3600.0)),
                            total_acts_window=3,
                            place_event_count=3,
                            max_active_thread_cos=float(rng.uniform(0.60, 0.92)),
                            active_thread_count=int(rng.integers(1, 4)),
                            sender_in_top_thread=True,
                            sender_reliability=float(rng.uniform(0.6, 0.95)),
                            is_sender_automated_peer=True,
                            room_has_other_automated_peers=True,
                            consecutive_automated_turns=1,
                            peer_expertise_margin=float(rng.uniform(0.40, 0.95)),
                        ),
                        schema_version,
                    )
                    y_gate[i, 1] = 0  # Silent (specialist handled it)
                    y_compute_tier[i, 1] = COMPUTE_TIER_FAST
                    y_user_correction[i, 1] = 0.0
                    mask[i, 1] = 1.0
                    v22_mask[i, 1] = 1.0
            else:
                # 7c: Own-expertise wake: low peer_expertise_margin (< 0.35 -> raw < -0.30) -> Go
                is_deep = bool(rng.random() < 0.45)
                unc = float(rng.uniform(0.55, 0.90)) if is_deep else float(rng.uniform(0.05, 0.30))
                e_256[i, 0] = _sample_embedding(rng, vec_drop)
                x_num[i, 0] = _build_features(
                    PerceptNumericInputsV2(
                        is_named_soul=True,
                        is_explicit_addressee=bool(rng.random() < 0.7),
                        is_reply_or_expect_link=False,
                        is_dyadic_place=False,
                        active_participant_count=n_participants,
                        dt_since_last_place_event_s=float(rng.uniform(1.0, 45.0)),
                        dt_since_last_soul_act_s=float(rng.uniform(30.0, 3600.0)),
                        total_acts_window=2,
                        place_event_count=2,
                        pending_belief_uncertainty=unc,
                        max_active_thread_cos=float(rng.uniform(0.50, 0.90)),
                        active_thread_count=int(rng.integers(1, 4)),
                        sender_in_top_thread=True,
                        sender_reliability=float(rng.uniform(0.5, 0.95)),
                        is_sender_automated_peer=False,
                        room_has_other_automated_peers=True,
                        consecutive_automated_turns=0,
                        peer_expertise_margin=float(rng.uniform(-0.95, -0.35)),  # maps to (0.025, 0.325)
                    ),
                    schema_version,
                )
                y_gate[i, 0] = 3  # Go (our soul is the right specialist)
                y_compute_tier[i, 0] = COMPUTE_TIER_DEEP if is_deep else COMPUTE_TIER_STANDARD
                y_user_correction[i, 0] = 0.0
                mask[i, 0] = 1.0
                v22_mask[i, 0] = 1.0

                if seq_len > 1:
                    # Peer acknowledges after our answer -> Silent
                    e_256[i, 1] = _sample_embedding(rng, vec_drop)
                    x_num[i, 1] = _build_features(
                        PerceptNumericInputsV2(
                            is_named_soul=False,
                            is_explicit_addressee=False,
                            is_reply_or_expect_link=True,
                            is_dyadic_place=False,
                            active_participant_count=n_participants,
                            dt_since_last_place_event_s=float(rng.uniform(0.5, 5.0)),
                            dt_since_last_soul_act_s=float(rng.uniform(0.5, 5.0)),
                            soul_acts_window=1,
                            total_acts_window=4,
                            place_event_count=4,
                            has_recent_visible_soul_act=True,
                            is_sender_automated_peer=True,
                            room_has_other_automated_peers=True,
                            consecutive_automated_turns=2,
                            peer_expertise_margin=float(rng.uniform(-0.2, 0.4)),
                        ),
                        schema_version,
                    )
                    y_gate[i, 1] = 0
                    y_compute_tier[i, 1] = COMPUTE_TIER_FAST
                    y_user_correction[i, 1] = 0.0
                    mask[i, 1] = 1.0
                    v22_mask[i, 1] = 1.0

    return {
        "e_256": e_256,
        "x_num": x_num,
        "y_gate": y_gate,
        "y_wait_ms": y_wait_ms,
        "mask": mask,
        "y_unc": zeros_f.copy(),
        "m_unc": zeros_f.copy(),
        "y_ver": zeros_f.copy(),
        "m_ver": zeros_f.copy(),
        "y_enc": zeros_f.copy(),
        "m_enc": zeros_f.copy(),
        "y_rel": zeros_f.copy(),
        "m_rel": zeros_f.copy(),
        "y_compute_tier": y_compute_tier,
        "y_user_correction": y_user_correction,
        "v22_mask": v22_mask,
        "dataset_id": dataset_id,
    }


def build_procedural_timing_shards(
    seq_len: int = SEQ_LEN_DEFAULT,
    n_train: int = 640,
    n_cal: int = 80,
    n_test: int = 80,
    seed: int = 20261121,
    schema_version: int = 1,
) -> dict[str, dict[str, np.ndarray]]:
    """Builds deterministic train/calibration/test splits for procedural group-chat curricula.

    `schema_version=1` emits 18d Regimes 0-3 (backward compatible);
    `schema_version=2` emits 32d Regimes 0-7 (including pecah balon, code-switch drift,
    multi-party self-repair, implicit user correction, and peer-agent coordination).
    """
    rng = np.random.default_rng(seed)
    return {
        "train": _generate_split(n_train, seq_len, rng, schema_version=schema_version),
        "calibration": _generate_split(n_cal, seq_len, rng, schema_version=schema_version),
        "test": _generate_split(n_test, seq_len, rng, schema_version=schema_version),
    }


def build_procedural_v22_shards(
    seq_len: int = SEQ_LEN_DEFAULT,
    n_train: int = 1280,
    n_cal: int = 160,
    n_test: int = 160,
    seed: int = 20261122,
) -> dict[str, dict[str, np.ndarray]]:
    """Convenience entry point generating 32d schema=2 procedural curricula across Regimes 0-7."""
    return build_procedural_timing_shards(
        seq_len=seq_len,
        n_train=n_train,
        n_cal=n_cal,
        n_test=n_test,
        seed=seed,
        schema_version=2,
    )

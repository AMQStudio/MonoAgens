"""Procedural group-chat timing curriculum (habituation, third-party patience/Wait, and vectorless robustness).

Generates general, randomized multi-turn sequences (100% synthetic, OSI-clean, zero sealed/ access)
teaching universal multi-party timing laws that text-only datasets under-represent:
  1. Rapid-repeat habituation in group rooms -> Silent (class 0)
  2. Open-floor loose soul mention in multi-party rooms -> Wait (class 2) + bounded wait_delta_ms
  3. Unaddressed group chatter -> Silent (class 0)
  4. Direct address / dyadic / reply-link -> Go (class 3)

Includes both vectorless (`e_256 = 0`) and random normalized embeddings so the controller stays
calibrated even when no embedding sidecar is attached.
"""

from __future__ import annotations

import numpy as np

from monoagens_train.features import NUM_FEATURES, PerceptNumericInputs, build_x_num

SEQ_LEN_DEFAULT = 24
EMB_DIM = 256


def _sample_embedding(rng: np.random.Generator, vectorless_prob: float) -> np.ndarray:
    if rng.random() < vectorless_prob:
        return np.zeros(EMB_DIM, dtype=np.float32)
    vec = rng.normal(0.0, 1.0, size=EMB_DIM).astype(np.float32)
    norm = float(np.linalg.norm(vec))
    return vec / max(norm, 1e-6)


def _generate_split(
    n_sequences: int,
    seq_len: int,
    rng: np.random.Generator,
) -> dict[str, np.ndarray]:
    e_256 = np.zeros((n_sequences, seq_len, EMB_DIM), dtype=np.float32)
    x_num = np.zeros((n_sequences, seq_len, NUM_FEATURES), dtype=np.float32)
    y_gate = np.zeros((n_sequences, seq_len), dtype=np.int64)
    y_wait_ms = np.zeros((n_sequences, seq_len), dtype=np.float32)
    mask = np.zeros((n_sequences, seq_len), dtype=np.float32)
    zeros_f = np.zeros((n_sequences, seq_len), dtype=np.float32)
    dataset_id = np.full((n_sequences,), "procedural_timing", dtype="<U24")

    for i in range(n_sequences):
        regime = i % 4
        vec_drop = 0.70  # strong coverage of vectorless (e_256=0) alongside random unit vectors
        n_participants = int(rng.integers(3, 18))

        if regime == 0:
            # Regime 0: Rapid-repeat habituation in multi-party room
            # Turn 0: casual room chatter -> Silent
            # Turn 1: explicit address -> Go (soul speaks)
            # Turns 2..k: rapid repeated loose mentions from same sender without explicit address -> Silent
            n_repeats = int(rng.integers(3, min(8, seq_len - 2)))
            t_place = float(rng.uniform(0.5, 10.0))
            sender_ex = 0
            place_cnt = 1
            soul_acts = 0
            total_acts = 1

            # Turn 0: casual chatter
            e_256[i, 0] = _sample_embedding(rng, vec_drop)
            x_num[i, 0] = build_x_num(
                PerceptNumericInputs(
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
                )
            )
            y_gate[i, 0] = 0
            mask[i, 0] = 1.0

            # Turn 1: direct address -> Go
            place_cnt += 1
            total_acts += 1
            sender_ex += 1
            e_256[i, 1] = _sample_embedding(rng, vec_drop)
            x_num[i, 1] = build_x_num(
                PerceptNumericInputs(
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
                )
            )
            y_gate[i, 1] = 3
            mask[i, 1] = 1.0

            # Soul responded right after turn 1
            soul_acts = 1
            total_acts += 1
            dt_soul = float(rng.uniform(1.0, 6.0))
            dt_sender = dt_soul

            for r in range(n_repeats):
                t = 2 + r
                place_cnt += 1
                total_acts += 1
                sender_ex += 1
                e_256[i, t] = _sample_embedding(rng, vec_drop)
                x_num[i, t] = build_x_num(
                    PerceptNumericInputs(
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
                    )
                )
                y_gate[i, t] = 0  # Habituated -> Silent
                mask[i, t] = 1.0
                gap = float(rng.uniform(0.3, 2.0))
                dt_soul += gap
                dt_sender = gap

        elif regime == 1:
            # Regime 1: Multi-party patience (Wait -> third-party answers -> Silent)
            # Turn 0: casual room chatter -> Silent
            # Turn 1: loose mention of soul in open group floor (not explicit addressee, soul hasn't just spoken) -> Wait
            # Turn 2: third party replies -> Silent
            # Optional Turn 3: follow-up chatter -> Silent
            e_256[i, 0] = _sample_embedding(rng, vec_drop)
            x_num[i, 0] = build_x_num(
                PerceptNumericInputs(
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
                )
            )
            y_gate[i, 0] = 0
            mask[i, 0] = 1.0

            # Turn 1: open-floor loose mention -> Wait (class 2) with 1500..6000 ms patience
            e_256[i, 1] = _sample_embedding(rng, vec_drop)
            x_num[i, 1] = build_x_num(
                PerceptNumericInputs(
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
                )
            )
            y_gate[i, 1] = 2  # Wait
            y_wait_ms[i, 1] = float(rng.uniform(1800.0, 5500.0))
            mask[i, 1] = 1.0

            # Turn 2: third party answers -> Silent
            e_256[i, 2] = _sample_embedding(rng, vec_drop)
            x_num[i, 2] = build_x_num(
                PerceptNumericInputs(
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
                )
            )
            y_gate[i, 2] = 0
            mask[i, 2] = 1.0

        elif regime == 2:
            # Regime 2: Unaddressed group chatter (keeps Silent solid without embeddings)
            n_turns = int(rng.integers(3, min(7, seq_len)))
            for t in range(n_turns):
                e_256[i, t] = _sample_embedding(rng, vec_drop)
                x_num[i, t] = build_x_num(
                    PerceptNumericInputs(
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
                    )
                )
                y_gate[i, t] = 0
                mask[i, t] = 1.0

        else:
            # Regime 3: Direct address / dyadic / reply-link positive control -> Go (class 3)
            is_dyadic = bool(rng.random() < 0.5)
            n_turns = int(rng.integers(2, 5))
            for t in range(n_turns):
                direct = (t % 2 == 1) or is_dyadic
                e_256[i, t] = _sample_embedding(rng, vec_drop)
                x_num[i, t] = build_x_num(
                    PerceptNumericInputs(
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
                    )
                )
                y_gate[i, t] = 3 if direct else 0
                mask[i, t] = 1.0

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
        "dataset_id": dataset_id,
    }


def build_procedural_timing_shards(
    seq_len: int = SEQ_LEN_DEFAULT,
    n_train: int = 640,
    n_cal: int = 80,
    n_test: int = 80,
    seed: int = 20261121,
) -> dict[str, dict[str, np.ndarray]]:
    """Builds deterministic train/calibration/test splits for procedural group-chat timing."""
    rng = np.random.default_rng(seed)
    return {
        "train": _generate_split(n_train, seq_len, rng),
        "calibration": _generate_split(n_cal, seq_len, rng),
        "test": _generate_split(n_test, seq_len, rng),
    }

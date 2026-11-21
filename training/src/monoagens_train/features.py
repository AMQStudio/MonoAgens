"""Model-agnostic numeric feature builders (`x_num`) for schema v1 (18d) and v2 (32d)."""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np

FEATURE_SCHEMA_VERSION = 1
FEATURE_SCHEMA_VERSION_V1 = 1
FEATURE_SCHEMA_VERSION_V2 = 2
NUM_FEATURES = 18
NUM_FEATURES_V1 = 18
NUM_FEATURES_V2 = 32

FEATURE_NAMES: tuple[str, ...] = (
    "is_named_soul",
    "is_explicit_addressee",
    "is_reply_or_expect_link",
    "is_dyadic_place",
    "active_participant_count_norm",
    "dt_since_last_place_event",
    "dt_since_last_soul_act",
    "dt_since_sender_last_act",
    "soul_talk_share_window",
    "sender_familiarity",
    "place_familiarity",
    "budget_used_ratio",
    "imminent_promise_debt",
    "max_visible_mem_cos",
    "max_visible_mem_recency",
    "pending_belief_uncertainty",
    "backlog_queue_depth_norm",
    "asr_conf_or_one",
)
FEATURE_NAMES_V1: tuple[str, ...] = FEATURE_NAMES

FEATURE_NAMES_V2_EXT: tuple[str, ...] = (
    # 18..24: 6d conversational repair features (R-68)
    "has_recent_visible_soul_act",
    "cos_soul_user",
    "cos_prev_user_rephrase",
    "cos_expected_reply",
    "is_reply_to_last_soul",
    "soul_intervened_since_sender_last",
    # 24..28: 4d thread/actor features (R-69)
    "max_active_thread_cos",
    "active_thread_count_norm",
    "sender_in_top_thread",
    "sender_reliability",
    # 28..32: 4d peer-agent features (R-71)
    "is_sender_automated_peer",
    "room_has_other_automated_peers",
    "consecutive_automated_turns_norm",
    "peer_expertise_margin",
)
FEATURE_NAMES_V2: tuple[str, ...] = FEATURE_NAMES + FEATURE_NAMES_V2_EXT


def _sanitize_unit(val: float, default: float = 0.0) -> float:
    if not math.isfinite(val):
        return default
    if val < 0.0:
        return 0.0
    if val > 1.0:
        return 1.0
    return float(val)


def _log1p_norm(val: float, max_scale: float) -> float:
    if not math.isfinite(val) or val <= 0.0:
        return 0.0
    return _sanitize_unit(math.log1p(val) / math.log1p(max_scale))


def _map_signed_unit(val: float | None, default: float = 0.5) -> float:
    """Maps a signed score in [-1, 1] onto [0, 1], falling back to `default` when None or non-finite."""
    if val is None or not math.isfinite(val):
        return default
    clamped = max(-1.0, min(1.0, float(val)))
    return _sanitize_unit(0.5 * (clamped + 1.0), default=default)


@dataclass(frozen=True)
class PerceptNumericInputs:
    is_named_soul: bool = False
    is_explicit_addressee: bool = False
    is_reply_or_expect_link: bool = False
    is_dyadic_place: bool = False
    active_participant_count: int = 2
    dt_since_last_place_event_s: float = 0.0
    dt_since_last_soul_act_s: float = 86400.0
    dt_since_sender_last_act_s: float = 86400.0
    soul_acts_window: int = 0
    total_acts_window: int = 1
    sender_exchange_count: int = 0
    place_event_count: int = 0
    budget_used_ratio: float = 0.0
    imminent_promise_debt: float = 0.0
    max_visible_mem_cos: float = 0.0
    max_visible_mem_age_s: float = 0.0
    pending_belief_uncertainty: float = 0.0
    backlog_queue_depth: int = 0
    asr_conf_or_one: float = 1.0


@dataclass(frozen=True)
class PerceptNumericInputsV2(PerceptNumericInputs):
    """Schema v2 (32d) numeric inputs extending v1 with repair, thread/actor, and peer-agent signals."""

    # 18..24: 6d conversational repair (R-68)
    has_recent_visible_soul_act: bool = False
    cos_soul_user: float = 0.0
    cos_prev_user_rephrase: float = 0.0
    cos_expected_reply: float = 0.0
    is_reply_to_last_soul: bool = False
    soul_intervened_since_sender_last: bool = False
    # 24..28: 4d thread/actor (R-69)
    max_active_thread_cos: float = 0.0
    active_thread_count: int = 0
    active_thread_count_norm: float | None = None
    sender_in_top_thread: bool = False
    sender_reliability: float = 0.5
    # 28..32: 4d peer-agent (R-71)
    is_sender_automated_peer: bool = False
    room_has_other_automated_peers: bool = False
    consecutive_automated_turns: int = 0
    consecutive_automated_turns_norm: float | None = None
    peer_expertise_margin: float | None = None


def build_x_num(inp: PerceptNumericInputs) -> np.ndarray:
    """Constructs the finite, [0, 1]-bounded 18d float32 `x_num` feature vector (schema v1)."""
    total_acts = max(1, int(inp.total_acts_window))
    soul_share = _sanitize_unit(float(max(0, inp.soul_acts_window)) / float(total_acts))
    sender_fam = _sanitize_unit(1.0 - math.exp(-max(0.0, float(inp.sender_exchange_count)) / 5.0))
    place_fam = _sanitize_unit(1.0 - math.exp(-max(0.0, float(inp.place_event_count)) / 20.0))

    vec = np.array(
        [
            1.0 if inp.is_named_soul else 0.0,
            1.0 if inp.is_explicit_addressee else 0.0,
            1.0 if inp.is_reply_or_expect_link else 0.0,
            1.0 if inp.is_dyadic_place else 0.0,
            _log1p_norm(float(max(0, inp.active_participant_count)), 64.0),
            _log1p_norm(inp.dt_since_last_place_event_s, 86400.0),
            _log1p_norm(inp.dt_since_last_soul_act_s, 86400.0),
            _log1p_norm(inp.dt_since_sender_last_act_s, 86400.0),
            soul_share,
            sender_fam,
            place_fam,
            _sanitize_unit(inp.budget_used_ratio),
            _sanitize_unit(inp.imminent_promise_debt),
            _sanitize_unit(inp.max_visible_mem_cos),
            _log1p_norm(inp.max_visible_mem_age_s, 2_592_000.0),
            _sanitize_unit(inp.pending_belief_uncertainty),
            _log1p_norm(float(max(0, inp.backlog_queue_depth)), 256.0),
            _sanitize_unit(inp.asr_conf_or_one, default=1.0),
        ],
        dtype=np.float32,
    )
    return vec


def build_x_num_v2(inp: PerceptNumericInputsV2) -> np.ndarray:
    """Constructs the finite, [0, 1]-bounded 32d float32 `x_num` feature vector (schema v2)."""
    base = build_x_num(inp)
    thread_count_norm = (
        _sanitize_unit(inp.active_thread_count_norm)
        if inp.active_thread_count_norm is not None
        else _sanitize_unit(float(min(max(0, int(inp.active_thread_count)), 6)) / 6.0)
    )
    consec_auto_norm = (
        _sanitize_unit(inp.consecutive_automated_turns_norm)
        if inp.consecutive_automated_turns_norm is not None
        else _sanitize_unit(float(min(max(0, int(inp.consecutive_automated_turns)), 6)) / 6.0)
    )
    ext = np.array(
        [
            # 18..24: 6d repair
            1.0 if inp.has_recent_visible_soul_act else 0.0,
            _sanitize_unit(inp.cos_soul_user),
            _sanitize_unit(inp.cos_prev_user_rephrase),
            _sanitize_unit(inp.cos_expected_reply),
            1.0 if inp.is_reply_to_last_soul else 0.0,
            1.0 if inp.soul_intervened_since_sender_last else 0.0,
            # 24..28: 4d thread/actor
            _sanitize_unit(inp.max_active_thread_cos),
            thread_count_norm,
            1.0 if inp.sender_in_top_thread else 0.0,
            _sanitize_unit(inp.sender_reliability, default=0.5),
            # 28..32: 4d peer-agent
            1.0 if inp.is_sender_automated_peer else 0.0,
            1.0 if inp.room_has_other_automated_peers else 0.0,
            consec_auto_norm,
            _map_signed_unit(inp.peer_expertise_margin, default=0.5),
        ],
        dtype=np.float32,
    )
    return np.concatenate([base, ext], axis=0)


def upgrade_x_num_v1_to_v2(x_num: np.ndarray) -> np.ndarray:
    """Upcasts `[..., 18]` schema v1 features to `[..., 32]` with neutral v2 defaults."""
    arr = np.asarray(x_num, dtype=np.float32)
    if arr.shape[-1] == NUM_FEATURES_V2:
        return arr.copy()
    if arr.shape[-1] != NUM_FEATURES_V1:
        raise ValueError(
            f"Expected x_num last dim {NUM_FEATURES_V1} or {NUM_FEATURES_V2}, got {arr.shape[-1]}"
        )
    out = np.zeros((*arr.shape[:-1], NUM_FEATURES_V2), dtype=np.float32)
    out[..., :NUM_FEATURES_V1] = arr
    out[..., 27] = 0.5  # sender_reliability neutral Beta(1, 1) prior
    out[..., 31] = 0.5  # peer_expertise_margin neutral default
    return out

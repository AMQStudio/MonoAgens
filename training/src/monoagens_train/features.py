"""18-scalar model-agnostic numeric feature builder (`x_num`) matching §3 of 01_mechanism_and_loc_audit.md."""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np

FEATURE_SCHEMA_VERSION = 1
NUM_FEATURES = 18

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


def build_x_num(inp: PerceptNumericInputs) -> np.ndarray:
    """Constructs the finite, [0, 1]-bounded 18d float32 `x_num` feature vector."""
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

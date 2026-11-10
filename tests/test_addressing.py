"""The addressing scorer (ADR-0012 §4.7): each signal alone, the bands, and the reasons."""

import pytest

from monoagens.engine.addressing import (
    ANSWER,
    GROUP,
    IGNORE,
    SELF,
    UNSURE,
    Result,
    Signals,
    score,
)
from monoagens.engine.config import (
    AddressingAdvancedConfig,
    AddressingConfig,
    AddressingWeightsConfig,
)


def cfg(**kwargs) -> AddressingConfig:
    return AddressingConfig(**kwargs)


def test_a_mention_alone_answers() -> None:
    result = score(Signals(mentions_self=True), cfg())

    assert result == Result(100, ANSWER, SELF, [("mention_self", 100)])


def test_a_reply_to_the_soul_alone_is_unsure() -> None:
    result = score(Signals(reply_to_self=True), cfg())

    assert result.score == 50
    assert result.decision == UNSURE
    assert result.addressed_to == GROUP


def test_quoting_the_soul_to_tag_someone_else_never_answers() -> None:
    result = score(
        Signals(reply_to_self=True, mentions_other=True, other_target="per_other"),
        cfg(),
    )

    # The tagging row replaces `reply_to_self` only; the mention of the person still applies.
    assert result.score == -160
    assert result.decision == IGNORE
    assert result.addressed_to == "person:per_other"
    assert result.reasons == [
        ("reply_to_self_tagging_other", -80),
        ("mention_other", -80),
    ]


def test_mentioning_another_person_points_at_that_person() -> None:
    result = score(Signals(mentions_other=True, other_target="per_other"), cfg())

    assert result.score == -80
    assert result.decision == IGNORE
    assert result.addressed_to == "person:per_other"
    assert result.reasons == [("mention_other", -80)]


def test_a_negative_signal_without_a_target_stays_a_group_message() -> None:
    result = score(Signals(mentions_other=True), cfg())

    assert result.addressed_to == GROUP


def test_a_reply_to_another_person_is_negative() -> None:
    result = score(
        Signals(reply_to_other=True, other_target="per_other"),
        cfg(),
    )

    assert result.reasons == [("reply_to_other", -50)]
    assert result.decision == IGNORE


def test_plain_chatter_scores_zero_and_ignores() -> None:
    result = score(Signals(), cfg())

    assert result.score == 0
    assert result.decision == IGNORE
    assert result.reasons == []
    assert result.addressed_to == GROUP


def test_continuity_and_a_pending_request_add_up() -> None:
    result = score(Signals(continuity=True, pending_request=True), cfg())

    assert result.score == 80
    assert result.decision == ANSWER
    assert result.addressed_to == SELF


def test_the_name_in_text_is_a_weak_signal() -> None:
    result = score(Signals(name_in_text=True), cfg())

    assert result.score == 15
    assert result.decision == IGNORE


def test_a_direct_chat_is_decisive() -> None:
    result = score(Signals(direct=True), cfg())

    assert result == Result(100, ANSWER, SELF, [("direct", 100)])


def test_a_direct_chat_is_decisive_whatever_else_fired() -> None:
    result = score(Signals(direct=True, mentions_other=True), cfg())

    assert result.decision == ANSWER
    assert result.addressed_to == SELF


@pytest.mark.parametrize(
    ("level", "decision"),
    [("strict", IGNORE), ("normal", UNSURE), ("lenient", ANSWER)],
)
def test_skepticism_shifts_both_bands(level: str, decision: str) -> None:
    assert score(Signals(continuity=True), cfg(), skepticism=level).decision == decision


def test_an_unknown_skepticism_level_falls_back_to_normal() -> None:
    assert score(Signals(continuity=True), cfg(), skepticism="wild").decision == UNSURE


def test_the_thresholds_come_from_the_config_not_from_constants() -> None:
    strict_cfg = cfg(answer_at=45, ignore_below=10)

    assert score(Signals(reply_to_self=True), strict_cfg).decision == ANSWER
    assert score(Signals(), strict_cfg).addressed_to == GROUP
    assert score(Signals(), strict_cfg).decision == IGNORE


def test_the_weights_come_from_the_config() -> None:
    custom = cfg(
        advanced=AddressingAdvancedConfig(
            weights=AddressingWeightsConfig(mention_self=5, reply_to_self=5, continuity=5)
        )
    )

    result = score(Signals(mentions_self=True), custom)

    assert result.score == 5
    assert result.decision == IGNORE

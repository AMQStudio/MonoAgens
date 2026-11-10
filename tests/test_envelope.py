"""Deterministic envelope labels (ADR-0012 §4.1): pure rules, no store and no database."""

from datetime import datetime

from monoagens.engine.envelope import (
    KINDS,
    SPEAKERS,
    TRUSTS,
    label_message,
    label_value,
    merge_labels,
    replies_to_self,
    stored_labels,
)
from monoagens.engine.turn.context import AUTHOR_BOT, Message, Quote


def message(text: str = "", *, outgoing: bool = False, author: str = "unknown", notice: str = ""):
    return Message(
        id="m1",
        sender_name="Budi",
        sender_id="628222",
        when=datetime(2026, 1, 1, 10),
        text=text,
        outgoing=outgoing,
        sender_address="628222@s.whatsapp.net",
        author=author,
        notice=notice,
    )


def test_inbound_human_text_is_a_chat_from_the_sender() -> None:
    labels = label_message(message("halo"))

    assert labels["speaker"] == "sender"
    assert labels["kind"] == "chat"
    assert labels["label_meta"]["speaker"] == {"source": "rule", "version": 1}
    assert labels["label_meta"]["kind"] == {"source": "rule", "version": 1}


def test_inbound_slash_command_is_a_command() -> None:
    labels = label_message(message("/approve #5"))

    assert labels["speaker"] == "sender"
    assert labels["kind"] == "command"


def test_the_command_rule_is_only_a_leading_slash_and_a_name() -> None:
    assert label_message(message("/help"))["kind"] == "command"
    assert label_message(message("/approve\nrest"))["kind"] == "command"
    assert label_message(message("/"))["kind"] == "chat"
    assert label_message(message("/9x"))["kind"] == "chat"
    assert label_message(message("halo /approve"))["kind"] == "chat"


def test_outgoing_bot_text_is_self() -> None:
    labels = label_message(message("siap, kak", outgoing=True, author=AUTHOR_BOT))

    assert labels["speaker"] == "self"
    assert labels["kind"] == "chat"


def test_outgoing_human_text_has_no_certain_speaker() -> None:
    labels = label_message(message("dikirim manual", outgoing=True, author="manual_owner"))

    assert labels["speaker"] == "unknown"
    assert labels["kind"] == "chat"


def test_empty_text_is_an_unknown_kind_and_keeps_only_set_labels_in_meta() -> None:
    labels = label_message(message(""))

    assert labels["kind"] == "unknown"
    assert labels["speaker"] == "sender"
    assert labels["label_meta"] == {"speaker": {"source": "rule", "version": 1}}


def test_a_notice_is_a_notice() -> None:
    labels = label_message(message("timeout", notice="notice:timeout"))

    assert labels["kind"] == "notice"


def test_localized_text_never_changes_the_result() -> None:
    english = label_message(message("hello there"))
    indonesian = label_message(message("halo, apa kabar?"))
    arabic = label_message(message("مرحبا، كيف حالك؟"))

    assert english == indonesian == arabic


def test_every_label_is_inside_the_allowed_sets() -> None:
    cases = [
        message(""),
        message("halo"),
        message("/approve #5"),
        message("siap", outgoing=True, author=AUTHOR_BOT),
        message("x", notice="notice:timeout"),
        message("", outgoing=True, author=AUTHOR_BOT),
        message("9", outgoing=True, author="manual_owner"),
    ]

    for case in cases:
        labels = label_message(case)
        assert labels["speaker"] in SPEAKERS
        assert labels["kind"] in KINDS


def test_label_value_replaces_a_value_outside_its_set() -> None:
    assert label_value("bogus", SPEAKERS) == "unknown"
    assert label_value(None, KINDS) == "unknown"
    assert label_value("member", TRUSTS) == "member"


def test_stored_labels_keep_the_labels_a_message_already_carries() -> None:
    labelled = Message(
        id="m1",
        sender_name="Budi",
        sender_id="628222",
        when=datetime(2026, 1, 1, 10),
        text="halo",
        speaker="other_person",
        kind="question",
        label_meta={"speaker": {"source": "gateway", "version": 1}},
    )

    labels = stored_labels(labelled)

    assert labels["speaker"] == "other_person"
    assert labels["kind"] == "question"
    assert labels["label_meta"] == {"speaker": {"source": "gateway", "version": 1}}


def test_stored_labels_fall_back_to_the_rules_when_both_are_unknown() -> None:
    assert stored_labels(message("halo"))["speaker"] == "sender"


def test_the_souls_own_text_that_looks_like_a_command_stays_chat() -> None:
    """ADR-0017 seed: an archive summary that mentions `/approve #5` is the Soul speaking."""
    spoken = message("/approve #5 is waiting for you", outgoing=True, author=AUTHOR_BOT)

    labels = label_message(spoken)

    assert labels["speaker"] == "self"
    assert labels["kind"] == "chat"


def quoted(sender_id: str, *, quote_id: str = "q1") -> Message:
    found = message("halo")
    found.quote = Quote(
        id=quote_id,
        sender_name="Soul",
        sender_id=sender_id,
        text="siap",
        when=datetime(2026, 1, 1, 9),
    )
    return found


def test_a_quote_of_a_self_handle_replies_to_self() -> None:
    assert replies_to_self(quoted("628567770837"), {"628567770837"}, None) is True


def test_the_quote_author_is_compared_in_normalised_form() -> None:
    assert replies_to_self(quoted("628111@S.WHATSAPP.NET"), {"628111@s.whatsapp.net"}, None)


def test_a_quote_of_a_stored_bot_message_replies_to_self() -> None:
    found = message("siap, kak", outgoing=True, author=AUTHOR_BOT)

    assert replies_to_self(quoted("628222", quote_id="m1"), set(), found) is True


def test_a_quote_of_another_person_is_not_a_reply_to_self() -> None:
    found = message("halo")

    assert replies_to_self(quoted("628222", quote_id="m1"), {"628567770837"}, found) is False


def test_a_message_without_a_quote_is_not_a_reply_to_self() -> None:
    assert replies_to_self(message("halo"), {"628567770837"}, None) is False


def test_no_handle_and_no_found_message_is_not_a_reply_to_self() -> None:
    assert replies_to_self(quoted("628222"), set(), None) is False
    assert replies_to_self(quoted(""), {""}, None) is False


def test_merge_labels_never_downgrades_a_stored_label() -> None:
    old = message("halo")
    old.speaker = "other_person"
    old.kind = "question"
    old.trust = "member"
    old.addressed_to = "self"
    old.actor = "per_1"
    old.reply_to_self = True
    old.valid_until = {"at": 10}
    old.label_meta = {"speaker": {"source": "gateway", "version": 1}}

    merged = merge_labels(old, message("halo"))

    assert merged == {
        "speaker": "other_person",
        "kind": "question",
        "trust": "member",
        "addressed_to": "self",
        "actor": "per_1",
        "reply_to_self": True,
        "valid_until": {"at": 10},
        "label_meta": {"speaker": {"source": "gateway", "version": 1}},
    }


def test_merge_labels_prefers_the_signal_the_new_row_carries() -> None:
    old = message("halo")
    old.speaker = "sender"
    old.actor = "per_1"
    old.label_meta = {"speaker": {"source": "rule", "version": 1}}
    fresh = message("halo")
    fresh.speaker = "system_receipt"
    fresh.actor = "per_2"
    fresh.kind = "notice"
    fresh.label_meta = {"speaker": {"source": "transport", "version": 1}}

    merged = merge_labels(old, fresh)

    assert merged["speaker"] == "system_receipt"
    assert merged["kind"] == "notice"
    assert merged["actor"] == "per_2"
    assert merged["label_meta"] == {"speaker": {"source": "transport", "version": 1}}


def test_a_sticky_actor_survives_a_duplicate_without_one() -> None:
    old = message("halo")
    old.actor = "per_1"
    old.reply_to_self = True

    merged = merge_labels(old, message("halo"))

    assert merged["actor"] == "per_1"
    assert merged["reply_to_self"] is True


def test_a_duplicate_with_unknown_labels_keeps_the_window_labels() -> None:
    from monoagens.engine.turn.context import WorkingMemory

    window = WorkingMemory()
    first = message("halo")
    first.speaker = "system_receipt"
    first.kind = "notice"
    first.actor = "per_1"
    window.append("chat", first, persist=False)
    window.append("chat", message("halo"), persist=False)

    kept = window.get("chat")[0]

    assert kept.speaker == "system_receipt"
    assert kept.kind == "notice"
    assert kept.actor == "per_1"

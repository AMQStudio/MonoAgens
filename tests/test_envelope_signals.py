"""Envelope signals that need the store (ADR-0012 §4.1): reply-to-self and command receipts.

The labels are written once, at ingress: `TurnIntake` marks a quote of the Soul's own message,
and a command receipt is stored as a system notice with the person who ran the command.
"""

from datetime import datetime

from support import inbound_message, make_agent

from monoagens.engine.turn.context import AUTHOR_BOT, Message, Quote

DM = "628111@s.whatsapp.net"
OTHER = "628222@s.whatsapp.net"
SELF_HANDLE = "628567770837"


def bot_message(message_id: str = "bot-1") -> Message:
    return Message(
        id=message_id,
        sender_name="Soul",
        sender_id=SELF_HANDLE,
        when=datetime(2026, 1, 1, 10),
        text="siap, kak",
        outgoing=True,
        author=AUTHOR_BOT,
    )


def quoted(quote_id: str, sender_id: str, *, message_id: str = "m-1"):
    inbound = inbound_message("halo", thread=DM, message_id=message_id, dm=False)
    inbound.payload.quote = Quote(
        id=quote_id,
        sender_name="Soul",
        sender_id=sender_id,
        text="siap",
        when=datetime(2026, 1, 1, 9),
    )
    return inbound


def test_a_command_receipt_is_stored_as_a_system_notice_with_its_actor(tmp_path) -> None:
    env = make_agent(tmp_path)

    env.agent.delivery.remember(
        DM,
        "Approval #5 recorded.",
        gateway=env.gateway,
        message_id="receipt-1",
        receipt_actor="per_1",
    )

    stored = env.history.recent(DM, 10)[0]
    assert stored.speaker == "system_receipt"
    assert stored.actor == "per_1"
    assert stored.kind == "notice"
    assert stored.outgoing is True
    assert stored.author == AUTHOR_BOT
    assert stored.label_meta == {
        "speaker": {"source": "rule", "version": 1},
        "actor": {"source": "transport", "version": 1},
    }


def test_an_ordinary_self_message_stays_the_souls_own_words(tmp_path) -> None:
    env = make_agent(tmp_path)

    env.agent.delivery.remember(
        DM, "siap, kak", gateway=env.gateway, message_id="self-1"
    )

    stored = env.history.recent(DM, 10)[0]
    assert stored.speaker == "self"
    assert stored.actor == ""
    assert stored.kind == "chat"


def test_a_quote_of_a_stored_bot_message_is_marked_reply_to_self(tmp_path) -> None:
    env = make_agent(tmp_path)
    env.history.add(DM, bot_message())
    env.windows.append(DM, bot_message(), persist=False)

    assert env.agent.intake.observe_group(quoted("bot-1", OTHER), env.gateway, "group_mention")
    stored = env.history.find(DM, "m-1")
    assert stored is not None
    assert stored.reply_to_self is True


def test_a_quote_of_another_person_is_not_a_reply_to_self(tmp_path) -> None:
    env = make_agent(tmp_path)
    human = Message(
        id="human-1",
        sender_name="Budi",
        sender_id="628222",
        when=datetime(2026, 1, 1, 10),
        text="halo",
        sender_address=f"{OTHER}",
    )
    env.history.add(DM, human)
    env.windows.append(DM, human, persist=False)

    assert env.agent.intake.observe_group(quoted("human-1", OTHER), env.gateway, "group_mention")
    stored = env.history.find(DM, "m-1")
    assert stored is not None
    assert stored.reply_to_self is False


def test_a_quote_of_a_self_handle_is_marked_without_a_stored_row(tmp_path) -> None:
    env = make_agent(tmp_path)

    assert env.agent.intake.observe_group(
        quoted("missing", SELF_HANDLE), env.gateway, "group_mention"
    )
    stored = env.history.find(DM, "m-1")
    assert stored is not None
    assert stored.reply_to_self is True

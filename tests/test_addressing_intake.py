"""Addressing at intake (ADR-0012 §4.7): the decision, its reasons, and the render line.

The score is recorded with the message; it does not decide who answers yet (phase P3).
"""

import asyncio
from dataclasses import replace
from datetime import datetime

from support import inbound_message, make_agent

from monoagens.engine.turn.context import AUTHOR_BOT, Message, Quote, build_user_content

GROUP = "123@g.us"
DM = "628111@s.whatsapp.net"
SELF_HANDLE = "628567770837"
OTHER = "333"


def handle(env, inbound):
    return asyncio.run(env.agent.handle(inbound, env.gateway))


def group_message(text: str = "pagi", **hints):
    inbound = inbound_message(text, thread=GROUP, dm=False)
    return replace(inbound, hints={**inbound.hints, **hints})


def stored(env, message_id: str) -> Message:
    found = env.history.find(GROUP, message_id)
    assert found is not None
    return found


def test_a_group_mention_is_stored_as_addressed_to_the_soul(tmp_path) -> None:
    env = make_agent(tmp_path)
    inbound = group_message("halo mono", mention=True)

    handle(env, inbound)

    entry = stored(env, inbound.payload.id)
    assert entry.addressed_to == "self"
    record = entry.label_meta["addressed_to"]
    assert record["source"] == "rule"
    assert record["version"] == 1
    assert record["decision"] == "answer"
    assert record["score"] == 100
    assert record["reasons"] == [["mention_self", 100]]


def test_a_reply_to_the_soul_that_tags_someone_else_is_not_an_answer(tmp_path) -> None:
    env = make_agent(tmp_path)
    bot = Message(
        id="bot-1",
        sender_name="Soul",
        sender_id=SELF_HANDLE,
        when=datetime.now(),
        text="siap, kak",
        outgoing=True,
        author=AUTHOR_BOT,
        speaker="self",
    )
    env.history.add(GROUP, bot)
    env.windows.append(GROUP, bot, persist=False)
    inbound = group_message("@ani ini maksudnya", mentions=[OTHER])
    inbound.payload.quote = Quote(
        id="bot-1", sender_name="Soul", sender_id=SELF_HANDLE, text="siap, kak"
    )

    handle(env, inbound)

    entry = stored(env, inbound.payload.id)
    record = entry.label_meta["addressed_to"]
    assert entry.reply_to_self is True
    assert record["decision"] != "answer"
    assert entry.addressed_to == f"person:{OTHER}"
    names = [name for name, _points in record["reasons"]]
    assert "reply_to_self_tagging_other" in names
    assert "mention_other" in names


def test_plain_chatter_is_stored_as_ignored(tmp_path) -> None:
    env = make_agent(tmp_path)
    inbound = group_message("besok rapat jam sembilan")

    handle(env, inbound)

    entry = stored(env, inbound.payload.id)
    assert entry.addressed_to == "group"
    assert entry.label_meta["addressed_to"]["decision"] == "ignore"
    assert entry.label_meta["addressed_to"]["score"] == 0


def test_a_direct_chat_is_addressed_to_the_soul(tmp_path) -> None:
    env = make_agent(tmp_path)
    inbound = replace(inbound_message("halo", thread=DM), hints={"dm": True})

    handle(env, inbound)

    entry = env.history.find(DM, inbound.payload.id)
    assert entry is not None
    assert entry.addressed_to == "self"
    assert entry.label_meta["addressed_to"]["reasons"] == [["direct", 100]]


def test_the_souls_name_in_plain_text_is_a_weak_signal(tmp_path) -> None:
    env = make_agent(tmp_path)
    inbound = group_message("tanya monoagens dulu")

    handle(env, inbound)

    entry = stored(env, inbound.payload.id)
    assert entry.label_meta["addressed_to"]["reasons"] == [["name_in_text", 15]]


def test_a_duplicate_delivery_keeps_the_addressing_record(tmp_path) -> None:
    env = make_agent(tmp_path)
    inbound = group_message("halo mono", mention=True)
    handle(env, inbound)

    duplicate = Message(
        id=inbound.payload.id,
        sender_name=inbound.payload.sender_name,
        sender_id=inbound.payload.sender_id,
        when=datetime.now(),
        text="halo mono",
    )
    env.history.add(GROUP, duplicate)

    entry = stored(env, inbound.payload.id)
    assert entry.label_meta["addressed_to"]["reasons"] == [["mention_self", 100]]


def test_the_current_message_carries_its_addressing_reason(tmp_path) -> None:
    env = make_agent(tmp_path)
    current = Message(
        id="m-9",
        sender_name="Budi",
        sender_id="628222",
        when=datetime(2026, 1, 1, 10),
        text="halo mono",
        addressed_to="self",
        label_meta={
            "addressed_to": {
                "source": "rule",
                "version": 1,
                "score": 100,
                "decision": "answer",
                "reasons": [["mention_self", 100]],
            }
        },
    )

    text = asyncio.run(build_user_content(env.config, GROUP, [current], current, group=True))

    assert "  addressing: answer (mention_self +100)" in text
    assert ", to you" in text


def test_an_unlabelled_message_renders_as_before(tmp_path) -> None:
    env = make_agent(tmp_path)
    current = Message(
        id="m-9",
        sender_name="Budi",
        sender_id="628222",
        when=datetime(2026, 1, 1, 10),
        text="pagi",
    )

    text = asyncio.run(build_user_content(env.config, GROUP, [current], current, group=True))

    assert "addressing:" not in text
    assert ", to you" not in text

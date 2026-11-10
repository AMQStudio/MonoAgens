"""Model-facing rendering of the stored envelope labels: receipts and replies to the Soul."""

import asyncio
from datetime import datetime
from types import SimpleNamespace

from support import build_config

from monoagens.engine.soul import Soul
from monoagens.engine.tools.thread_history import _lines
from monoagens.engine.turn.context import Message, Quote, build_user_content, prompt_entry

QUIET = Soul(
    language="id",
    locale="id-ID",
    timezone="UTC",
    name="monoagens",
    persona="p",
)

BOT = "monoagens"
BOT_ID = "628567770837"


def receipt(mid: str = "r1", *, actor: str = "p1") -> Message:
    return Message(
        id=mid,
        sender_name=BOT,
        sender_id=BOT_ID,
        when=datetime(2026, 1, 1, 10),
        text="Approved #5.",
        author="bot",
        speaker="system_receipt",
        actor=actor,
        kind="notice",
    )


def bot_message(mid: str = "b1", text: str = "halo") -> Message:
    return Message(
        id=mid,
        sender_name=BOT,
        sender_id=BOT_ID,
        when=datetime(2026, 1, 1, 10),
        text=text,
        author="bot",
    )


def quoted(mid: str, *, to_self: bool) -> Message:
    return Message(
        id=mid,
        sender_name="Budi",
        sender_id="628222",
        when=datetime(2026, 1, 1, 10),
        text="bro gaslighting",
        author="unknown",
        reply_to_self=to_self,
        quote=Quote(
            id="q1",
            sender_name=BOT if to_self else "Siti",
            sender_id=BOT_ID if to_self else "628333",
            text="I approved #5",
            when=datetime(2026, 1, 1, 9),
        ),
    )


def payload(tmp_path, entry: Message) -> str:
    config = build_config(tmp_path, soul=QUIET)
    out = asyncio.run(build_user_content(config, "chat", [entry], entry))
    assert isinstance(out, str)
    return out


def test_receipt_renders_as_the_system_not_the_soul(tmp_path) -> None:
    text = payload(tmp_path, receipt())

    head = next(line for line in text.splitlines() if line.startswith("[2026-01-01 10:00]"))
    assert head == "[2026-01-01 10:00] system [role=bot vis=L2, system receipt, run by p1]:"
    assert ", you" not in head


def test_normal_bot_message_keeps_the_you_marker(tmp_path) -> None:
    text = payload(tmp_path, bot_message())

    head = next(line for line in text.splitlines() if line.startswith("[2026-01-01 10:00]"))
    assert head == f"[2026-01-01 10:00] {BOT} ({BOT_ID}) [role=bot vis=L2, you]:"


def test_receipt_prompt_entry_names_the_actor() -> None:
    line = prompt_entry(receipt(), BOT)

    assert line.startswith("[role=bot vis=L2, system receipt, run by p1] system (run by p1):")
    assert "you (" not in line


def test_quote_of_the_soul_renders_as_replying_to_you(tmp_path) -> None:
    text = payload(tmp_path, quoted("q1", to_self=True))

    quote_line = next(line for line in text.splitlines() if "replying to" in line)
    assert quote_line == (
        '  ↳ replying to YOU (your own earlier message) [2026-01-01 09:00]: "I approved #5"'
    )
    assert f"replying to {BOT}" not in quote_line


def test_quote_of_someone_else_renders_as_before(tmp_path) -> None:
    text = payload(tmp_path, quoted("q2", to_self=False))

    quote_line = next(line for line in text.splitlines() if "replying to" in line)
    assert quote_line == '  ↳ replying to [2026-01-01 09:00] Siti (628333): "I approved #5"'


def test_prompt_entry_marks_a_reply_to_the_soul() -> None:
    assert 'replying to you: "I approved #5"' in prompt_entry(quoted("q1", to_self=True), BOT)
    assert 'replying to Siti: "I approved #5"' in prompt_entry(quoted("q2", to_self=False), BOT)


def archive_lines(entry: Message) -> list[str]:
    ctx = SimpleNamespace(thread_id="chat")
    return _lines(ctx, [entry], "chat", "chat", "", 10)


def test_thread_history_renders_a_receipt_and_a_reply_to_the_soul() -> None:
    receipt_lines = archive_lines(receipt())
    assert receipt_lines[1].startswith(
        "[2026-01-01 10:00] system (run by p1) [role=bot vis=L2, system receipt, run by p1]:"
    )
    assert ", you]" not in receipt_lines[1]

    self_reply = archive_lines(quoted("q1", to_self=True))
    assert self_reply[2] == "  ↳ replying to you (the Soul): I approved #5"

    other_reply = archive_lines(quoted("q2", to_self=False))
    assert other_reply[2] == "  ↳ replying to Siti: I approved #5"

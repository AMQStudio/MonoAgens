"""ADR-0013 §4.4: a requirement is data on the action, never text parsed back out of chat.

`BaseGateway.ui_of` reads an explicit `meta["ui"]`/`meta["options"]` block only. The core puts
the block there when it parks an action; a reply that merely mentions `/approve #5` is text.
"""


import asyncio
from pathlib import Path
from types import SimpleNamespace

from support import FakeGateway, inbound_message, make_agent

from monoagens.engine.i18n import t
from monoagens.engine.inbound import Action, Address
from monoagens.engine.turn.approval_flow import ApprovalFlow
from monoagens.gateways.base import BaseGateway
from monoagens.gateways.telegram.gateway import TelegramGateway

ROOT = Path(__file__).resolve().parents[1]


class Body(BaseGateway):
    """The smallest body that renders labels: a config with a reply language."""

    name = "body"

    def __init__(self, language: str = "en") -> None:
        self.config = SimpleNamespace(soul=SimpleNamespace(language=language))


def ui_action(payload: str = "Approve?", **ui) -> Action:
    return Action(
        address=Address("body", "chat"),
        capability="send_text",
        payload=payload,
        meta={"ui": {"kind": "approval", **ui}},
    )


def test_an_approval_block_becomes_the_localized_pair() -> None:
    block = Body("id").ui_of(ui_action(approval_id=42))

    assert block["kind"] == "approval"
    assert block["approval_id"] == 42
    approve, deny = block["buttons"]
    assert approve["label"] == t("approval.button_approve", "id")
    assert approve["command"] == "/approve #42"
    assert deny["label"] == t("approval.button_deny", "id")
    assert deny["command"] == "/deny #42"


def test_a_text_that_mentions_an_approval_is_plain_text() -> None:
    body = Body()

    for text in ("/approve #7", "waiting: /approve 12", "someone said /approve #5"):
        action = Action(
            address=Address("body", "chat"), capability="send_text", payload=text
        )
        assert body.ui_of(action) == {}
        assert body.prompt_options(action) == []


def test_an_explicit_ui_block_wins() -> None:
    action = Action(
        address=Address("body", "chat"),
        capability="send_text",
        payload="Pick one",
        meta={
            "ui": {
                "kind": "choices",
                "title": "Models",
                "summary": "two are configured",
                "buttons": [
                    {"id": "m1", "label": "fast", "command": "/models fast"},
                    {"label": "slow", "command": "/models slow", "style": "secondary"},
                ],
            }
        },
    )

    block = Body().ui_of(action)

    assert block["kind"] == "choices"
    assert block["title"] == "Models"
    assert block["summary"] == "two are configured"
    assert [item["command"] for item in block["buttons"]] == [
        "/models fast",
        "/models slow",
    ]
    assert block["buttons"][1]["id"] == "/models slow"


def test_a_bare_options_list_is_understood() -> None:
    action = Action(
        address=Address("body", "chat"),
        capability="send_text",
        payload="Pick one",
        meta={"options": [{"label": "Retry", "command": "/retry"}]},
    )

    block = Body().ui_of(action)

    assert block["kind"] == "options"
    assert block["buttons"] == [
        {"id": "/retry", "label": "Retry", "command": "/retry", "style": ""}
    ]


def test_malformed_options_are_dropped() -> None:
    action = Action(
        address=Address("body", "chat"),
        capability="send_text",
        payload="Pick one",
        meta={
            "options": [
                {"label": "no command"},
                {"command": "/nameless"},
                "junk",
                {"label": "kept", "command": "/kept"},
            ]
        },
    )

    assert [item["command"] for item in Body().prompt_options(action)] == ["/kept"]


def test_an_action_without_a_prompt_has_no_block() -> None:
    body = Body()

    assert body.ui_of(None) == {}
    assert body.ui_of(Action(Address("body", "chat"), "send_text", "plain")) == {}
    assert body.prompt_options(Action(Address("body", "chat"), "send_text", "plain")) == []


def test_the_default_language_is_english() -> None:
    assert BaseGateway().language() == "en"


def test_the_engine_keeps_the_payload_and_adds_no_ui(tmp_path) -> None:
    """A plain `send_text` still carries the readable text and no block."""
    env = make_agent(tmp_path)

    asyncio.run(
        env.agent.send_text(
            "chat",
            "halo",
            gateway=env.gateway,
            origin=Address(env.gateway.name, "chat"),
        )
    )

    action = env.gateway.dispatched[-1]
    assert action.payload == "halo"
    assert action.meta == {}


def test_a_parked_approval_is_computed_as_a_block(tmp_path) -> None:
    env = make_agent(tmp_path)
    row = asyncio.run(
        env.agent.approvals.create(
            kind="tool",
            thread_id="chat",
            sender="628111",
            tool="shell",
            args={},
            payload={},
            summary="rm -rf scratch",
            approval_class="authorization",
        )
    )

    class Host(ApprovalFlow):
        def __init__(self) -> None:
            self.runtime = SimpleNamespace(  # pyright: ignore[reportAttributeAccessIssue]
                approvals=env.agent.approvals,
                soul=SimpleNamespace(language="en"),
            )

    found = asyncio.run(Host()._parked_ui("chat", "628111", 0.0))

    assert found == {"kind": "approval", "approval_id": row.id}


def test_the_last_chunk_of_a_reply_carries_the_block(tmp_path) -> None:
    env = make_agent(tmp_path)
    ui = {"kind": "approval", "approval_id": 7}

    _sent, actions, stop = asyncio.run(
        env.agent.delivery.send_reply(
            turn_id="turn-ui",
            thread_id="chat",
            gateway=env.gateway,
            inbound=inbound_message("halo"),
            chunks=["satu", "dua"],
            pace=False,
            quote_all=False,
            generation=0,
            partial="skip",
            humanize=None,
            ui=ui,
        )
    )

    assert stop == ""
    assert [action.meta for action in actions] == [{}, {"ui": ui}]


def test_the_privacy_question_carries_a_block(tmp_path) -> None:
    env = make_agent(tmp_path)
    env.history.intake(
        "chat",
        inbound_message("halo").payload,
        "628111",
        language="en",
        gateway="fake",
    )
    hits = asyncio.run(
        env.agent.privacy.ask_memory("chat", "628111", ["m1"], level=3, question="simpan?")
    )
    assert hits is None

    action = env.gateway.dispatched[-1]
    assert action.payload == "simpan?"
    assert action.meta["ui"]["kind"] == "approval"
    assert action.meta["ui"]["approval_id"] > 0


def test_no_core_module_appends_the_badge_text() -> None:
    found = sorted(
        path.relative_to(ROOT).as_posix()
        for path in (ROOT / "monoagens" / "engine").rglob("*.py")
        if "approval_badge" in path.read_text(encoding="utf-8")
    )
    assert found == []


class TelegramBody(TelegramGateway):
    """The real body's prompt resolution without its transport or its `__init__`."""

    def __init__(self) -> None:
        self.config = SimpleNamespace(soul=SimpleNamespace(language="en"))


def test_the_telegram_body_renders_the_block_as_a_keyboard() -> None:
    markup = TelegramBody().ui_of(ui_action(approval_id=9))

    assert markup["kind"] == "approval"
    assert [item["command"] for item in markup["buttons"]] == [
        "/approve #9",
        "/deny #9",
    ]


def test_a_body_without_a_config_still_reads_an_explicit_block() -> None:
    class Bare(BaseGateway):
        name = "bare"

    action = Action(
        address=Address("bare", "t"),
        capability="send_text",
        payload="x",
        meta={"options": [{"label": "ok", "command": "/ok"}]},
    )

    assert [item["label"] for item in Bare().prompt_options(action)] == ["ok"]


def test_a_plain_fake_gateway_keeps_its_text() -> None:
    """A core-only body inherits nothing that rewrites its payload."""
    gateway = FakeGateway()
    assert not hasattr(gateway, "ui_of")


def test_an_unknown_language_falls_back_to_the_catalog_default() -> None:
    block = Body("xx").ui_of(ui_action(approval_id=1))
    assert block["buttons"][0]["label"] == t("approval.button_approve", "en")

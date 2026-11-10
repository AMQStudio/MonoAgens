"""One decision contract: the typed commands and a gateway gesture decide the same row.

ADR-0013 §4.4-4.5: a decision is data (`Decision`), the core validates deciders, expiry, and
status, and a gesture never becomes a chat command or a turn.
"""


import asyncio
import time
from dataclasses import replace
from datetime import datetime

from sqlalchemy import text as sql
from support import DirectoryFakeGateway, make_agent, role

from monoagens.engine.commands.approve import COMMAND as APPROVE
from monoagens.engine.commands.deny import COMMAND as DENY
from monoagens.engine.config import ApprovalConfig
from monoagens.engine.i18n import t
from monoagens.engine.inbound import DECISION, Action, Address, Decision, Percept
from monoagens.engine.stores.turns import turn_identity
from monoagens.engine.turn.context import Message
from monoagens.gateways.base import BaseGateway

DM = "chat"
OTHER = "other"
PERSON = "628111"
STRANGER = "628999"
ADMIN = "628777"


class GestureGateway(DirectoryFakeGateway):
    """The fixture body plus the shared gesture helper under test."""

    decision_percept = BaseGateway.decision_percept


def send_action(text: str = "ringkasan", *, action_id: str) -> Action:
    return Action(
        address=Address("fake", OTHER), capability="send_text", payload=text, id=action_id
    )


def grant(env, name: str, person: str) -> None:
    """Give one fixture role its person id; `by_name` is the loader's mutable table."""
    current = env.config.roles.get(name)
    assert current is not None
    env.config.roles.by_name[name] = replace(current, users=(person,))


def env_for(tmp_path, *, members=(PERSON, STRANGER, ADMIN)):
    gateway = GestureGateway(members=set(members))
    env = make_agent(
        tmp_path,
        gateway=gateway,
        approvals=ApprovalConfig(mode="default"),
        default_role="guest",
        roles={
            "member": role("member", tools=("send_to",), commands=("*",)),
            "admin": role("admin", tools=("*",), commands=("*",)),
            "guest": role("guest", tools=("read",), commands=()),
        },
    )
    grant(env, "member", env.agent.identity.resolve("fake", PERSON))
    grant(env, "admin", env.agent.identity.resolve("fake", ADMIN))
    gateway.members.add(env.agent.identity.resolve("fake", PERSON))
    return env


def person_of(env, transport: str = PERSON) -> str:
    return env.agent.identity.resolve("fake", transport)


def stage(env, action_id: str, text: str = "ringkasan"):
    """Park one cross-room send through the real door and return its row."""
    asyncio.run(
        env.agent.dispatch(
            person_of(env),
            Address("fake", OTHER),
            send_action(text, action_id=action_id),
            origin=Address("fake", DM),
            reason="kirim",
        )
    )
    row = env.agent.approvals.find_action(action_id)
    assert row is not None
    assert row.status == "pending"
    return row


def decision_percept(
    env,
    approval,
    *,
    approve: bool = True,
    transport: str = PERSON,
    thread: str = DM,
    message_id: str = "d-1",
) -> Percept:
    return Percept(
        source="fake",
        thread=thread,
        actor=transport,
        kind=DECISION,
        payload=Message(
            id=message_id,
            sender_name="Budi",
            sender_id=transport,
            when=datetime.now(),
            text="",
        ),
        hints={"dm": True, "interaction": True, "decision": Decision(approval.id, approve)},
    )


def handle(env, percept) -> list:
    return asyncio.run(env.agent.handle(percept, env.gateway))


def texts(env) -> list[tuple[str, str]]:
    return [(chat, text) for chat, text, _reply in env.gateway.sent]


def test_the_requester_approves_and_the_action_runs_once(tmp_path) -> None:
    env = env_for(tmp_path)
    row = stage(env, "d-approve")

    handle(env, decision_percept(env, row, message_id="d-approve"))

    assert env.agent.approvals.get(row.id).status == "approved"
    assert [chat for chat, _text in texts(env)].count(OTHER) == 1
    sent = t("command.approve.sent", env.config.soul.language, id=row.id, target=OTHER)
    assert (DM, sent) in texts(env)


def test_a_decision_is_not_a_turn_and_never_calls_the_model(tmp_path) -> None:
    env = env_for(tmp_path)
    row = stage(env, "d-noturn")

    handle(env, decision_percept(env, row, message_id="d-noturn"))

    assert env.agent.llm.calls == 0
    assert env.agent.llm.decisions == 0
    assert env.agent.turns.get(turn_identity("fake", DM, "d-noturn")) is None


def test_a_different_member_is_foreign(tmp_path) -> None:
    env = env_for(tmp_path)
    row = stage(env, "d-foreign")

    handle(env, decision_percept(env, row, transport=STRANGER))

    assert env.agent.approvals.get(row.id).status == "pending"
    assert (DM, t("command.approve.foreign", env.config.soul.language)) in texts(env)
    assert OTHER not in [chat for chat, _text in texts(env)]


def test_a_privileged_role_cannot_decide_another_persons_privacy_row(tmp_path) -> None:
    env = env_for(tmp_path)
    row = asyncio.run(
        env.agent.approvals.create(
            kind="privacy",
            thread_id=DM,
            sender=person_of(env),
            gateway="fake",
            tool="memory",
            args={"memory_ids": ["m1"]},
            payload={"level": 3, "question": "simpan?"},
            summary="simpan?",
        )
    )

    handle(env, decision_percept(env, row, transport=ADMIN))

    assert env.agent.approvals.get(row.id).status == "pending"
    assert (DM, t("command.approve.foreign", env.config.soul.language)) in texts(env)


def test_an_expired_row_is_refused_and_marked_expired(tmp_path) -> None:
    env = env_for(tmp_path)
    row = stage(env, "d-expired")
    with env.agent.db.connect() as conn:
        conn.execute(
            sql("UPDATE approvals SET expires_at=:when WHERE id=:id"),
            {"when": time.time() - 5, "id": row.id},
        )

    handle(env, decision_percept(env, row))

    assert env.agent.approvals.get(row.id).status == "expired"
    assert DM in [chat for chat, _text in texts(env)]
    assert OTHER not in [chat for chat, _text in texts(env)]


def test_an_already_decided_row_is_refused(tmp_path) -> None:
    env = env_for(tmp_path)
    row = stage(env, "d-twice")
    assert asyncio.run(APPROVE.fn(env.agent.command_context(
        DM, person_of(env), (env.config.roles.get("member"),), env.gateway, str(row.id)
    )))
    assert env.agent.approvals.get(row.id).status == "approved"

    handle(env, decision_percept(env, row, message_id="d-again"))

    assert env.agent.approvals.get(row.id).status == "approved"
    assert [chat for chat, _text in texts(env)].count(OTHER) == 1
    assert (
        DM,
        t("command.approve.already", env.config.soul.language, id=row.id, status="approved"),
    ) in texts(env)


def test_deny_works_through_the_same_contract(tmp_path) -> None:
    env = env_for(tmp_path)
    row = stage(env, "d-deny")

    handle(env, decision_percept(env, row, approve=False))

    assert env.agent.approvals.get(row.id).status == "denied"
    assert OTHER not in [chat for chat, _text in texts(env)]
    assert (DM, t("command.deny.done", env.config.soul.language, id=row.id)) in texts(env)


def test_the_typed_approve_and_the_decision_percept_reach_the_same_outcome(tmp_path) -> None:
    env = env_for(tmp_path)
    typed = stage(env, "d-typed")
    gesture = stage(env, "d-gesture")
    member = env.config.roles.get("member")

    typed_text = asyncio.run(
        APPROVE.fn(
            env.agent.command_context(
                DM, person_of(env), (member,), env.gateway, str(typed.id)
            )
        )
    )
    handle(env, decision_percept(env, gesture))

    assert env.agent.approvals.get(typed.id).status == "approved"
    assert env.agent.approvals.get(gesture.id).status == "approved"
    assert typed_text == t(
        "command.approve.sent", env.config.soul.language, id=typed.id, target=OTHER
    )
    assert [chat for chat, _text in texts(env)].count(OTHER) == 2


def test_the_typed_commands_still_own_their_own_messages(tmp_path) -> None:
    env = env_for(tmp_path)
    row = stage(env, "d-typed-foreign")
    guest = env.config.roles.get("guest")

    refused = asyncio.run(
        APPROVE.fn(
            env.agent.command_context(DM, STRANGER, (guest,), env.gateway, str(row.id))
        )
    )
    denied = asyncio.run(
        DENY.fn(
            env.agent.command_context(DM, STRANGER, (guest,), env.gateway, str(row.id))
        )
    )

    assert refused == t("command.approve.foreign", env.config.soul.language)
    assert denied == t("command.deny.foreign", env.config.soul.language)
    assert env.agent.approvals.get(row.id).status == "pending"


def test_a_standby_instance_refuses_a_decision(tmp_path) -> None:
    env = env_for(tmp_path)
    row = stage(env, "d-standby")
    env.agent.mode = "standby"

    handle(env, decision_percept(env, row))

    assert env.agent.approvals.get(row.id).status == "pending"
    assert texts(env) == []


def test_a_body_gesture_becomes_a_decision_percept(tmp_path) -> None:
    """A click/reaction uses the shared body helper; the core decides the row."""
    env = env_for(tmp_path)
    row = stage(env, "d-gesture-helper")

    percept = env.gateway.decision_percept(
        "fake",
        DM,
        PERSON,
        "Budi",
        "gesture-1",
        Decision(row.id, True),
        hints={"dm": True},
    )

    assert percept.kind == DECISION
    assert percept.hints["decision"] == Decision(row.id, True)
    assert percept.payload.text == ""

    handle(env, percept)

    assert env.agent.approvals.get(row.id).status == "approved"
    assert [chat for chat, _text in texts(env)].count(OTHER) == 1


def test_a_strangers_reaction_is_refused_and_runs_nothing(tmp_path) -> None:
    env = env_for(tmp_path)
    row = stage(env, "d-gesture-stranger")

    percept = env.gateway.decision_percept(
        "fake", DM, STRANGER, "Asing", "gesture-2", Decision(row.id, True), hints={"dm": True}
    )
    handle(env, percept)

    assert env.agent.approvals.get(row.id).status == "pending"
    assert (DM, t("command.approve.foreign", env.config.soul.language)) in texts(env)
    assert OTHER not in [chat for chat, _text in texts(env)]

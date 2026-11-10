"""Approval contract: origin-tracked exit door, three tiers, and structured decisions.

ADR-0004 §4.2 / ADR-0006 §4.5: every outbound action names the room it was asked in, the
approval rows carry that origin plus their action/batch ids, and only a control or a typed
command decides a staged action.
"""


import asyncio
import json
from types import SimpleNamespace

import pytest
from support import DirectoryFakeGateway, FakeLLM, inbound_message, make_agent, role

from monoagens.engine.commands.approve import COMMAND as APPROVE
from monoagens.engine.commands.approve import decision_refusal
from monoagens.engine.config import ApprovalConfig
from monoagens.engine.guard import ActionRequest, ApprovalPolicy
from monoagens.engine.i18n import t
from monoagens.engine.inbound import Action, Address, SendRefused
from monoagens.engine.registry import Handler
from monoagens.engine.tools import REGISTRY as TOOLS
from monoagens.engine.tools import execute
from monoagens.engine.turn.delivery import bind_origin, reset_origin

DM = "chat"
OTHER = "other"
PERSON = "628111"


# --------------------------------------------------------------- helpers


def send_action(text: str = "ringkasan", *, action_id: str = "a-1", thread: str = OTHER):
    return Action(
        address=Address("fake", thread), capability="send_text", payload=text, id=action_id
    )


def door_request(
    declared: str, *, gateway: str = "fake", origin: str = "fake", thread: str = "chat"
):
    return ActionRequest(
        tool="send",
        args={"chat": thread, "capability": "send_text", "action_id": "x"},
        thread_id=thread,
        sender="p",
        gateway=gateway,
        origin_gateway=origin,
        roles=("member",),
        declared_class=declared,
        door=True,
    )


def tool_request(tool: str, *, target: str = "", thread: str = "chat", **args):
    spec = TOOLS.by_name.get(tool)
    return ActionRequest(
        tool=tool,
        args=args,
        thread_id=thread,
        sender="p",
        gateway="fake",
        roles=("member",),
        target=target or (spec.target if spec is not None else ""),
        declared_class=spec.approval_class if spec is not None else "authorization",
        destructive=spec.destructive if spec is not None else (),
        path_arg=spec.path_arg if spec is not None else "",
        scan=spec.scan if spec is not None else (),
    )


def verdict(config: ApprovalConfig, request: ActionRequest) -> str:
    return ApprovalPolicy(config).check(request).verdict


async def _send_to(args: dict, ctx) -> str:
    """A body tool that sends to another room through the one door."""
    address = Address(str(ctx.gateway.name), str(args.get("chat") or ""))
    action = Action(
        address=address,
        capability="send_text",
        payload=str(args.get("text") or ""),
        id=f"turn:{ctx.turn_id}:send:{ctx.used.count('send_to')}",
    )
    receipt = await ctx.agent.dispatch(
        ctx.sender,
        address,
        action,
        origin=Address(str(ctx.gateway.name), str(ctx.thread_id)),
        reason=str(args.get("reason") or "the requester asked for it"),
    )
    if receipt.state == "sent":
        return "ok: sent"
    if receipt.state == "pending":
        return f"pending approval: {receipt.reason}"
    return f"error: {receipt.reason}"


SEND_TOOL = Handler(
    name="send_to",
    family="write",
    target="room",
    approval_class="consent",
    description="Send text to another chat through the door.",
    fn=_send_to,
    parameters={
        "type": "object",
        "properties": {
            "chat": {"type": "string"},
            "text": {"type": "string"},
            "reason": {"type": "string"},
        },
        "required": ["chat", "text"],
        "additionalProperties": False,
    },
)


class SendToolLLM(FakeLLM):
    """Plans a send on the first turn and stages again when the user asks for a change."""

    def __init__(self, *, texts: tuple[str, ...] = ("ringkasan",), **kwargs) -> None:
        super().__init__(**kwargs)
        self.texts = texts
        self.contents: list[object] = []
        self.tool_results: list[str] = []
        self.ran = 0

    async def run(self, system, content, specs, ctx, session=None, **kwargs):
        self.contents.append(content)
        self.ctx = ctx
        body = str(content)
        if '"status": "superseded"' in body and self.texts:
            result = await execute(
                {handler.name: handler for handler in specs},
                "send_to",
                {"chat": OTHER, "text": "versi pendek"},
                ctx,
            )
            self.tool_results.append(result)
            return "Sudah kuperbarui. Kirim versi pendeknya?"
        if self.ran < len(self.texts):
            text = self.texts[self.ran]
            self.ran += 1
            result = await execute(
                {handler.name: handler for handler in specs},
                "send_to",
                {"chat": OTHER, "text": text},
                ctx,
            )
            self.tool_results.append(result)
            return f"Aku sudah siapkan ringkasan. Mau kukirim ke grup lain? ({result})"
        return "Sudah kukirim, beres."


def handle(env, inbound):
    return asyncio.run(env.agent.handle(inbound, env.gateway))


def send_env(tmp_path, *, mode: str = "yolo", llm=None, members=("628111",)):
    gateway = DirectoryFakeGateway(members=set(members), tools=(SEND_TOOL,))
    env = make_agent(
        tmp_path,
        gateway=gateway,
        approvals=ApprovalConfig(mode=mode),
        roles={
            "member": role("member", users=(PERSON,), tools=("*",), commands=("*",)),
            "owner": role("owner", users=("628222",), tools=("*",), commands=("*",)),
            "guest": role("guest", users=("628333",), tools=("read",), commands=("approve",)),
        },
    )
    env.agent.llm = llm if llm is not None else SendToolLLM()
    if isinstance(gateway.members, set):
        gateway.members.add(env.agent.identity.resolve("fake", PERSON))
    return env


def person_of(env) -> str:
    """The canonical person id the turn loop stages approvals under."""
    return env.agent.identity.resolve("fake", PERSON)


# ---------------------------------------------------- explicit origin door


def test_a_cross_room_send_without_an_explicit_origin_fails(tmp_path) -> None:
    env = send_env(tmp_path)
    action = send_action()

    with pytest.raises(TypeError):
        asyncio.run(env.agent.dispatch(PERSON, action.address, action))
    with pytest.raises(TypeError):
        env.agent.delivery._door(action, subject=PERSON)


def test_a_bound_turn_origin_parks_a_tool_facing_cross_room_send(tmp_path) -> None:
    env = send_env(tmp_path, mode="default")
    token = bind_origin(Address("fake", DM))
    try:
        with pytest.raises(SendRefused):
            asyncio.run(
                env.agent.delivery.text(
                    OTHER,
                    "rahasia proyek",
                    gateway=env.gateway,
                    subject=PERSON,
                    reason="teruskan",
                )
            )
    finally:
        reset_origin(token)

    rows = env.agent.approvals.pending(OTHER, PERSON)
    assert len(rows) == 1
    row = rows[0]
    assert row.origin_thread == DM
    assert row.thread_id == OTHER
    assert env.gateway.sent == []


def test_the_same_action_id_parks_once(tmp_path) -> None:
    env = send_env(tmp_path, mode="default")

    first = asyncio.run(
        env.agent.dispatch(
            PERSON, Address("fake", OTHER), send_action(), origin=Address("fake", DM), reason="r"
        )
    )
    second = asyncio.run(
        env.agent.dispatch(
            PERSON, Address("fake", OTHER), send_action(), origin=Address("fake", DM), reason="r"
        )
    )

    assert first.state == second.state == "pending"
    assert first.reason == second.reason
    assert env.agent.approvals.count_pending() == 1
    row = env.agent.approvals.find_action("a-1")
    assert row is not None
    assert row.action_id == "a-1"
    assert row.batch_id


def test_approval_runs_from_the_origin_and_the_target_room(tmp_path) -> None:
    env = send_env(tmp_path, mode="default")
    # III.3 makes any wildcard-tool role privileged, so this test needs a plain member.
    member = role("member", users=(PERSON,), tools=("send_to",), commands=("*",))
    lang = env.config.soul.language
    foreign = t("command.approve.foreign", lang)

    def park(action_id: str):
        asyncio.run(
            env.agent.dispatch(
                PERSON,
                Address("fake", OTHER),
                send_action(action_id=action_id),
                origin=Address("fake", DM),
                reason="kirim",
            )
        )
        return env.agent.approvals.find_action(action_id)

    origin_row = park("s-origin")
    ctx = env.agent.command_context(DM, PERSON, (member,), env.gateway, str(origin_row.id))
    result = asyncio.run(APPROVE.fn(ctx))
    assert f"#{origin_row.id}" in result
    assert env.agent.approvals.get(origin_row.id).status == "approved"

    target_row = park("s-target")
    ctx = env.agent.command_context(OTHER, PERSON, (member,), env.gateway, str(target_row.id))
    result = asyncio.run(APPROVE.fn(ctx))
    assert f"#{target_row.id}" in result
    assert env.agent.approvals.get(target_row.id).status == "approved"

    unrelated = park("s-stranger")
    ctx = env.agent.command_context("stranger", PERSON, (member,), env.gateway, str(unrelated.id))
    assert asyncio.run(APPROVE.fn(ctx)) == foreign
    assert env.agent.approvals.get(unrelated.id).status == "pending"
    assert [chat for chat, _text, _reply in env.gateway.sent] == [OTHER, OTHER]


def test_an_approver_with_target_authority_may_decide_elsewhere(tmp_path) -> None:
    env = send_env(tmp_path, mode="default")
    owner = env.config.roles.get("owner")
    guest = env.config.roles.get("guest")

    asyncio.run(
        env.agent.dispatch(
            PERSON,
            Address("fake", OTHER),
            send_action(action_id="owned"),
            origin=Address("fake", DM),
            reason="kirim",
        )
    )
    row = env.agent.approvals.find_action("owned")
    allowed = env.agent.command_context(DM, "628222", (owner,), env.gateway, str(row.id))
    assert f"#{row.id}" in asyncio.run(APPROVE.fn(allowed))
    assert env.agent.approvals.get(row.id).status == "approved"

    asyncio.run(
        env.agent.dispatch(
            PERSON,
            Address("fake", OTHER),
            send_action(action_id="not-owned"),
            origin=Address("fake", DM),
            reason="kirim",
        )
    )
    other = env.agent.approvals.find_action("not-owned")
    refused = env.agent.command_context(DM, "628222", (guest,), env.gateway, str(other.id))
    assert asyncio.run(APPROVE.fn(refused)) == t(
        "command.approve.foreign", env.config.soul.language
    )
    assert env.agent.approvals.get(other.id).status == "pending"


# ------------------------------------------------------- privacy decisions


def privacy_row(kind: str = "privacy", *, sender: str = PERSON, thread: str = DM):
    """A question about one person's memory: it names its room and the person to answer."""
    return SimpleNamespace(
        kind=kind, sender=sender, thread_id=thread, origin_thread="", tool=""
    )


def test_the_person_a_privacy_row_names_decides_it() -> None:
    row = privacy_row()
    assert decision_refusal(row, thread_id=DM, sender=PERSON, roles=()) == ""


def test_no_privileged_role_answers_a_privacy_row_for_the_person() -> None:
    row = privacy_row()
    protected = role("owner", protected=True)
    wildcard = role("admin", tools=("*",))
    for stand_in in (protected, wildcard):
        assert decision_refusal(
            row, thread_id=DM, sender="628222", roles=(stand_in,)
        ) == "foreign"


def test_no_privileged_role_answers_a_disclosure_row_for_the_person() -> None:
    row = privacy_row("disclosure")
    owner = role("owner", protected=True)
    assert decision_refusal(row, thread_id=DM, sender="628222", roles=(owner,)) == "foreign"


def test_a_privacy_row_is_only_decided_from_its_own_rooms() -> None:
    row = privacy_row()
    assert decision_refusal(row, thread_id="stranger", sender=PERSON, roles=()) == "foreign"


def test_a_privileged_role_still_decides_other_kinds_for_the_requester() -> None:
    row = SimpleNamespace(
        kind="send", sender=PERSON, thread_id=DM, origin_thread="", tool="send_to"
    )
    owner = role("owner", protected=True)
    assert decision_refusal(row, thread_id=DM, sender="628222", roles=(owner,)) == ""


def test_a_cross_room_payload_that_quotes_the_origin_is_refused(tmp_path) -> None:
    env = send_env(tmp_path, mode="default")
    leaked = "rahasia proyek alpha jadwal rapat besok pagi pukul sembilan"
    env.history.intake(
        DM, inbound_message(leaked, thread=DM).payload, PERSON, language="id"
    )

    receipt = asyncio.run(
        env.agent.dispatch(
            PERSON,
            Address("fake", OTHER),
            send_action(leaked, action_id="leak"),
            origin=Address("fake", DM),
            reason="teruskan",
        )
    )

    assert receipt.state == "refused"
    assert "room lain" in receipt.reason
    assert env.agent.approvals.find_action("leak") is None
    assert env.gateway.sent == []

    # The same text inside the origin room is not a crossing and still goes out.
    same = asyncio.run(
        env.agent.dispatch(
            PERSON,
            Address("fake", DM),
            send_action(leaked, action_id="same-room", thread=DM),
            origin=Address("fake", DM),
        )
    )
    assert same.state == "sent"


def test_approval_rows_default_their_origin_to_the_current_room(tmp_path) -> None:
    env = send_env(tmp_path)
    sender = person_of(env)

    row = asyncio.run(
        env.agent.approvals.create(
            kind="tool", thread_id=DM, sender=sender, gateway="fake", tool="write", args={}
        )
    )

    assert (row.origin_gateway, row.origin_thread) == ("fake", DM)
    assert row.action_id == ""
    rows = env.agent.approvals.pending(DM, sender)
    assert [item.id for item in rows] == [row.id]


# ------------------------------------------------------------ tier matrix


def test_default_tier_matrix() -> None:
    config = ApprovalConfig(mode="default")

    assert verdict(config, door_request("consent")) == "allow"
    assert verdict(config, door_request("authorization")) == "pending"
    assert verdict(config, tool_request("read", path="public/a.txt")) == "allow"
    assert verdict(config, tool_request("web_search", query="x")) == "allow"
    assert verdict(config, tool_request("memory", action="search", query="x")) == "allow"
    assert verdict(config, tool_request("write", path="scratch/a.txt")) == "pending"
    assert verdict(config, tool_request("shell", command="ls")) == "pending"
    assert verdict(config, tool_request("sandbox", command="ls")) == "pending"
    assert verdict(config, tool_request("memory", action="forget", query="x")) == "pending"
    # ADR-0013 §4.5: a settings read observes; a settings write still waits.
    assert verdict(config, tool_request("settings", action="get")) == "allow"
    assert verdict(config, tool_request("settings", action="set")) == "pending"
    assert verdict(config, tool_request("send", target="room", chat=DM)) == "allow"
    assert verdict(config, tool_request("send_to", target="room", chat=OTHER)) == "pending"


def test_yolo_tier_matrix() -> None:
    config = ApprovalConfig(mode="yolo")

    assert verdict(config, tool_request("read", path="public/a.txt")) == "allow"
    assert verdict(config, tool_request("write", path="scratch/a.txt")) == "allow"
    assert verdict(config, tool_request("shell", command="ls")) == "allow"
    assert verdict(config, tool_request("memory", action="forget")) == "allow"
    assert verdict(config, tool_request("shell", command="rm -rf x")) == "pending"
    assert verdict(config, tool_request("settings", action="get")) == "allow"
    assert verdict(config, tool_request("settings", action="set")) == "pending"
    assert verdict(config, tool_request("send_to", target="room", chat=OTHER)) == "pending"
    assert verdict(config, tool_request("task", task="x")) == "pending"

    trusted = ApprovalConfig(
        mode="yolo", confirm_cross_gateway=False, confirm_tasks=False, settings=False
    )
    assert verdict(trusted, tool_request("send_to", target="room", chat=OTHER)) == "allow"
    assert verdict(trusted, tool_request("task", task="x")) == "allow"
    assert verdict(trusted, tool_request("settings", action="set")) == "allow"


def test_strict_tier_matrix() -> None:
    config = ApprovalConfig(mode="strict")

    assert verdict(config, door_request("consent")) == "allow"
    assert verdict(config, door_request("authorization")) == "pending"
    assert verdict(config, tool_request("read", path="public/a.txt")) == "pending"
    assert verdict(config, tool_request("memory", action="search", query="x")) == "pending"


def test_strict_mode_allows_an_origin_reply_and_parks_a_tool_call(tmp_path) -> None:
    env = make_agent(tmp_path, approvals=ApprovalConfig(mode="strict"))

    delivered = asyncio.run(
        env.agent.delivery.text(
            DM,
            "halo juga",
            gateway=env.gateway,
            origin=Address(env.gateway.name, DM),
        )
    )

    assert delivered == "msg-1"
    assert env.gateway.sent == [(DM, "halo juga", None)]

    member = env.config.roles.get("member")
    ctx = env.agent.tool_context(DM, PERSON, (member,), env.gateway)
    specs = {item.name: item for item in env.agent.tools.by_name.values()}
    result = asyncio.run(execute(specs, "read", {"path": "public/a.txt"}, ctx))

    assert result.startswith("pending approval #")
    assert env.agent.approvals.count_pending() == 1


# ------------------------------------------------ no keyword confirmation


def test_a_short_yes_does_not_decide_a_pending_row(tmp_path) -> None:
    llm = SendToolLLM()
    env = send_env(tmp_path, llm=llm)

    handle(env, inbound_message("kirim ringkasan ke grup lain", thread=DM, message_id="t1"))
    row = env.agent.approvals.pending(DM, person_of(env))[0]

    handle(env, inbound_message("Ok, send it", thread=DM, message_id="t2"))

    assert env.agent.approvals.get(row.id).status == "pending"
    assert all(chat != OTHER for chat, _text, _reply in env.gateway.sent)
    block = str(llm.contents[-1])
    assert '"type": "pending_approval"' in block
    assert '"status": "waiting"' in block


def test_a_reject_word_does_not_cancel_a_pending_row(tmp_path) -> None:
    llm = SendToolLLM()
    env = send_env(tmp_path, llm=llm)

    handle(env, inbound_message("kirim ringkasan ke grup lain", thread=DM, message_id="r1"))
    row = env.agent.approvals.pending(DM, person_of(env))[0]

    handle(env, inbound_message("cancel it", thread=DM, message_id="r2"))

    assert env.agent.approvals.get(row.id).status == "pending"


def test_an_unrelated_message_leaves_the_staged_send_waiting(tmp_path) -> None:
    llm = SendToolLLM()
    env = send_env(tmp_path, llm=llm)

    handle(env, inbound_message("kirim ringkasan ke grup lain", thread=DM, message_id="w1"))
    row = env.agent.approvals.pending(DM, person_of(env))[0]

    handle(env, inbound_message("nanti aku cek sendiri ya", thread=DM, message_id="w2"))

    assert env.agent.approvals.get(row.id).status == "pending"
    assert '"status": "waiting"' in str(llm.contents[-1])
    assert "Sudah kukirim, beres." in " ".join(
        text for chat, text, _reply in env.gateway.sent if chat == DM
    )


def test_the_parked_approval_rides_the_reply_as_data(tmp_path) -> None:
    llm = SendToolLLM()
    env = send_env(tmp_path, llm=llm)

    handle(env, inbound_message("kirim ringkasan ke grup lain", thread=DM, message_id="g1"))

    row = env.agent.approvals.pending(DM, person_of(env))[0]
    reply = [action for action in env.gateway.dispatched if action.capability == "send_text"][-1]
    assert reply.meta["ui"] == {"kind": "approval", "approval_id": row.id}


def test_a_modified_action_is_not_json_serialized_twice(tmp_path) -> None:
    """The pending block is JSON, so a summary with quotes still round-trips."""
    llm = SendToolLLM()
    env = send_env(tmp_path, llm=llm)

    handle(env, inbound_message("kirim ringkasan ke grup lain", thread=DM, message_id="j1"))
    handle(env, inbound_message("nanti aku cek sendiri ya", thread=DM, message_id="j2"))

    block = str(llm.contents[-1]).split("## Tool result: pending_approval\n", 1)[-1]
    payload = json.loads(block.split("\n\nThread ", 1)[0])
    assert payload["type"] == "pending_approval"
    assert payload["status"] == "waiting"

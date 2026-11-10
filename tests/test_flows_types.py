"""Pure flow types: outcomes, requirements, context, and definitions (ADR-0013 §4)."""


from monoagens.engine.flows import (
    AccountProviders,
    Approval,
    Connect,
    Done,
    ExternalAction,
    Flow,
    FlowContext,
    FlowRegistry,
    Needs,
    OperatorConfig,
    Refused,
    Resume,
    requirement_from_json,
)


async def ping(ctx: FlowContext) -> Done:
    return Done()


def test_every_requirement_round_trips_exactly() -> None:
    samples = (
        Approval(deciders=("p1", "p2"), action="edit the file"),
        Approval(deciders=("p1",), action="share", expires_in=60.0),
        Connect(account_kind="google"),
        Connect(account_kind="google", instruction="sign in with your work account"),
        ExternalAction(instruction="share the file as editor", verify="check_share"),
        OperatorConfig(keys=("GOOGLE_KEY", "GOOGLE_SUBJECT")),
    )
    for requirement in samples:
        assert requirement_from_json(requirement.to_json()) == requirement


def test_an_unknown_requirement_kind_fails_closed() -> None:
    for body in ({"kind": "teleport"}, {"kind": ""}, {}, None, "approval"):
        try:
            requirement_from_json(body)
        except ValueError:
            continue
        raise AssertionError(f"{body!r} should not parse")


def test_a_malformed_requirement_fails_closed() -> None:
    bad = (
        {"kind": "approval", "deciders": [], "action": "x"},
        {"kind": "approval", "deciders": ["p1"], "action": ""},
        {"kind": "approval", "deciders": ["p1"], "action": "x", "expires_in": "soon"},
        {"kind": "connect", "account_kind": ""},
        {"kind": "external", "instruction": "do it"},
        {"kind": "config", "keys": "GOOGLE_KEY"},
    )
    for body in bad:
        try:
            requirement_from_json(body)
        except ValueError:
            continue
        raise AssertionError(f"{body!r} should not parse")


def test_the_subclass_expected_kind_is_enforced() -> None:
    body = Connect(account_kind="google").to_json()
    assert Approval.from_json(Approval(deciders=("p",), action="a").to_json()).kind == "approval"
    try:
        Approval.from_json(body)
    except ValueError:
        return
    raise AssertionError("a Connect must not parse as an Approval")


def test_the_registry_refuses_a_duplicate_name() -> None:
    registry = FlowRegistry()
    flow = Flow(name="core/ping", steps={"start": ping})
    assert registry.register(flow) is flow
    assert registry.get("core/ping") is flow
    assert "core/ping" in registry
    try:
        registry.register(Flow(name="core/ping", steps={"start": ping}))
    except ValueError:
        return
    raise AssertionError("a duplicate flow name must be refused")


def test_a_flow_name_needs_an_owner() -> None:
    for name in ("ping", "core/", "/ping", "Core/Ping"):
        try:
            Flow(name=name, steps={"start": ping})
        except ValueError:
            continue
        raise AssertionError(f"'{name}' should be refused")


def test_a_flow_must_start_at_a_known_step() -> None:
    try:
        Flow(name="core/ping", steps={"start": ping}, first="missing")
    except ValueError:
        return
    raise AssertionError("an unknown first step must be refused")


def test_account_providers_fail_closed() -> None:
    accounts = AccountProviders()
    assert accounts.connected("google", "p1") is False
    accounts.register("google", lambda person: person == "p1")

    def broken(person: str) -> bool:
        raise RuntimeError("probe down")

    accounts.register("broken", broken)
    assert accounts.connected("google", "p1") is True
    assert accounts.connected("google", "p2") is False
    assert accounts.connected("broken", "p1") is False
    assert accounts.kinds() == ("broken", "google")


def test_outcomes_are_frozen_data() -> None:
    resume = Resume(step="after", state={"k": "v"})
    outcome = Needs(Connect(account_kind="google"), resume)
    assert outcome.resume.step == "after"
    assert Done().result == ""
    assert Refused("no access").reason == "no access"

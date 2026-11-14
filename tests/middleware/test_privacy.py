"""Privacy: the audience a place reports, the one items check, and consent to share.

Every test feeds percepts and inspects what reached the desk, the receipts and the Log: the
contract's surfaces only (contract.md §7).
"""

from __future__ import annotations

from dataclasses import replace
from datetime import datetime, timezone

from monoagens.boot import build
from monoagens.contract import ContextItem
from monoagens.core.context.layout import PFCContext
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event, read_state, write_state
from monoagens.middleware import privacy
from monoagens.types import Event
from tests.support import context, settings

START = datetime(2026, 11, 11, 8, 0, tzinfo=timezone.utc)
SETTINGS = settings()
OWNER = "person:owner"
BUDI = "space:wa:budi"
GROUP = "space:tg:-500"
OTHER = "space:tg:-900"


async def _audience(conn, place: str, actors, *, unknown: bool = False) -> int:
    """Report one place's audience, the one fact privacy reads."""
    return int((await append_event(
        conn, ts=START, kind="place.audience", place=place,
        payload={"place": place, "actors": list(actors), "unknown": unknown})).id)


async def _said(conn, who: str, place: str, text: str = "halo") -> int:
    """Record one percept at a place."""
    return int((await append_event(conn, ts=START, kind="percept", who=who, place=place,
                                   body="telegram", description=text)).id)


async def _filtered(kernel, conn, desk: PFCContext) -> PFCContext | None:
    """Run the items rule the owner policy installs last in the context hook."""
    from monoagens.contract.hooks import Continue, Replace

    outcome = await privacy.items_rule(kernel)(desk, {"conn": conn,
                                                      "event": desk.trigger, "now": START})
    if isinstance(outcome, Replace):
        return outcome.item
    return desk if isinstance(outcome, Continue) else None


def _kept(desk: PFCContext) -> list[str]:
    """Return the item text one filtered desk kept, notes aside."""
    return [item.text for item in desk.items if item.section != "notes"]


def _notes(desk: PFCContext) -> list[str]:
    return [item.text for item in desk.items if item.section == "notes"]


# ------------------------------------------------------------- the one fact


async def test_a_place_without_an_audience_is_unknown(aconn):
    """No report means unknown, which lets no private item in."""
    await _said(aconn, "telegram:1", GROUP)

    assert await privacy.audience(aconn, GROUP) is None


async def test_an_audience_reports_person_ids_and_goes_unknown_on_request(aconn):
    """A body that cannot list its members reports `unknown`, and that is the fact."""
    await _audience(aconn, GROUP, [OWNER, "person:budi"])

    assert await privacy.audience(aconn, GROUP) == frozenset({OWNER, "person:budi"})

    await _audience(aconn, GROUP, [], unknown=True)

    assert await privacy.audience(aconn, GROUP) is None


async def test_the_newest_report_before_a_moment_is_the_audience_then(aconn):
    """A newcomer never receives what was said before they could perceive it."""
    await _audience(aconn, GROUP, [OWNER])
    before = await _said(aconn, "person:owner", GROUP, "rahasia berdua")
    await _audience(aconn, GROUP, [OWNER, "person:charlie"])

    assert await privacy.audience(aconn, GROUP, before) == frozenset({OWNER})
    assert await privacy.audience(aconn, GROUP) == frozenset({OWNER, "person:charlie"})


async def test_the_owner_direct_place_is_the_owner_alone(aconn):
    """A place whose audience is only the owner is the owner's own direct place."""
    await _audience(aconn, BUDI, [OWNER])
    await _audience(aconn, GROUP, ["person:budi"])

    rows = await (await aconn.execute("SELECT place, payload FROM events WHERE kind='place.audience'")).fetchall()
    print("ROWS", rows)
    print("AU", await privacy.audience(aconn, BUDI))
    assert await privacy.owner_direct(aconn, BUDI) is True
    assert await privacy.owner_direct(aconn, GROUP) is False


async def test_a_task_thread_takes_its_parents_audience(aconn):
    """A thread the core opens has no report of its own: privacy reads its parent place."""
    report = await _audience(aconn, GROUP, [OWNER, "person:budi"])
    await write_state(aconn, "task:space:task:9", {"parent": GROUP, "status": "open"}, report)

    assert await privacy.audience(aconn, "space:task:9") == frozenset({OWNER, "person:budi"})


# ------------------------------------------------------------ the one check


def test_the_predicate_admits_only_inside_the_target_audience():
    """One predicate holds the rule; an unknown source is not private to anyone."""
    assert privacy.admits(frozenset({OWNER}), frozenset({"person:x"})) is True
    assert privacy.admits(None, None) is True
    assert privacy.admits(frozenset({OWNER, "person:a"}), None) is True
    assert privacy.admits(None, frozenset({OWNER})) is False
    assert privacy.admits(frozenset({OWNER, "person:a"}),
                          frozenset({OWNER, "person:a", "person:b"})) is True
    assert privacy.admits(frozenset({OWNER, "person:a", "person:b"}),
                          frozenset({OWNER, "person:a"})) is False


async def test_an_item_from_a_private_place_is_withheld_from_a_group(aconn):
    """The one check drops the item and says how many and from where, never what."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, BUDI, [OWNER, "person:budi"])
    private = await _said(aconn, "person:budi", BUDI, "gajiku belum turun")
    await _audience(aconn, GROUP, [OWNER, "person:charlie"])
    desk = context(now=START, place=GROUP, reason="test", recent_elsewhere=(
        Event(id=private, ts=START, kind="percept", who="person:budi", place=BUDI,
              description="gajiku belum turun"),))

    filtered = await _filtered(kernel, aconn, desk)

    assert "gajiku belum turun" not in _kept(filtered)
    assert _notes(filtered) == ["1 thing you know from space:wa:budi is not shown here"]


async def test_an_item_said_in_this_place_is_admitted(aconn):
    """What was said where it is shown needs no consent."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, GROUP, [OWNER, "person:budi"])
    line = await _said(aconn, "person:budi", GROUP, "rapat jam sembilan")
    desk = context(now=START, place=GROUP, reason="test", recent=(
        Event(id=line, ts=START, kind="percept", who="person:budi", place=GROUP,
              description="rapat jam sembilan"),))

    filtered = await _filtered(kernel, aconn, desk)

    assert _kept(filtered) == ["08:00 person:budi: rapat jam sembilan"]


async def test_content_from_an_unknown_place_flows_freely(aconn):
    """Content whose audience was unknown is not private to anyone (middleware.md §3)."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, OTHER, [], unknown=True)
    line = await _said(aconn, "person:someone", OTHER, "kabar umum")
    await _audience(aconn, GROUP, [OWNER, "person:charlie"])
    desk = context(now=START, place=GROUP, reason="test", recent_elsewhere=(
        Event(id=line, ts=START, kind="percept", who="person:someone", place=OTHER,
              description="kabar umum"),))

    filtered = await _filtered(kernel, aconn, desk)

    assert "kabar umum" in " ".join(_kept(filtered))


async def test_a_place_with_an_unknown_audience_keeps_only_unknown_items(aconn):
    """An unknown place keeps what was said there while it was unknown, and nothing known."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    own = await _said(aconn, "person:someone", GROUP, "obrolan grup")
    await _audience(aconn, BUDI, [OWNER, "person:budi"])
    known = await _said(aconn, "person:budi", BUDI, "hal pribadi")
    desk = context(now=START, place=GROUP, reason="test",
                   recent=(Event(id=own, ts=START, kind="percept", who="person:someone",
                                 place=GROUP, description="obrolan grup"),),
                   recent_elsewhere=(Event(id=known, ts=START, kind="percept",
                                           who="person:budi", place=BUDI,
                                           description="hal pribadi"),))

    filtered = await _filtered(kernel, aconn, desk)

    assert len(_kept(filtered)) == 1 and "obrolan grup" in _kept(filtered)[0]


async def test_the_owners_own_place_sees_everything(aconn):
    """The owner's direct place is a view of everything, not ownership (D45)."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, BUDI, [OWNER, "person:budi"])
    line = await _said(aconn, "person:budi", BUDI, "rahasia")
    await _audience(aconn, OTHER, [OWNER])
    desk = context(now=START, place=OTHER, reason="test", recent_elsewhere=(
        Event(id=line, ts=START, kind="percept", who="person:budi", place=BUDI,
              description="rahasia"),))

    filtered = await _filtered(kernel, aconn, desk)

    assert "rahasia" in " ".join(_kept(filtered))


async def test_a_memory_is_judged_by_the_events_it_was_derived_from(aconn):
    """A memory's audience comes from its sources (D53)."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, BUDI, [OWNER, "person:budi"])
    source = await _said(aconn, "person:budi", BUDI, "pindah kantor")
    await _audience(aconn, GROUP, [OWNER, "person:charlie"])
    desk = context(now=START, place=GROUP, reason="test", memories=("- pindah kantor",))
    desk = replace(desk, items=tuple(
        replace(item, sources=(source,)) if item.section == "memories" else item
        for item in desk.items))

    filtered = await _filtered(kernel, aconn, desk)

    assert not [item for item in filtered.items if item.section == "memories"]
    assert _notes(filtered)


async def test_a_person_card_is_never_withheld(aconn):
    """A card only names people already here, so it is not a private item."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, GROUP, [OWNER, "person:budi"])
    desk = context(now=START, place=GROUP, reason="test",
                   people=("- Budi (person:budi): no direct place known yet",))

    filtered = await _filtered(kernel, aconn, desk)

    assert _kept(filtered) == ["- Budi (person:budi): no direct place known yet"]


async def test_a_hidden_trigger_stops_the_wake(aconn):
    """A wake whose own event may not be shown here stops, and the caller records it."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, BUDI, [OWNER, "person:budi"])
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:budi",
                                 place=BUDI, body="telegram", description="rahasia")
    await _audience(aconn, GROUP, [OWNER, "person:charlie"])
    desk = context(now=START, place=GROUP, reason="test",
                   trigger=replace(trigger, id=int(trigger.id)))

    assert await _filtered(kernel, aconn, desk) is None


# ------------------------------------------------------------ consent to share


async def test_share_grants_only_the_named_events_to_the_named_person(aconn):
    """`share` names event ids; only someone who was part of them may share them (D46)."""
    await _audience(aconn, BUDI, [OWNER, "person:budi"])
    line = await _said(aconn, "person:budi", BUDI, "laporan final")
    event = Event(id=line + 1, ts=START, kind="percept", who="person:owner",
                  place=BUDI, origin_actor=OWNER)

    allowed, source, ids = await privacy.consent(aconn, event, {"items": [line]})

    assert allowed is True and source == BUDI and ids == (line,)


async def test_share_is_refused_for_content_the_actor_was_not_part_of(aconn):
    """Another person's private line cannot be shared by someone who was not there."""
    await _audience(aconn, BUDI, [OWNER, "person:budi"])
    line = await _said(aconn, "person:budi", BUDI, "rahasia budi")
    event = Event(id=line + 1, ts=START, kind="percept", who="person:charlie",
                  place=GROUP, origin_actor="person:charlie")

    allowed, _, _ = await privacy.consent(aconn, event, {"items": [line]})

    assert allowed is False


async def test_a_grant_lets_the_named_event_into_the_target_place(aconn):
    """A recorded grant adds the people it names to that event's audience."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, BUDI, [OWNER, "person:budi"])
    line = await _said(aconn, "person:budi", BUDI, "laporan final")
    await _audience(aconn, GROUP, [OWNER, "person:charlie"])
    stored = Event(id=line, ts=START, kind="percept", who="person:budi", place=BUDI)
    await privacy.grant(kernel, stored, {"items": [line]}, to=GROUP, source=BUDI,
                        episode=[line])

    assert await privacy.visible(aconn, context=GROUP, source=BUDI, before=line) is True
    assert await privacy.visible(aconn, context=GROUP, source=BUDI) is False


async def test_share_without_an_id_is_refused_with_a_question(aconn):
    """A share that names nothing is refused, never guessed (contract.md §10)."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    outcome = await privacy.guard(kernel, "first")(
        {"to": "person:budi"},
        {"conn": aconn, "event": Event(id=1, ts=START, kind="percept"),
         "capability": privacy.SHARE})

    assert outcome.reason == "which event ids do you want to share?"


async def test_an_episode_that_is_not_an_id_is_refused():
    """`episode` is one event id."""
    assert privacy.episode_error({"episode": "kemarin"}) != ""
    assert privacy.episode_error({"episode": 12}) == ""


# ------------------------------------------------ asking on someone's behalf (D80)


async def test_the_one_answer_to_a_send_carries_its_requester(aconn):
    """D80: the event that answers the soul's message is labelled with who asked for it."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, BUDI, [OWNER])
    send = await append_event(aconn, ts=START, kind="act.out", who=OWNER, place=BUDI,
                              body="telegram", description="laporan?", origin_actor=OWNER,
                              payload={"communicative": True})
    answer = await append_event(aconn, ts=START, kind="percept", who="person:budi",
                                place=BUDI, body="telegram", description="sudah kukirim",
                                payload={"about": str(send.id)})

    await privacy.delegation(kernel)(answer, {"conn": aconn, "now": START})

    assert await read_state(aconn, f"delegated:{int(answer.id)}") == {"actors": [OWNER]}


# -------------------------------------------------- speaking elsewhere (D45)


async def test_an_intent_is_replaced_when_its_source_is_private(aconn):
    """An intent quotes its words only where their source may be shown (D45)."""
    kernel = build(aconn, clock=FakeClock(START), settings=SETTINGS)
    await _audience(aconn, BUDI, [OWNER, "person:budi"])
    source = await _said(aconn, "person:budi", BUDI, "rahasia gaji")
    await _audience(aconn, GROUP, [OWNER, "person:charlie"])
    sent = await append_event(aconn, ts=START, kind="act.out", who=OWNER, place=BUDI,
                              body="telegram", description="draft")
    asked = await append_event(aconn, ts=START, kind="act.intent", who=OWNER, place=BUDI,
                               description="intent", caused_by=sent.id)
    intent = await append_event(aconn, ts=START, kind="intent", place=GROUP,
                                description="you set out to say 'rahasia gaji' to charlie",
                                caused_by=asked.id, payload={"to": GROUP})
    kernel._wake_private_sources = {(int(sent.id), BUDI): ((BUDI, source),)}

    outcome = await privacy.intent_words(kernel)(intent, {"conn": aconn, "now": START})

    assert "rahasia gaji" not in outcome.item.description
    assert "cannot appear here" in outcome.item.description


# ------------------------------------------------------------ attachments


def test_a_capability_without_a_file_has_nothing_to_confirm():
    """No stored file in the arguments means nothing to confirm."""
    assert privacy.carries_file({"text": "hello"}, {"communicative": True}) is False
    assert privacy.carries_file({"text": "hello"}, {}) is False

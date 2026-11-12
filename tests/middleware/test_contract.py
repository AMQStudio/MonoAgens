"""The read-only view a plugin gets: the Log and the state board, never a connection."""

from __future__ import annotations

from datetime import datetime, timezone

from monoagens.infra.store import append_event, write_state
from monoagens.middleware import contract
from monoagens.middleware.loader import PluginAPI

START = datetime(2026, 1, 1, tzinfo=timezone.utc)
PLACE = "space:tg:1"


async def _seed(conn) -> None:
    first = await append_event(conn, ts=START, kind="percept", who="person:tg:1", place=PLACE,
                               body="telegram",
                               payload={"description": "halo pagi", "place_kind": "direct"})
    await append_event(conn, ts=START, kind="percept", who="person:tg:1", place=PLACE,
                       body="telegram",
                       payload={"description": "kabar kopi", "place_kind": "direct"})
    await append_event(conn, ts=START, kind="act.out", who="person:owner", place=PLACE,
                       body="telegram",
                       payload={"capability": "telegram.say",
                                "arguments": {"text": "pagi juga"}})
    await write_state(conn, "person:tg:1", {"name": "Budi"}, first.id)
    await write_state(conn, "task:space:task:1", {"status": "open"}, first.id)


async def test_the_view_reads_recent_and_kind_rows(aconn):
    await _seed(aconn)
    view = contract.LogView(aconn)

    recent = await view.recent(PLACE, limit=5, kinds=["percept"])
    assert [event.payload["description"] for event in recent] == ["halo pagi", "kabar kopi"]
    assert [event.kind for event in await view.recent(PLACE, kinds=["act.out"])] == ["act.out"]
    assert len(await view.recent(limit=10)) == 3
    newest = recent[-1].id
    assert [event.kind for event in await view.by_kind("percept", after_id=newest)] == []
    assert len(await view.by_kind("percept")) == 2


async def test_the_view_reads_state_and_searches_text(aconn):
    await _seed(aconn)
    view = contract.LogView(aconn)

    assert await view.state("person:tg:1") == {"name": "Budi"}
    assert await view.state("absent") is None
    assert list(await view.state_prefix("task:")) == ["task:space:task:1"]
    assert [event.payload["description"] for event in await view.search("kopi")] == [
        "kabar kopi"]


def test_the_view_has_no_write_method_and_never_hands_out_the_connection():
    view = contract.LogView()

    for name in ("append", "write", "append_event", "conn", "connection", "execute"):
        assert not hasattr(view, name)


async def test_an_unbound_view_refuses_instead_of_reading_somewhere_else():
    view = contract.LogView()

    try:
        await view.recent()
    except RuntimeError as exc:
        assert "not bound" in str(exc)
    else:  # pragma: no cover - the refusal is the point
        raise AssertionError("an unbound view must refuse")


def test_a_plugin_api_carries_the_view_and_no_connection():
    api = PluginAPI(name="demo", manifest={}, options={}, features={},
                    log=contract.LogView())

    assert isinstance(api.log, contract.LogView)
    assert not hasattr(api, "conn")
    assert not hasattr(api, "connection")

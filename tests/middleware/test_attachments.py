"""Attachments: content-addressed files, the one open door, hard forgetting and splitting."""

from __future__ import annotations

from datetime import datetime, timezone

import pytest

from monoagens.boot import build
from monoagens.infra import attachments
from monoagens.infra.attachments import MAX_BYTES, AttachmentStore
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event
from monoagens.core.memory.forget import forget_hard
from monoagens.middleware import contract, interaction
from monoagens.middleware.contract import describe_attachments
from tests.support import settings

START = datetime(2026, 1, 1, tzinfo=timezone.utc)
OWNER_DM = "space:tg:1001"
GROUP = "space:tg:-500"
OTHER = "space:wa:other"


async def _carrier(conn, place: str, kind: str, attachment: dict, *, who="person:owner"):
    return await append_event(conn, ts=START, kind="percept", who=who, place=place,
                              body="telegram",
                              payload={"description": "ini filenya",
                                       "place_kind": kind,
                                       "data": {"attachments": [attachment]}})


async def test_the_same_bytes_always_get_one_ref_and_one_copy(tmp_path):
    store = AttachmentStore(tmp_path)

    first = await store.put(b"isi laporan", name="report.txt", mime="text/plain")
    second = await store.put(b"isi laporan", name="report.txt", mime="text/plain")

    assert first == second
    assert first["ref"].startswith("att_") and len(first["ref"]) == 28
    assert first["size"] == len(b"isi laporan")
    assert store.path(first["ref"], first["name"]).read_bytes() == b"isi laporan"
    assert len(list(tmp_path.glob("*/*/*"))) == 1


async def test_a_file_over_the_limit_is_refused(tmp_path):
    store = AttachmentStore(tmp_path)

    with pytest.raises(ValueError, match="file too large"):
        await store.put(b"x" * (MAX_BYTES + 1), name="big.bin")


def test_a_name_is_made_safe():
    assert attachments.safe_filename("../../etc/passwd") == "passwd"
    assert attachments.safe_filename("laporan satu.txt") == "laporan satu.txt"
    assert attachments.safe_filename("") == "attachment"
    assert len(attachments.safe_filename("a" * 300)) == 120


async def test_opening_allows_the_origin_place_and_the_owners_chat(aconn, tmp_path):
    store = AttachmentStore(tmp_path)
    attachment = await store.put(b"kode", name="halo.py")
    await _carrier(aconn, OWNER_DM, "direct", attachment)
    await _carrier(aconn, "space:tg:9", "direct", attachment, who="person:owner")

    data, info = await contract.open_attachment(aconn, store, attachment["ref"],
                                                context=OWNER_DM)
    assert data == b"kode" and info["name"] == "halo.py"
    await _carrier(aconn, OTHER, "direct", {"ref": "att_nothing"}, who="person:other")

    with pytest.raises(PermissionError, match="may not see"):
        await contract.open_attachment(aconn, store, attachment["ref"], context=OTHER)


async def test_a_share_lets_another_place_open_the_file(aconn, tmp_path):
    store = AttachmentStore(tmp_path)
    attachment = await store.put(b"kode", name="halo.py")
    source = await _carrier(aconn, OWNER_DM, "direct", attachment)
    group_ev = await _carrier(aconn, GROUP, "shared", {"ref": "att_nothing"}, who="person:budi")
    await aconn.execute(
        "INSERT INTO state (key, value, updated_by_event) VALUES (%s, %s, %s)",
        (f"place:{GROUP}", '{"members": ["person:budi"]}', group_ev.id))

    with pytest.raises(PermissionError):
        await contract.open_attachment(aconn, store, attachment["ref"], context=GROUP)

    await append_event(aconn, ts=START, kind="share.granted", place=OWNER_DM,
                       payload={"persons": ["person:budi"], "source_place": OWNER_DM,
                                "episode": source.id, "to": "person:budi"},
                       caused_by=source.id)

    data, _ = await contract.open_attachment(aconn, store, attachment["ref"], context=GROUP)
    assert data == b"kode"


async def test_the_same_file_sent_elsewhere_stays_one_stored_file(aconn, tmp_path):
    store = AttachmentStore(tmp_path)
    attachment = await store.put(b"kode", name="halo.py")
    await _carrier(aconn, OWNER_DM, "direct", attachment)
    await _carrier(aconn, OTHER, "direct", attachment, who="person:other")

    origins = await contract.attachment_origins(aconn, attachment["ref"])

    assert [place for _, place in origins] == [OWNER_DM, OTHER]
    assert len(list(tmp_path.glob("*/*/*"))) == 1


async def test_a_forgotten_origin_makes_the_file_unopenable_and_the_sweep_deletes_it(
        aconn, tmp_path, monkeypatch):
    store = AttachmentStore(tmp_path)
    attachment = await store.put(b"laporan", name="laporan.txt")
    carrier = await _carrier(aconn, OWNER_DM, "direct", attachment)
    await forget_hard(aconn, [carrier.id], now=START)

    assert await contract.attachment_origins(aconn, attachment["ref"]) == []
    with pytest.raises((PermissionError, FileNotFoundError)):
        await contract.open_attachment(aconn, store, attachment["ref"], context=OWNER_DM)
    assert await store.sweep(aconn) == 1
    assert store.find(attachment["ref"]) is None


async def test_the_sweep_keeps_a_file_that_still_has_an_origin(aconn, tmp_path):
    store = AttachmentStore(tmp_path)
    attachment = await store.put(b"laporan", name="laporan.txt")
    await _carrier(aconn, OWNER_DM, "direct", attachment)

    assert await store.sweep(aconn) == 0
    assert store.find(attachment["ref"]) is not None


def test_describe_attachments_reads_as_one_sentence():
    files = [{"name": "report.pdf", "size": 10}, {"name": "photo.jpg", "size": 20},
             {"name": "data.xlsx", "size": 30}]

    assert describe_attachments(files, sent_by="Budi") == (
        "Budi sent 3 files: report.pdf, photo.jpg, data.xlsx")
    assert describe_attachments([{"name": "a.txt"}], sent_by="Budi") == "Budi sent 1 file: a.txt"
    assert describe_attachments([{"size": 17}], sent_by="Budi") == "Budi sent 1 file: 17 bytes"
    assert describe_attachments(
        [{"ref": "att_ab12cd34ef56", "name": "a.txt"}], sent_by="Budi") == (
        "Budi sent 1 file: a.txt (att_ab12cd34ef56)")


async def test_the_door_stores_and_opens_with_the_context(aconn, tmp_path):
    store = AttachmentStore(tmp_path)
    door = contract.AttachmentDoor(store, aconn)
    attachment = await door.put(b"isi", name="a.txt")
    await _carrier(aconn, OWNER_DM, "direct", attachment)

    data, info = await door.open(attachment["ref"], context=OWNER_DM)

    assert data == b"isi" and info["ref"] == attachment["ref"]


class _Limited:
    """A stand-in adapter with a file limit, for the splitting rule."""

    name = "cli"
    FILES = {"per_message": 2}

    async def act(self, action) -> None:
        """Record nothing: the split rule only needs the limit."""
        return None


async def test_splitting_three_files_for_an_adapter_that_takes_two():
    parts = interaction.split_attachments(_Limited(), {"text": "ini filenya",
                                                       "attachments": ["a", "b", "c"]})

    assert [part["attachments"] for part in parts] == [["a", "b"], ["c"]]
    assert parts[0]["text"] == "ini filenya"
    assert parts[1]["text"] == "(part 2/2)"


class _One:
    """A stand-in adapter that takes one file per message."""

    name = "cli"
    FILES = {"per_message": 1}

    async def act(self, action) -> None:
        """Record nothing: the split rule only needs the limit."""
        return None


async def test_splitting_three_files_one_per_message(aconn, tmp_path):
    kernel = build(aconn, clock=FakeClock(START), settings=settings())
    kernel.register_adapter(_One())
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:owner",
                                 place=OWNER_DM, body="cli",
                                 payload={"description": "kirim", "place_kind": "direct"})
    ctx = {"conn": aconn, "event": trigger, "now": START, "capability": "cli.say",
           "descriptor": {"communicative": True}, "origin_actor": "person:owner",
           "arguments": {"text": "ini", "attachments": ["a", "b", "c"]}, "to": None,
           "via": None, "place": OWNER_DM, "refusal": None, "handled": False, "labels": {},
           "long_running": False, "identity_changing": False}

    outcome = await interaction.split(kernel)({"text": "ini", "attachments": ["a", "b", "c"]},
                                              ctx)

    assert ctx["handled"] is True
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'act.out' ORDER BY id")
    rows = [row[0] for row in await cur.fetchall()]
    assert [row["arguments"]["text"] for row in rows] == ["ini", "(part 2/3)", "(part 3/3)"]
    split_events = [row for row in rows if row.get("parts")]
    assert outcome is not None and not split_events
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'interaction.split'")
    assert (await cur.fetchone())[0] == {"capability": "cli.say", "parts": 3, "attachments": 3}

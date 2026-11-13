"""WhatsApp history sync and offline backfill ingest (D66, D69)."""

from __future__ import annotations

from collections.abc import Iterable
from datetime import datetime
from typing import Any

from monoagens.adapters.whatsapp.format import album_parent_id, format_album, format_message
from monoagens.adapters.whatsapp.identity import WhatsAppDirectory, jid_text, place_id
from monoagens.adapters.whatsapp.media import store_deferred
from monoagens.types import Event

HISTORY_DAYS = 730
HISTORY_SIZE_MB = 5000
STORAGE_QUOTA_MB = 10240


def _varint(value: int) -> bytes:
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        out.append(byte | (0x80 if value else 0))
        if not value:
            return bytes(out)


def _read_varint(data: bytes, index: int) -> tuple[int, int]:
    value = shift = 0
    while True:
        byte = data[index]
        index += 1
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value, index
        shift += 7


def _replace_message_field(data: bytes, field: int, payload: bytes) -> bytes:
    """Replace a length-delimited field while preserving all unknown protobuf fields."""
    out = bytearray()
    index = 0
    found = False
    while index < len(data):
        start = index
        key, index = _read_varint(data, index)
        number, wire = key >> 3, key & 7
        if wire == 0:
            _, index = _read_varint(data, index)
        elif wire == 1:
            index += 8
        elif wire == 2:
            length, index = _read_varint(data, index)
            index += length
        elif wire == 5:
            index += 4
        else:
            raise ValueError(f"unknown wire type: {wire}")
        if number == field:
            found = True
            out += _varint((field << 3) | 2) + _varint(len(payload)) + payload
        else:
            out += data[start:index]
    if not found:
        out += _varint((field << 3) | 2) + _varint(len(payload)) + payload
    return bytes(out)


def history_props(require_full: bool = True) -> Any:
    """Return DeviceProps asking WhatsApp for history sync including group history (field 15)."""
    from neonize.proto.waCompanionReg.WAWebProtobufsCompanionReg_pb2 import DeviceProps

    config = DeviceProps.HistorySyncConfig(
        fullSyncDaysLimit=HISTORY_DAYS,
        fullSyncSizeMbLimit=HISTORY_SIZE_MB,
        storageQuotaMb=STORAGE_QUOTA_MB,
        inlineInitialPayloadInE2EeMsg=True,
        recentSyncDaysLimit=HISTORY_DAYS,
        supportCallLogHistory=False,
        supportBotUserAgentChatHistory=True,
        supportCagReactionsAndPolls=True,
        supportBizHostedMsg=True,
        supportRecentSyncChunkMessageCountTuning=True,
        supportHostedGroupMsg=True,
        supportFbidBotChatHistory=True,
        supportAddOnHistorySyncMigration=True,
        supportMessageAssociation=True,
    )
    props = DeviceProps(
        os="MonoAgens",
        platformType=DeviceProps.SAFARI,
        requireFullSync=require_full,
        historySyncConfig=config,
    )
    raw_config = config.SerializeToString() + b"\x78\x01"
    return DeviceProps.FromString(_replace_message_field(props.SerializeToString(), 5, raw_config))


async def sent_ids_from_log(conn: Any, body: str = "whatsapp") -> set[str]:
    """Load message ids the soul previously sent from the Log so D66 holds across restarts."""
    if conn is None:
        return set()
    cur = await conn.execute(
        "SELECT payload->'result'->>'message_id', payload->'result'->'message_ids' "
        "FROM events WHERE kind = 'act.result' AND payload->>'capability' LIKE %s",
        (f"{body}.%",),
    )
    ids: set[str] = set()
    for single, multi in await cur.fetchall():
        if single:
            ids.add(str(single))
        if isinstance(multi, list):
            ids.update(str(item) for item in multi if item)
    return ids


def ingest_contacts(sync: Any, directory: WhatsAppDirectory) -> None:
    """Populate LID->PN mappings and display names carried inside a HistorySync payload."""
    for mapping in getattr(sync, "phoneNumberToLidMappings", ()) or ():
        directory.learn_lid(getattr(mapping, "lidJID", ""), getattr(mapping, "pnJID", ""))
    for push in getattr(sync, "pushnames", ()) or ():
        directory.set_name(getattr(push, "ID", ""), getattr(push, "pushname", ""))
    for contact in getattr(sync, "inlineContacts", ()) or ():
        name = str(getattr(contact, "fullName", "") or getattr(contact, "firstName", "") or "")
        pn = getattr(contact, "pnJID", "")
        lid = getattr(contact, "lidJID", "")
        if lid and pn:
            directory.learn_lid(lid, pn)
        if name:
            if pn:
                directory.set_name(pn, name)
            if lid:
                directory.set_name(lid, name)


async def extract_sync_events(sync: Any, *, directory: WhatsAppDirectory, files: Any,
                              now: datetime, name: str = "whatsapp",
                              self_account: str | None = None,
                              own_sent: Iterable[str] = (),
                              backfill: bool = False) -> list[Event]:
    """Convert a `HistorySync` payload into chronological percept Events with deferred media."""
    if sync is None:
        return []
    ingest_contacts(sync, directory)
    sent_set = set(own_sent)
    events: list[Event] = []
    for conv in getattr(sync, "conversations", ()) or ():
        chat_raw = str(getattr(conv, "ID", "") or "")
        if not chat_raw:
            continue
        pn_jid = str(getattr(conv, "pnJID", "") or "")
        lid_jid = str(getattr(conv, "lidJID", "") or "")
        if lid_jid and pn_jid:
            directory.learn_lid(lid_jid, pn_jid)
        chat_jid = directory.canonical_chat(chat_raw, alt=pn_jid)
        place = place_id(chat_jid)
        parent_group = str(getattr(conv, "parentGroupID", "") or "")
        chat_name = str(getattr(conv, "name", "") or getattr(conv, "displayName", "") or "").strip()
        read_only = bool(getattr(conv, "readOnly", False) or getattr(conv, "locked", False))
        form = ("broadcast" if chat_jid.endswith("@newsletter")
                else "group" if chat_jid.endswith("@g.us") else "dm")
        directory.update_place(
            place,
            form=form,
            parent=place_id(parent_group) if parent_group else None,
            can_post=not read_only,
            name=chat_name,
        )
        fallback_ts = int(getattr(conv, "conversationTimestamp", 0) or 0)
        albums: dict[str, list[Event]] = {}
        for item in getattr(conv, "messages", ()) or ():
            web = getattr(item, "message", item)
            msg = getattr(web, "message", None)
            if msg is None:
                continue
            key = getattr(web, "key", None)
            msg_id = str(getattr(key, "ID", "") or "")
            atts, spec = await store_deferred(files, body=name, chat_jid=chat_jid,
                                              message_id=msg_id, message=msg)
            ev = format_message(
                web, directory=directory, now=now, name=name, self_account=self_account,
                own_sent=sent_set, attachments=atts, media_spec=spec,
                imported=not backfill, backfill=backfill,
                fallback_chat=chat_jid, fallback_ts=fallback_ts,
            )
            if ev is None:
                continue
            parent_id = album_parent_id(msg)
            if parent_id:
                albums.setdefault(parent_id, []).append(ev)
            else:
                events.append(ev)
        for album_id, grouped in albums.items():
            merged = format_album(grouped, album_id=album_id, directory=directory)
            if merged is not None:
                events.append(merged)
    return events

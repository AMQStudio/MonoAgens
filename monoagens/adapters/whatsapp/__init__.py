"""WhatsApp adapter (neonize): shared or own number, full history import, deferred media."""

from __future__ import annotations

import asyncio
import contextlib
import logging
import pathlib
from collections.abc import Awaitable, Callable
from typing import Any

from monoagens.adapters.whatsapp import send
from monoagens.adapters.whatsapp.capabilities import (
    CAPABILITIES,
    GAPS,
    OPTIONAL_CAPABILITIES,
    READS,
    SHOWS,
)
from monoagens.adapters.whatsapp.format import (
    album_expected_count,
    album_parent_id,
    clean_event,
    clean_text,
    format_album,
    format_call,
    format_message,
)
from monoagens.adapters.whatsapp.history_sync import (
    extract_sync_events,
    history_props,
    sent_ids_from_log,
)
from monoagens.adapters.whatsapp.identity import (
    WhatsAppDirectory,
    digits_of,
    import_contacts_sqlite,
    jid_text,
    place_id,
)
from monoagens.adapters.whatsapp.media import fetch_media, store_deferred
from monoagens.infra.clock import Clock
from monoagens.types import Action, Event

Put = Callable[[Event], Awaitable[None]]
LOGGER = logging.getLogger(__name__)
REQUIRES: tuple[str, ...] = ()
FILES = {"per_message": 20, "max_bytes": 100_000_000, "mimes": ()}
DEFAULT_ALBUM_WINDOW = 0.8


class _PutFailed(RuntimeError):
    """Wrap an error already handled/recorded by kernel `put()` so adapter won't duplicate it."""

    def __init__(self, cause: BaseException) -> None:
        super().__init__(str(cause))
        self.cause = cause


def create(options: dict[str, Any], env: dict[str, str]) -> "WhatsAppBody":
    """Build the WhatsApp adapter from `bodies.whatsapp` in config.yaml and `.env`."""
    session = str(options.get("session_db") or env.get("WHATSAPP_SESSION_DB")
                  or "var/whatsapp.db")
    full_sync = bool(options.get("full_sync", True))
    tools = [str(t) for t in (options.get("tools") or ()) if t]
    unknown = [t for t in tools if t not in OPTIONAL_CAPABILITIES]
    if unknown:
        raise ValueError(f"whatsapp: unknown optional tool {unknown[0]!r}")
    return WhatsAppBody(session_db=session, full_sync=full_sync, enabled_tools=tools)


class WhatsAppBody:
    """Eyes and hands for WhatsApp over neonize: shared account, history import, deferred media."""

    name = "whatsapp"
    reads = READS
    shows = SHOWS
    gaps = GAPS

    def __init__(self, session_db: str | pathlib.Path = "var/whatsapp.db", *,
                 full_sync: bool = True, clock: Clock | None = None,
                 client: Any = None, client_factory: Callable[..., Any] | None = None,
                 self_account: str | None = None,
                 album_window: float = DEFAULT_ALBUM_WINDOW,
                 enabled_tools: list[str] | tuple[str, ...] = ()) -> None:
        """Hold session configuration, directory state, and optional test client hooks."""
        self.session_db = pathlib.Path(session_db)
        self.full_sync = full_sync
        self._clock = clock or Clock()
        self._client = client
        self._client_factory = client_factory
        self._files: Any = None
        self.directory = WhatsAppDirectory()
        self.me: Any = None
        self.self_account: str | None = self_account
        self.album_window = album_window
        self.enabled_tools = tuple(t for t in enabled_tools if t in OPTIONAL_CAPABILITIES)
        self._sent: set[str] = set()
        self._sent_loaded = False
        self._choices_prompts: dict[str, list[str]] = {}
        self._albums: dict[str, dict[str, Any]] = {}
        self._connected_once = False
        self._backfilling = False
        self._loop: asyncio.AbstractEventLoop | None = None

    @property
    def files(self) -> Any:
        """Return the bound AttachmentDoor."""
        return self._files

    @files.setter
    def files(self, door: Any) -> None:
        """Bind `AttachmentDoor` and register our deferred-media fetcher on its store."""
        self._files = door
        if door is not None and hasattr(door, "register_fetcher"):
            door.register_fetcher(self.name, self.fetch_attachment)

    def capabilities(self) -> tuple[dict[str, Any], ...]:
        """Return declared capabilities plus any tools enabled in `bodies.whatsapp.tools`."""
        extras = tuple(OPTIONAL_CAPABILITIES[name] for name in self.enabled_tools)
        return CAPABILITIES + extras

    def remember_sent(self, message_id: str) -> None:
        """Record an outgoing message id so D66 ignores its own `IsFromMe` echo."""
        if message_id:
            self._sent.add(str(message_id))

    async def is_own_message(self, message_id: str) -> bool:
        """Return whether `message_id` was sent by the soul (in memory or recorded in Log)."""
        if not message_id:
            return False
        if message_id in self._sent:
            return True
        if not self._sent_loaded:
            conn = getattr(self._files, "_conn", None)
            if conn is not None:
                found = await sent_ids_from_log(conn, body=self.name)
                self._sent.update(found)
                self._sent_loaded = True
        return message_id in self._sent

    def _sync_self_identity(self, me: Any) -> None:
        if me is None:
            return
        self.me = me
        pn_jid = jid_text(getattr(me, "JID", None))
        lid_jid = jid_text(getattr(me, "LID", None))
        if pn_jid and lid_jid:
            self.directory.learn_lid(lid_jid, pn_jid)
        acc = self.directory.canonical_account(pn_jid or lid_jid)
        if acc:
            self.self_account = acc

    async def fetch_attachment(self, ref: str, meta: dict[str, Any]) -> bytes:
        """Materialize one deferred attachment when first opened."""
        return await fetch_media(self._client, ref, meta)

    async def run(self, put: Put) -> None:
        """Connect to WhatsApp, stream percepts into `put`, and flush media albums."""
        self._loop = asyncio.get_running_loop()
        if self._files is not None and hasattr(self._files, "register_fetcher"):
            self._files.register_fetcher(self.name, self.fetch_attachment)
        if not self._sent_loaded:
            conn = getattr(self._files, "_conn", None)
            if conn is not None:
                self._sent.update(await sent_ids_from_log(conn, body=self.name))
            self._sent_loaded = True
        if self._client is None:
            self.session_db.parent.mkdir(parents=True, exist_ok=True)
            if callable(self._client_factory):
                self._client = self._client_factory(str(self.session_db),
                                                    props=history_props(self.full_sync))
            else:
                from neonize.aioze.client import NewAClient

                self._client = NewAClient(str(self.session_db),
                                          props=history_props(self.full_sync))
        client = self._client
        safe_put = self._wrap_put(put)
        self._register_handlers(client, safe_put)
        try:
            await client.connect()
            idle = getattr(client, "idle", None)
            if callable(idle):
                await idle()
        finally:
            await self.aclose()

    def _wrap_put(self, put: Put) -> Put:
        """Marshal puts onto the adapter's event loop and tag errors already handled by put()."""
        async def safe_put(ev: Event) -> Any:
            target_loop = self._loop
            try:
                running = asyncio.get_running_loop()
            except RuntimeError:
                running = None
            try:
                if target_loop is not None and not target_loop.is_closed() and running is not target_loop:
                    return await asyncio.wrap_future(
                        asyncio.run_coroutine_threadsafe(put(ev), target_loop)
                    )
                return await put(ev)
            except Exception as exc:
                raise _PutFailed(exc) from exc

        return safe_put

    async def _emit_adapter_error(self, put: Put, exc: BaseException, *, label: str) -> None:
        """Record one adapter-layer failure as an `error` event and log it."""
        detail = clean_text(f"{type(exc).__name__}: {exc}" if str(exc) else type(exc).__name__)
        LOGGER.exception("whatsapp %s failed: %s", label, detail)
        if isinstance(exc, _PutFailed):
            return
        err_ev = Event(
            id=None,
            ts=self._clock.now(),
            kind="error",
            who=self.name,
            place="",
            body=self.name,
            payload={"adapter": self.name, "handler": label, "error": f"{self.name} {label}: {detail}"},
        )
        try:
            await put(err_ev)
        except Exception as emit_exc:
            LOGGER.error("whatsapp failed to emit error event for %s: %s", label, emit_exc)

    def _register_handlers(self, client: Any, put: Put) -> None:
        from neonize.aioze.events import (
            CallOfferEv,
            CallOfferNoticeEv,
            CallTerminateEv,
            ConnectedEv,
            GroupInfoEv,
            HistorySyncEv,
            JoinedGroupEv,
            MessageEv,
            NewsletterJoinEv,
            NewsletterLiveUpdateEv,
            OfflineSyncCompletedEv,
            OfflineSyncPreviewEv,
        )

        async def _guarded(label: str, coro: Awaitable[Any]) -> None:
            try:
                await coro
            except Exception as exc:
                await self._emit_adapter_error(put, exc, label=label)

        @client.event(ConnectedEv)
        async def _on_connected(accepted: Any, _ev: Any) -> None:
            async def work() -> None:
                self._sync_self_identity(
                    getattr(accepted, "me", None) or getattr(client, "me", None))
                await asyncio.to_thread(import_contacts_sqlite, self.session_db, self.directory)
                await self._sync_places(accepted)
                self._connected_once = True

            await _guarded("connected", work())

        @client.event(OfflineSyncPreviewEv)
        async def _on_offline_preview(_accepted: Any, _ev: Any) -> None:
            self._backfilling = True

        @client.event(OfflineSyncCompletedEv)
        async def _on_offline_done(_accepted: Any, _ev: Any) -> None:
            self._backfilling = False

        @client.event(MessageEv)
        async def _on_message(accepted: Any, ev: Any) -> None:
            async def work() -> None:
                self._sync_self_identity(
                    getattr(accepted, "me", None) or getattr(client, "me", None))
                await self.handle_message(ev, put)

            await _guarded("message", work())

        @client.event(HistorySyncEv)
        async def _on_history(_accepted: Any, ev: Any) -> None:
            if getattr(ev, "Data", None) is not None:
                await _guarded("history_sync", self.handle_history_sync(ev.Data, put))

        @client.event(GroupInfoEv)
        async def _on_group_info(_accepted: Any, ev: Any) -> None:
            await _guarded("group_info", self.handle_group_info(ev, put))

        @client.event(JoinedGroupEv)
        async def _on_joined_group(_accepted: Any, ev: Any) -> None:
            async def work() -> None:
                info = getattr(ev, "GroupInfo", None)
                if info is not None:
                    self.directory.record_group(info, self_digits=digits_of(self.self_account or ""))

            await _guarded("joined_group", work())

        @client.event(NewsletterJoinEv)
        async def _on_news_join(_accepted: Any, ev: Any) -> None:
            async def work() -> None:
                self.directory.record_newsletter(ev)

            await _guarded("newsletter_join", work())

        @client.event(NewsletterLiveUpdateEv)
        async def _on_news_update(_accepted: Any, ev: Any) -> None:
            async def work() -> None:
                self.directory.record_newsletter(ev)

            await _guarded("newsletter_update", work())

        @client.event(CallOfferEv)
        async def _on_call_offer(_accepted: Any, ev: Any) -> None:
            await _guarded("call_offer", self.handle_call("incoming", ev.basicCallMeta, put))

        @client.event(CallOfferNoticeEv)
        async def _on_call_notice(_accepted: Any, ev: Any) -> None:
            is_video = str(getattr(ev, "media", "") or "").lower() == "video"
            await _guarded("call_notice",
                           self.handle_call("incoming", ev.basicCallMeta, put, is_video=is_video))

        @client.event(CallTerminateEv)
        async def _on_call_end(_accepted: Any, ev: Any) -> None:
            reason = str(getattr(ev, "reason", "") or "")
            status = "missed" if "miss" in reason.lower() or "timeout" in reason.lower() else "ended"
            await _guarded("call_end",
                           self.handle_call(status, ev.basicCallMeta, put, reason=reason))

    async def _sync_places(self, client: Any) -> None:
        own_digits = digits_of(self.self_account or "")
        getter = getattr(client, "get_joined_groups", None)
        if callable(getter):
            try:
                for info in await getter() or ():
                    self.directory.record_group(info, self_digits=own_digits)
            except Exception as exc:
                LOGGER.warning("whatsapp _sync_places failed: %s", exc)

    async def handle_message(self, ev: Any, put: Put, *, backfill: bool = False) -> Event | None:
        """Convert one live `MessageEv` into a percept (collapsing albums) and emit via `put`."""
        info = getattr(ev, "Info", None)
        msg_id = str(getattr(info, "ID", "") or "")
        source = getattr(info, "MessageSource", None)
        from_me = bool(getattr(source, "IsFromMe", False))
        if from_me and msg_id and await self.is_own_message(msg_id):
            return None
        chat_raw = jid_text(getattr(source, "Chat", None))
        alt = getattr(source, "SenderAlt", None)
        chat_jid = self.directory.canonical_chat(chat_raw, alt=alt)
        message = getattr(ev, "Message", None)
        if message is None:
            return None
        expected = album_expected_count(message)
        if expected > 0 and msg_id:
            entry = self._albums.setdefault(msg_id, {"expected": expected, "events": [],
                                                    "task": None, "put": put})
            entry["expected"] = expected
            return None
        atts, spec = await store_deferred(self._files, body=self.name, chat_jid=chat_jid,
                                          message_id=msg_id, message=message)
        event = format_message(
            ev, directory=self.directory, now=self._clock.now(), name=self.name,
            self_account=self.self_account, own_sent=self._sent,
            attachments=atts, media_spec=spec,
            choices_prompts=self._choices_prompts,
            backfill=backfill or self._backfilling,
        )
        if event is None:
            return None
        parent_id = album_parent_id(message)
        if parent_id:
            await self._collect_album(parent_id, event, put)
            return None
        await put(event)
        return event

    async def _collect_album(self, album_id: str, event: Event, put: Put) -> None:
        entry = self._albums.setdefault(album_id, {"expected": 0, "events": [],
                                                   "task": None, "put": put})
        entry["events"].append(event)
        expected = int(entry.get("expected") or 0)
        if expected and len(entry["events"]) >= expected:
            task = entry.get("task")
            if task is not None and not task.done():
                task.cancel()
            await self.flush_album(album_id)
            return
        if entry.get("task") is None or entry["task"].done():
            async def _timer() -> None:
                await asyncio.sleep(self.album_window)
                try:
                    await self.flush_album(album_id)
                except Exception as exc:
                    await self._emit_adapter_error(put, exc, label="album_flush")

            entry["task"] = asyncio.create_task(_timer())

    async def flush_album(self, album_id: str) -> Event | None:
        """Emit one combined percept for all collected items in `album_id`."""
        entry = self._albums.pop(album_id, None)
        if not entry or not entry.get("events"):
            return None
        task = entry.get("task")
        if task is not None and not task.done() and task is not asyncio.current_task():
            task.cancel()
        merged = format_album(entry["events"], album_id=album_id, directory=self.directory)
        if merged is not None:
            await entry["put"](merged)
        return merged

    async def handle_history_sync(self, sync: Any, put: Put, *,
                                  backfill: bool = False) -> list[Event]:
        """Ingest one `HistorySync` payload into percepts and put each without waking."""
        if not self._sent_loaded:
            conn = getattr(self._files, "_conn", None)
            if conn is not None:
                self._sent.update(await sent_ids_from_log(conn, body=self.name))
            self._sent_loaded = True
        events = await extract_sync_events(
            sync, directory=self.directory, files=self._files, now=self._clock.now(),
            name=self.name, self_account=self.self_account, own_sent=self._sent,
            backfill=backfill,
        )
        for ev in events:
            try:
                await put(ev)
            except Exception as exc:
                await self._emit_adapter_error(put, exc, label="history_sync_put")
        return events

    async def handle_group_info(self, ev: Any, put: Put) -> Event | None:
        """Update cached place metadata and emit a percept describing the group change."""
        jid = jid_text(getattr(ev, "JID", None))
        if not jid:
            return None
        place = place_id(jid)
        changes: list[str] = []
        new_name = str(getattr(getattr(ev, "Name", None), "Name", "") or "").strip()
        if new_name:
            self.directory.update_place(place, name=new_name)
            changes.append(f"renamed group to {new_name}")
        topic = str(getattr(getattr(ev, "Topic", None), "Topic", "") or "").strip()
        if topic:
            changes.append(f"updated topic: {topic}")
        announce = getattr(ev, "Announce", None)
        if announce is not None and getattr(announce, "IsAnnounce", None) is not None:
            self.directory.update_place(place, can_post=not bool(announce.IsAnnounce))
            changes.append("restricted posting to admins" if announce.IsAnnounce else "opened posting")
        joined = [self.directory.canonical_account(j) for j in getattr(ev, "Join", ()) or ()]
        left = [self.directory.canonical_account(j) for j in getattr(ev, "Leave", ()) or ()]
        if joined:
            changes.append(f"joined: {', '.join(joined)}")
        if left:
            changes.append(f"left: {', '.join(left)}")
        if not changes:
            return None
        sender = self.directory.canonical_account(getattr(ev, "Sender", None)) or f"whatsapp:{jid}"
        ev_out = clean_event(Event(
            id=None, ts=self._clock.now(), kind="percept", who=sender, place=place, body=self.name,
            payload={"description": "; ".join(changes), "addressed": False, "place_kind": "shared",
                     "place_info": self.directory.place_meta(place),
                     **({"self_account": self.self_account} if self.self_account else {}),
                     "data": {"group_event": changes, "joined": joined, "left": left}},
        ))
        await put(ev_out)
        return ev_out

    async def handle_call(self, status: str, meta: Any, put: Put, *, reason: str = "",
                          is_video: bool = False) -> Event | None:
        """Emit an incoming, missed, or ended call percept."""
        ev = format_call(status, meta, reason=reason, is_video=is_video,
                         directory=self.directory, now=self._clock.now(), name=self.name,
                         self_account=self.self_account)
        if ev is not None:
            await put(ev)
        return ev

    async def act(self, action: Action) -> Any:
        """Perform `say`, `react`, or `edit` in a WhatsApp chat and record sent message ids."""
        if action.kind == "say":
            res = await send.send_say(self._client, self.directory, self._files,
                                      action.place, dict(action.payload or {}))
        elif action.kind == "react":
            res = await send.send_react(
                self._client, action.place, str(action.payload.get("emoji") or ""),
                str(action.payload.get("message_id") or ""))
        elif action.kind == "edit":
            res = await send.send_edit(
                self._client, action.place, str(action.payload.get("message_id") or ""),
                str(action.payload.get("text") or ""))
        elif action.kind == "pin" and "pin" in self.enabled_tools:
            getter = getattr(self._client, "pin_message", None)
            if callable(getter):
                await getter(send.jid_of(send.chat_jid_from_place(action.place)),
                             str(action.payload.get("message_id") or ""))
            res = {"pinned": True}
        elif action.kind == "reject_call" and "reject_call" in self.enabled_tools:
            rejector = getattr(self._client, "reject_call", None)
            if callable(rejector):
                await rejector(str(action.payload.get("caller") or ""),
                               str(action.payload.get("call_id") or ""))
            res = {"rejected": True}
        else:
            raise ValueError(f"Unsupported action kind: {action.kind!r}")
        if isinstance(res, dict):
            if res.get("message_id"):
                self.remember_sent(str(res["message_id"]))
            for mid in res.get("message_ids") or ():
                self.remember_sent(str(mid))
            cp = res.pop("choices_prompt", None)
            if isinstance(cp, dict) and cp.get("message_id"):
                self._choices_prompts[str(cp["message_id"])] = list(cp.get("options") or ())
        return res

    async def show(self, place: str, kind: str, **data: Any) -> Any:
        """Show typing, message edit, or reaction feedback on WhatsApp."""
        return await send.send_show(self._client, place, kind, **data)

    async def place_info(self, place: str) -> dict[str, Any]:
        """Return structured place info `{form, parent, can_post, name, members, roles}`."""
        jid_str = place.removeprefix("space:wa:")
        if self._client is not None:
            own_digits = digits_of(self.self_account or "")
            if jid_str.endswith("@g.us"):
                getter = getattr(self._client, "get_group_info", None)
                if callable(getter):
                    with contextlib.suppress(Exception):
                        info = await getter(send.jid_of(jid_str))
                        if info is not None:
                            return self.directory.record_group(info, self_digits=own_digits)
            elif jid_str.endswith("@newsletter"):
                getter = getattr(self._client, "get_newsletter_info", None)
                if callable(getter):
                    with contextlib.suppress(Exception):
                        meta = await getter(send.jid_of(jid_str))
                        if meta is not None:
                            return self.directory.record_newsletter(meta)
        return self.directory.place_meta(place)

    async def aclose(self) -> None:
        """Cancel pending album timers and stop the neonize client cleanly."""
        for album_id in list(self._albums):
            task = self._albums[album_id].get("task")
            if task is not None and not task.done():
                task.cancel()
        self._albums.clear()
        if self._client is not None:
            stop = getattr(self._client, "stop", None)
            if callable(stop):
                with contextlib.suppress(Exception):
                    res = stop()
                    if hasattr(res, "__await__"):
                        await res

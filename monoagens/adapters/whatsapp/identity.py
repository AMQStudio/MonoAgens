"""WhatsApp identity and place directory: one canonical form per account (LID + PN) and place map."""

from __future__ import annotations

import pathlib
import sqlite3
from typing import Any

WA_PREFIX = "whatsapp:"
PLACE_PREFIX = "space:wa:"


def digits_of(value: Any) -> str:
    """Return digits from the user head of a JID/account; fall back to lowercased text."""
    text = str(value or "").strip()
    if text.startswith(WA_PREFIX):
        text = text.removeprefix(WA_PREFIX)
    head = text.split("@")[0].split(":")[0]
    digits = "".join(ch for ch in head if ch.isdigit())
    return digits or text.lower()


def jid_text(jid: Any) -> str:
    """Format a protobuf JID or raw string into `<user>@<server>` without device suffix."""
    if jid is None:
        return ""
    if isinstance(jid, str):
        raw = jid.strip()
        if "@" not in raw:
            return raw
        user, _, server = raw.partition("@")
        return f"{user.split(':')[0]}@{server.lower()}"
    user = str(getattr(jid, "User", "") or "").split(":")[0]
    server = str(getattr(jid, "Server", "") or "").lower()
    if not user:
        return ""
    return f"{user}@{server}" if server else user


def place_id(chat_jid: str) -> str:
    """Return the neutral place id (`space:wa:<jid>`) for one WhatsApp chat."""
    bare = jid_text(chat_jid)
    return bare if bare.startswith(PLACE_PREFIX) else f"{PLACE_PREFIX}{bare}"


def chat_jid_from_place(place: str) -> str:
    """Extract `<user>@<server>` from a `space:wa:<jid>` place id."""
    return str(place or "").removeprefix(PLACE_PREFIX)


class WhatsAppDirectory:
    """Maps LIDs to phone numbers, tracks contact names, and keeps place metadata."""

    def __init__(self) -> None:
        """Start empty; populated live from stanzas, history sync, and session DB."""
        self.lid_to_pn: dict[str, str] = {}
        self.pn_to_lid: dict[str, str] = {}
        self.names: dict[str, str] = {}
        self.places: dict[str, dict[str, Any]] = {}
        self.jids: dict[str, str] = {}

    def learn_lid(self, lid_jid: Any, pn_jid: Any) -> None:
        """Record that `lid_jid` and `pn_jid` belong to the same WhatsApp account."""
        lid = digits_of(jid_text(lid_jid) if not isinstance(lid_jid, str) else lid_jid)
        pn_str = jid_text(pn_jid) if not isinstance(pn_jid, str) else str(pn_jid or "")
        if pn_str.endswith("@lid"):
            return
        pn = digits_of(pn_str)
        if not lid or not pn or lid == pn:
            return
        self.lid_to_pn[lid] = pn
        self.pn_to_lid[pn] = lid
        if "@" in pn_str:
            self.jids[pn] = pn_str
        elif lid in self.names and pn not in self.names:
            self.names[pn] = self.names[lid]

    def set_name(self, account_or_jid: Any, name: str) -> None:
        """Remember one display name under its canonical account digits."""
        clean = str(name or "").strip()
        if not clean:
            return
        digits = self.canonical_digits(jid_text(account_or_jid))
        if digits:
            self.names[digits] = clean

    def canonical_digits(self, jid_or_account: Any, *, alt: Any = None) -> str:
        """Return canonical digits: phone number whenever known, otherwise raw digits."""
        raw = jid_text(jid_or_account)
        alt_text = jid_text(alt) if alt is not None else ""
        if raw.endswith("@lid") and alt_text and not alt_text.endswith("@lid"):
            self.learn_lid(raw, alt_text)
        elif alt_text.endswith("@lid") and raw and not raw.endswith("@lid"):
            self.learn_lid(alt_text, raw)
        key = digits_of(raw)
        if key in self.lid_to_pn:
            pn = self.lid_to_pn[key]
            self.jids.setdefault(pn, f"{pn}@s.whatsapp.net")
            return pn
        if raw.endswith("@s.whatsapp.net") and key:
            self.jids[key] = raw
        elif alt_text.endswith("@s.whatsapp.net"):
            pn = digits_of(alt_text)
            if pn:
                self.jids[pn] = alt_text
                return pn
        return key

    def canonical_account(self, jid_or_account: Any, *, alt: Any = None) -> str:
        """Return `whatsapp:<digits>` collapsed across LID and phone number."""
        digits = self.canonical_digits(jid_or_account, alt=alt)
        return f"{WA_PREFIX}{digits}" if digits else ""

    def canonical_chat(self, chat_jid: Any, *, alt: Any = None) -> str:
        """Normalize a DM `@lid` chat JID to `<pn>@s.whatsapp.net` when the PN is known."""
        raw = jid_text(chat_jid)
        if not raw.endswith("@lid"):
            return raw
        if alt is not None:
            alt_text = jid_text(alt)
            if alt_text and not alt_text.endswith("@lid"):
                self.learn_lid(raw, alt_text)
        digits = self.canonical_digits(raw)
        if digits and digits != digits_of(raw):
            return self.jids.get(digits) or f"{digits}@s.whatsapp.net"
        return raw

    def jid_for_account(self, account: str) -> str:
        """Return the JID string for sending a DM or mention to `whatsapp:<digits>`."""
        digits = self.canonical_digits(account)
        return self.jids.get(digits) or f"{digits}@s.whatsapp.net"

    def label(self, account_or_jid: Any, *, pushname: str = "") -> str:
        """Return the best human label available for a sender."""
        digits = self.canonical_digits(account_or_jid)
        if pushname.strip() and digits not in self.names:
            self.names[digits] = pushname.strip()
        return self.names.get(digits) or pushname.strip() or digits or "someone"

    def place_meta(self, place: str) -> dict[str, Any]:
        """Return `{form, parent, can_post, name, members, roles}` for one place."""
        if place in self.places:
            return dict(self.places[place])
        jid = chat_jid_from_place(place)
        if jid.endswith("@newsletter"):
            return {"form": "broadcast", "parent": None, "can_post": False,
                    "name": "", "members": [], "roles": []}
        if jid.endswith("@g.us"):
            return {"form": "group", "parent": None, "can_post": True,
                    "name": "", "members": [], "roles": []}
        return {"form": "dm", "parent": None, "can_post": True,
                "name": self.names.get(digits_of(jid), ""), "members": [], "roles": []}

    def update_place(self, place: str, **updates: Any) -> dict[str, Any]:
        """Merge known place fields and return the updated dict."""
        current = self.place_meta(place)
        for key in ("form", "parent", "can_post", "name", "members", "roles"):
            if key in updates and updates[key] is not None:
                current[key] = updates[key]
        self.places[place] = current
        return dict(current)

    def record_group(self, info: Any, *, self_digits: str = "") -> dict[str, Any]:
        """Extract Community `parent`, announce lock, members, and admins from GroupInfo."""
        jid = jid_text(getattr(info, "JID", None))
        place = place_id(jid)
        name = str(getattr(getattr(info, "GroupName", None), "Name", "") or "").strip()
        linked = jid_text(getattr(getattr(info, "GroupLinkedParent", None), "LinkedParentJID", None))
        parent = place_id(linked) if linked else None
        is_announce = bool(getattr(getattr(info, "GroupAnnounce", None), "IsAnnounce", False))
        members: list[str] = []
        roles: list[str] = []
        self_admin = False
        for participant in getattr(info, "Participants", ()) or ():
            pn = jid_text(getattr(participant, "PhoneNumber", None))
            p_jid = jid_text(getattr(participant, "JID", None))
            p_lid = jid_text(getattr(participant, "LID", None))
            if p_lid and pn:
                self.learn_lid(p_lid, pn)
            elif p_jid.endswith("@lid") and pn:
                self.learn_lid(p_jid, pn)
            account = self.canonical_account(pn or p_jid or p_lid)
            if account:
                members.append(account)
            if getattr(participant, "IsAdmin", False) or getattr(participant, "IsSuperAdmin", False):
                if account:
                    roles.append(f"{account}:admin")
                if self_digits and self.canonical_digits(account) == self_digits:
                    self_admin = True
        can_post = not (is_announce and not self_admin)
        return self.update_place(place, form="group", parent=parent, can_post=can_post,
                                 name=name, members=members, roles=roles)

    def record_newsletter(self, meta: Any) -> dict[str, Any]:
        """Extract `broadcast` form and `can_post` permission from NewsletterMetadata."""
        jid = jid_text(getattr(meta, "ID", None))
        place = place_id(jid)
        thread_meta = getattr(meta, "ThreadMeta", None)
        name_obj = getattr(thread_meta, "Name", None)
        name = str(getattr(name_obj, "Text", "") or getattr(thread_meta, "Name", "") or "").strip()
        viewer = getattr(meta, "ViewerMeta", None)
        role_raw = getattr(viewer, "Role", 0)
        role_str = str(role_raw).upper()
        # NewsletterRole enum: SUBSCRIBER=1, GUEST=2, ADMIN=3, OWNER=4
        can_post = role_str in ("3", "4", "OWNER", "ADMIN") if viewer is not None else False
        roles = [f"viewer:{role_str.lower()}"] if viewer is not None else []
        return self.update_place(place, form="broadcast", parent=None, can_post=can_post,
                                 name=name, roles=roles)


def import_contacts_sqlite(session_db: str | pathlib.Path, directory: WhatsAppDirectory) -> int:
    """Read LID mappings and contact names stored in neonize's sqlite database, if present."""
    path = pathlib.Path(session_db)
    if not path.is_file():
        return 0
    count = 0
    try:
        conn = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    except sqlite3.Error:
        return 0
    try:
        tables = {row[0] for row in conn.execute(
            "SELECT name FROM sqlite_master WHERE type='table'")}
        if "whatsmeow_lid_map" in tables:
            for lid, pn in conn.execute("SELECT lid, pn FROM whatsmeow_lid_map"):
                directory.learn_lid(f"{lid}@lid", f"{pn}@s.whatsapp.net")
                count += 1
        if "whatsmeow_contacts" in tables:
            for jid, full_name, push_name, business_name in conn.execute(
                "SELECT their_jid, COALESCE(full_name, ''), COALESCE(push_name, ''), "
                "COALESCE(business_name, '') FROM whatsmeow_contacts"
            ):
                label = full_name.strip() or business_name.strip() or push_name.strip()
                if label:
                    directory.set_name(jid, label)
                    count += 1
    except sqlite3.Error:
        return count
    finally:
        conn.close()
    return count

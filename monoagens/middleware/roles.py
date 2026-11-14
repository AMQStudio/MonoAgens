"""Roles: what each person may do — tiers, permissions, role settings and approval modes.

Always present, so no module or plugin needs a branch for "no roles" (middleware.md §8).
The default book is empty: the owner may do everything, everyone else only talks. A
permission without a `base` is the owner's alone; plugins declare their permissions with a
label and the lowest tier that gets them, and `config.d/<plugin>.yaml` may move a tier.
"""

from __future__ import annotations

import pathlib
from collections.abc import Sequence
from dataclasses import dataclass, field, replace
from typing import Any, Mapping

from monoagens.core.hooks import Continue, Middleware
from monoagens.infra.store import append_event, read_state, write_state
from monoagens.middleware import identity, privacy
from monoagens.middleware.registry import CommandEntry
from monoagens.types import Event

TIERS = ("guest", "member", "trusted", "admin")
OWNER_TIER = "owner"  # above admin: a permission without a base needs it
APPROVALS = ("yolo", "default", "strict")
DEFAULT_APPROVAL = "default"

# The middleware's own catalog: declared in code, movable by the owner like any permission.
# A permission without a base (None) is the owner's alone and cannot be given away.
CORE_PERMISSIONS: dict[str, str | None] = {
    "help": "guest",
    "identity.whoami": "guest",
    "identity.link": "guest",
    "identity.unlink": "guest",
    "identity.set": None,
    "self.recall": "guest",
    "privacy.share": "guest",
    "registry.describe": "guest",
    "registry.more": "guest",
    "config.reload": "admin",
    "plugin.manage": "admin",
    "roles.manage": "admin",
    "identity.whois": "admin",
    "approval.answer": "admin",
    "budget.manage": None,
    "quiet.manage": None,
    "interaction.show": None,
}


def _rank(tier: str | None) -> int:
    """Return a tier's position; the owner is above every tier."""
    if tier == OWNER_TIER:
        return len(TIERS)
    return TIERS.index(tier) if tier in TIERS else -1


@dataclass(frozen=True, slots=True)
class Role:
    """One role: a tier role (it has a `base`) or a group role (it only marks a group)."""

    name: str
    base: str | None = None
    allow: frozenset[str] = frozenset()
    deny: frozenset[str] = frozenset()
    settings: dict[str, Any] = field(default_factory=dict)
    approval: str | None = None
    label: str | None = None
    context: str | None = None
    platform_role: tuple[str, ...] = ()

    def base_tier(self) -> str:
        """Return this role's declared base tier (`guest` if omitted); `allow` never raises it."""
        return self.base if self.base in TIERS else "guest"

    def effective_tier(self, bases: Mapping[str, str | None]) -> str:
        """Return the highest tier this role reaches: its base or the base of what it allows."""
        reached = [_rank(self.base)]
        for permission in self.allow:
            owner_only = permission in bases and bases[permission] is None
            reached.append(_rank(OWNER_TIER if owner_only else bases.get(permission)))
        best = max(reached)
        return OWNER_TIER if best >= len(TIERS) else TIERS[max(best, 0)]


@dataclass(frozen=True, slots=True)
class Person:
    """One person as `config.yaml` records them: a label, the roles held and a context line."""

    id: str
    label: str | None = None
    roles: tuple[str, ...] = ()
    context: str | None = None


@dataclass(frozen=True, slots=True)
class RoleBook:
    """The resolved roles: who holds what, what each tier gets, and what it may do."""

    roles: Mapping[str, Role] = field(default_factory=dict)
    people: Mapping[str, Person] = field(default_factory=dict)
    fallback: str = ""
    bases: Mapping[str, str | None] = field(default_factory=dict)
    places: Mapping[str, Mapping[str, frozenset[str]]] = field(default_factory=dict)
    role_settings: Mapping[str, Mapping[str, Any]] = field(default_factory=dict)
    sources: Mapping[str, str] = field(default_factory=dict)
    merges: Mapping[str, str] = field(default_factory=dict)
    place_parents: Mapping[str, str] = field(default_factory=dict)
    platform_roles: Mapping[str, Mapping[str, frozenset[str]]] = field(default_factory=dict)

    @classmethod
    def empty(cls) -> "RoleBook":
        """Return the default book: the owner may do everything, everyone else only talks."""
        return cls(bases=dict(CORE_PERMISSIONS))

    def with_bases(self, more: Mapping[str, str | None]) -> "RoleBook":
        """Return this book with the base tier of each named permission added.

        A body declares the base tier of its own capabilities; the book learns them when the
        body registers, so a kernel with no deployment still knows what talking costs.
        """
        return replace(self, bases={**dict(self.bases), **dict(more)})

    @classmethod
    def build(cls, config: Mapping[str, Any], catalog: Mapping[str, str | None],
              manifests: Mapping[str, Mapping[str, Any]],
              config_d: Mapping[str, Mapping[str, Any]], report: Any) -> "RoleBook":
        """Resolve config.yaml's roles and people against every declared permission."""
        bases, sources = _bases(catalog, manifests, config_d, report)
        settings_by_tier = _role_settings(manifests, config_d)
        definitions = dict(config.get("roles") or {})
        fallback = str(definitions.pop("fallback", "") or "")
        found: dict[str, Role] = {}
        for name, raw in definitions.items():
            values = dict(raw or {})
            found[str(name)] = Role(
                name=str(name), base=values.get("base"),
                allow=_expand(values.get("allow") or (), bases, report),
                deny=_expand(values.get("deny") or (), bases, report),
                settings=dict(values.get("settings") or {}),
                approval=values.get("approval"), label=values.get("label"),
                context=values.get("context"),
                platform_role=_parse_platform_roles(values.get("platform_role"), str(name), report))
        if fallback and fallback not in found:
            report.add(f"role {fallback} is not defined; ignored")
            fallback = ""
        people: dict[str, Person] = {}
        for who, raw in (config.get("people") or {}).items():
            values = dict(raw or {})
            if not str(who).startswith("person:"):
                report.add(f"people.{who}: unknown; ignored")
                continue
            held = []
            for role in values.get("roles") or ():
                if str(role) in found:
                    held.append(str(role))
                else:
                    report.add(f"role {role} is not defined; ignored")
            people[str(who)] = Person(str(who), values.get("label"), tuple(held),
                                      values.get("context"))
        places = {str(place): {"allow": frozenset((values or {}).get("allow") or ()),
                               "deny": frozenset((values or {}).get("deny") or ())}
                  for place, values in (config.get("places") or {}).items()}
        return cls(roles=found, people=people, fallback=fallback, bases=bases, places=places,
                   role_settings=settings_by_tier, sources=sources)

    def with_person(self, who: str, held: Sequence[str]) -> "RoleBook":
        """Return the book with one person's roles replaced (the running book, no restart)."""
        person = self.people.get(who) or Person(who)
        return replace(self, people={**self.people, who: replace(person, roles=tuple(held))})

    def with_merges(self, merges: Mapping[str, str], report: Any = None) -> "RoleBook":
        """Attach `merged:` links so younger entries keep applying and boot warns once."""
        combined = {**self.merges, **{str(src): str(dst) for src, dst in merges.items() if dst}}
        if report is not None:
            for younger, surviving in sorted(combined.items()):
                if younger in self.people and younger != surviving:
                    report.add(
                        f"roles: people.{younger} was merged into {surviving}; "
                        f"keeping its entry until moved to people.{surviving}")
        return replace(self, merges=combined)

    def _aliases_for(self, who: str) -> tuple[str, ...]:
        """Return this person id first, followed by any younger merged ids."""
        target = self.merges.get(str(who), str(who))
        younger = sorted(src for src, dst in self.merges.items() if dst == target and src != target)
        return (target, *younger) if target == str(who) else (str(who), target, *younger)

    def with_place_parents(self, parents: Mapping[str, str]) -> "RoleBook":
        """Return the book updated with opaque `place -> parent` links for ancestor walks."""
        if not parents:
            return self
        return replace(self, place_parents={**self.place_parents,
                                            **{str(k): str(v) for k, v in parents.items() if v}})

    def with_platform_roles(self, who: str, space: str, role_ids: Sequence[str]) -> "RoleBook":
        """Replace `who`'s active platform role ids inside organisation `space`."""
        spc, person = str(space or ""), str(who or "")
        if not spc or not person:
            return self
        by_space = dict(self.platform_roles.get(person) or {})
        by_space[spc] = frozenset(str(rid) for rid in role_ids if str(rid))
        return replace(self, platform_roles={**self.platform_roles, person: by_space})

    def with_platform_context(self, who: str, space: str, role_ids: Sequence[str],
                              parents: Mapping[str, str] | None = None) -> "RoleBook":
        """Return the book updated with one percept's parent chain and held platform role ids."""
        book = self.with_place_parents(parents or {})
        return book.with_platform_roles(who, space, role_ids) if space else book

    def _place_chain(self, place: str | None) -> tuple[str, ...]:
        """Walk `place`'s opaque parent chain from innermost to outermost."""
        chain: list[str] = []
        seen: set[str] = set()
        cur = str(place or "")
        while cur and cur not in seen:
            seen.add(cur)
            chain.append(cur)
            cur = str(self.place_parents.get(cur) or "")
        return tuple(chain)

    def _matching_platform_roles(self, who: str, place: str | None) -> tuple[str, ...]:
        """Return mapped role names earned via platform roles in `place`'s parent chain."""
        ancestors = self._place_chain(place)
        if not ancestors:
            return ()
        matched: list[str] = []
        for space in ancestors:
            held_ids: set[str] = set()
            for alias in self._aliases_for(str(who)):
                by_space = self.platform_roles.get(alias)
                if by_space and space in by_space:
                    held_ids.update(by_space[space])
            if not held_ids:
                continue
            prefix = f"{space}/"
            for role in self.roles.values():
                if role.name not in matched and any(
                    entry.startswith(prefix) and entry.removeprefix(prefix) in held_ids
                    for entry in role.platform_role
                ):
                    matched.append(role.name)
        return tuple(matched)

    def held(self, who: str, place: str | None = None) -> tuple[Role, ...]:
        """Return the roles one person holds here, in configuration order."""
        names: list[str] = []
        for alias in self._aliases_for(str(who)):
            person = self.people.get(alias)
            for name in (person.roles if person is not None else ()):
                if name not in names:
                    names.append(name)
        for name in self._matching_platform_roles(who, place):
            if name not in names:
                names.append(name)
        if self.fallback and self.fallback not in names:
            names.append(self.fallback)
        return tuple(self.roles[name] for name in names if name in self.roles)

    def tier(self, who: str, place: str | None = None) -> str:
        """Return one person's tier: the owner is above all, else the highest role base held."""
        if str(who) == "person:owner":
            return OWNER_TIER
        reached = [_rank(role.base_tier()) for role in self.held(who, place) if role.base in TIERS]
        best = max(reached) if reached else 0
        return TIERS[max(best, 0)]

    def permissions(self, who: str, place: str | None = None) -> frozenset[str]:
        """Return every permission one person holds: the tier's, plus allows, minus denies."""
        if str(who) == "person:owner":
            return frozenset(self.bases)
        tier = _rank(self.tier(who, place))
        held = {name for name, base in self.bases.items()
                if base is not None and _rank(base) <= tier}
        for role in self.held(who, place):
            held |= set(role.allow)
            held -= set(role.deny)
        return frozenset(held)

    def can(self, who: str, permission: str, place: str | None = None) -> bool:
        """Return whether one person may use one permission here (a place may override).

        The owner is always free, including for a permission this book does not know yet
        (a plugin tool's own name): nothing may stop the owner from using their own tools.
        """
        if not permission:
            return True  # a talking capability of an adapter belongs to everyone
        if str(who) == "person:owner":
            return True
        override = self.places.get(str(place)) if place else None
        if override:
            if permission in override["deny"]:
                return False
            if permission in override["allow"]:
                return True
        return permission in self.permissions(who, place)

    def settings(self, who: str, place: str | None = None) -> dict[str, Any]:
        """Return one person's role settings: the tier's, then each role's (lists merge)."""
        tier = self.tier(who, place)
        resolved: dict[str, Any] = {}
        for name in TIERS:
            if _rank(name) <= _rank(tier):
                resolved.update(dict(self.role_settings.get(name) or {}))
        for role in self.held(who, place):
            for key, value in role.settings.items():
                if key.endswith("shared"):
                    merged = list(resolved.get(key) or [])
                    for item in value if isinstance(value, list) else [value]:
                        if item not in merged:
                            merged.append(item)
                    resolved[key] = merged
                else:
                    resolved[key] = value
        return resolved

    def approval(self, who: str, place: str | None = None) -> str:
        """Return one person's approval mode (`yolo`, `default` or `strict`)."""
        for role in reversed(self.held(who, place)):
            if role.approval in APPROVALS:
                return str(role.approval)
        return DEFAULT_APPROVAL

    def label(self, who: str) -> str | None:
        """Return one person's label: theirs first, merged entry next, then a role's."""
        for alias in self._aliases_for(str(who)):
            person = self.people.get(alias)
            if person is not None and person.label:
                return person.label
        for role in self.held(who):
            if role.label:
                return role.label
        return None

    def contexts(self, who: str, place: str | None = None) -> tuple[str, ...]:
        """Return the context lines for one person: each role's, then their own."""
        lines = [role.context for role in self.held(who, place) if role.context]
        for alias in self._aliases_for(str(who)):
            person = self.people.get(alias)
            if person is not None and person.context:
                lines.append(person.context)
        return tuple(lines)

    def may_give(self, giver: str, role_name: str, place: str | None = None) -> bool:
        """Return whether one person may give a role: strictly below their own tier."""
        role = self.roles.get(role_name)
        if role is None:
            return False
        mine, theirs = _rank(self.tier(giver, place)), _rank(role.effective_tier(self.bases))
        return self.tier(giver, place) == OWNER_TIER or theirs < mine


# -------------------------------------------------------------- resolution


def _parse_platform_roles(raw: Any, role_name: str, report: Any) -> tuple[str, ...]:
    """Parse `platform_role:` entries (`<space>/<role id>`) and warn on malformed specs."""
    if raw is None:
        return ()
    items = [raw] if isinstance(raw, str) else list(raw)
    parsed: list[str] = []
    for entry in items:
        text = str(entry or "").strip()
        space, sep, role_id = text.rpartition("/")
        if not sep or not space.strip() or not role_id.strip():
            if report is not None:
                report.add(f"role {role_name}: invalid platform_role {text!r} "
                           "(expected '<space>/<role id>'); ignored")
            continue
        norm = f"{space.strip()}/{role_id.strip()}"
        if norm not in parsed:
            parsed.append(norm)
    return tuple(parsed)


def validate_permission_name(owner: str, permission: str, source: str) -> None:
    """Refuse redefining a core permission or one outside `owner`'s namespace."""
    perm = str(permission)
    if perm in CORE_PERMISSIONS or not perm.startswith(f"{owner}."):
        raise ValueError(
            f"{source}: permission {perm!r} redefines core or is outside namespace '{owner}.'")


def _bases(catalog: Mapping[str, str | None], manifests: Mapping[str, Mapping[str, Any]],
           config_d: Mapping[str, Mapping[str, Any]],
           report: Any) -> tuple[dict[str, str | None], dict[str, str]]:
    """Return the effective base of every permission and where it came from."""
    bases = {str(name): base for name, base in catalog.items()}
    sources = {str(name): "middleware" for name in catalog}
    for owner, manifest in manifests.items():
        for name, spec in (manifest.get("permissions") or {}).items():
            validate_permission_name(str(owner), str(name), f"plugin {owner}")
            bases[str(name)] = (spec or {}).get("base")
            sources[str(name)] = "manifest"
    for owner, values in config_d.items():
        for name, base in (values.get("permissions") or {}).items():
            validate_permission_name(str(owner), str(name), f"config.d/{owner}.yaml")
            if name in bases and bases[name] != base:
                report.add(f"permission {name}: {base or OWNER_TIER} (config.d)")
            bases[str(name)] = base
            sources[str(name)] = "config.d"
    for pattern in [name for name in bases if _is_pattern(name)]:
        matched = _wildcard(pattern, bases)
        report.add(f"roles: {pattern} expands to {', '.join(sorted(matched))}")
        for name in matched:
            bases[name] = bases[pattern]
            sources[name] = sources[pattern]
        bases.pop(pattern, None)
        sources.pop(pattern, None)
    return bases, sources


def _role_settings(manifests: Mapping[str, Mapping[str, Any]],
                   config_d: Mapping[str, Mapping[str, Any]]) -> dict[str, dict[str, Any]]:
    """Return every role setting by tier: the manifests first, then config.d."""
    by_tier: dict[str, dict[str, Any]] = {tier: {} for tier in TIERS}
    sources = [manifest.get("role_settings") or {} for manifest in manifests.values()]
    sources += [values.get("role_settings") or {} for values in config_d.values()]
    for source in sources:
        for key, per_tier in source.items():
            for tier, value in (per_tier or {}).items():
                if tier in TIERS:
                    by_tier[tier][str(key)] = value
    return by_tier


def _is_pattern(name: str) -> bool:
    """Return whether one permission name is a wildcard (`*` or `plugin.*`)."""
    return name == "*" or name.endswith(".*")


def _wildcard(pattern: str, bases: Mapping[str, str | None]) -> set[str]:
    """Return every permission a wildcard stands for: declared, not core, never owner-only."""
    candidates = [name for name, base in bases.items()
                  if name not in CORE_PERMISSIONS and base is not None
                  and not _is_pattern(name)]
    if pattern == "*":
        return set(candidates)
    prefix = pattern.removesuffix("*")
    return {name for name in candidates if name.startswith(prefix)}


def _expand(names: Any, bases: Mapping[str, str | None], report: Any) -> frozenset[str]:
    """Expand the wildcards of one role's allow or deny into the permission names."""
    found: set[str] = set()
    for name in names:
        if _is_pattern(str(name)):
            matched = _wildcard(str(name), bases)
            report.add(f"roles: {name} expands to {', '.join(sorted(matched))}")
            found |= matched
        else:
            found.add(str(name))
    return frozenset(found)


def describe_role(book: RoleBook, role_name: str) -> str:
    """Render one role's merged permissions and settings, each line with its source."""
    role = book.roles.get(role_name)
    if role is None:
        return f"no role called {role_name}"
    tier = role.base_tier()
    held = {name for name, base in book.bases.items()
            if base is not None and _rank(base) <= _rank(tier)}
    held = (held | set(role.allow)) - set(role.deny)
    lines = [f"{name}: {book.bases.get(name) or OWNER_TIER} "
             f"({book.sources.get(name, 'config.yaml')})" for name in sorted(held)]
    for key, value in (("label", role.label), ("approval", role.approval),
                       ("context", role.context),
                       ("platform_role",
                        ", ".join(role.platform_role) if role.platform_role else None)):
        if value:
            lines.append(f"{key}: {value} (config.yaml)")
    for key, value in sorted((book.role_settings.get(tier) or {}).items()):
        lines.append(f"{key}: {value} ({book.sources.get(key, 'manifest')})")
    for key, value in sorted(role.settings.items()):
        lines.append(f"{key}: {value} (config.yaml)")
    return "\n".join(lines) or f"{role_name} holds nothing"


# ---------------------------------------------------------------- commands


async def reconcile_platform_roles(conn: Any, book: RoleBook) -> RoleBook:
    """Load persisted `state: place:*` parent chains and `state: platform_roles:*` into `book`."""
    cur_places = await conn.execute("SELECT key, value FROM state WHERE key LIKE 'place:%'")
    parents: dict[str, str] = {}
    for key, value in await cur_places.fetchall():
        parent = str((value or {}).get("parent") or "")
        if parent:
            parents[str(key).removeprefix("place:")] = parent
    book = book.with_place_parents(parents)
    cur_roles = await conn.execute(
        "SELECT value FROM state WHERE key LIKE 'platform_roles:%'")
    for (value,) in await cur_roles.fetchall():
        doc = value or {}
        who = str(doc.get("who") or "")
        space = str(doc.get("space") or "")
        if who and space:
            role_ids = [str(r) for r in (doc.get("roles") or ())]
            book = book.with_platform_roles(who, space, role_ids)
    return book


async def reconcile_merges(conn: Any, book: RoleBook, report: Any = None) -> RoleBook:
    """Load `state: merged:*`, place parent chains and platform role holdings into `book`."""
    from monoagens.core.memory.cards import merge_map  # local: avoid top-level cycle

    merges = await merge_map(conn)
    return await reconcile_platform_roles(conn, book.with_merges(merges, report=report))


def platform_roles(kernel: Any) -> Middleware:
    """Inbound rule: keep `kernel.roles` synced with each percept's platform roles and parents."""

    async def rule(item: Any, ctx: Any) -> Any:
        del ctx
        if getattr(item, "kind", "") != "percept":
            return Continue()
        payload = item.payload or {}
        if payload.get("imported") is True:
            return Continue()
        place = str(getattr(item, "place", "") or "")
        place_info = payload.get("place_info") or {}
        data = payload.get("data") or {}
        parents: dict[str, str] = {}
        raw_parent = str(place_info.get("parent") or "")
        if place and raw_parent:
            parents[place] = raw_parent
        extra_parents = place_info.get("parents")
        if isinstance(extra_parents, Mapping):
            for child, par in extra_parents.items():
                if child and par:
                    parents[str(child)] = str(par)
        book = _book(kernel)
        raw_pr = data.get("platform_roles")
        space = str(raw_pr.get("space") or "") if isinstance(raw_pr, Mapping) else ""
        if space:
            who = str(getattr(item, "origin_actor", None) or getattr(item, "who", "") or "")
            role_ids = [str(r) for r in (raw_pr.get("roles") or ())]
            if who and getattr(item, "id", None):
                key = f"platform_roles:{who}|{space}"
                cur = await read_state(kernel.conn, key)
                if list((cur or {}).get("roles") or ()) != role_ids:
                    await write_state(kernel.conn, key,
                                      {"who": who, "space": space, "roles": role_ids},
                                      int(item.id))
            book = book.with_platform_context(who, space, role_ids, parents)
        elif parents:
            book = book.with_place_parents(parents)
        kernel.roles = book
        return Continue()

    return rule


def install_commands(kernel: Any) -> None:
    """Register role commands and keep platform role mappings updated on inbound percepts."""
    tools = kernel.tools
    for name, description, permission, handler in (
            ("whoami", "Show who you are here, and what you may do.", "identity.whoami",
             whoami),
            ("whois", "Show one person's id and linked accounts.", "identity.whois", whois),
            ("role", "Give or take a role: /role add|remove <person> <role>.",
             "roles.manage", give_role),
            ("roles", "Show what one role gets: /roles show <role>.", "roles.manage", show_role)):
        if tools.command(name) is None:
            tools.add_command(CommandEntry(name, description, permission, handler,
                                           owner="roles"))
    inbound = getattr(getattr(kernel, "hooks", None), "inbound", None)
    if inbound is not None and "platform-roles" not in getattr(inbound, "_names", set()):
        inbound.add("platform-roles", platform_roles(kernel), slot="first")


def _held_line(book: RoleBook, who: str, place: str | None = None) -> str:
    """Return the roles line: the owner is named `owner`, then every role held.

    The fallback role is dropped for the owner: it describes someone not yet known, so it
    says nothing true about the owner, whose tier is above every role.
    """
    owner = book.tier(who, place) == OWNER_TIER
    named = ["owner"] if owner else []
    for role in book.held(who, place):
        if owner and role.name == book.fallback:
            continue
        named.append(role.name + (f" ({role.label})" if role.label else ""))
    return ", ".join(named) or "none"


def _book(kernel: Any) -> RoleBook:
    """Return the kernel's role book, or the default one before a loader installed it."""
    return getattr(kernel, "roles", None) or RoleBook.empty()


async def whoami(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Show the asker's id and roles; accounts only ever in a direct chat."""
    conn, book = kernel.conn, _book(kernel)
    who, place = str(event.origin_actor or event.who), str(event.place or "")
    shared = not await privacy.direct_for(conn, place, who)
    name = await identity.name_of(conn, who)
    person = book.people.get(who)
    label = (person.label if person is not None and person.label else None) \
        or name or book.label(who)
    lines = [f"you: {who}" + (f" ({label})" if label else "")]
    accounts = await identity.accounts_of(conn, who)
    if shared:
        target = await identity.direct_place(conn, who, event.body)
        if accounts and target:
            await tell(kernel, event, target, "/whoami: your linked accounts: "
                                              + ", ".join(accounts))
            lines.append("(accounts sent to your direct chat)")
    elif accounts:
        lines.append("accounts: " + ", ".join(accounts))
    lines.append(f"roles: {_held_line(book, who, place)}; tier {book.tier(who, place)}; "
                 f"approval {book.approval(who, place)}")
    return "\n".join(lines)


async def whois(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Show one person's id, label and linked accounts (`identity.whois`)."""
    query = " ".join(arguments).strip()
    if not query:
        return "usage: /whois <name or account>"
    who = await identity.find_person(kernel.conn, query, labels=_labels(kernel))
    if who is None:
        return f"no one called {query}"
    label = _labels(kernel).get(who) or await identity.name_of(kernel.conn, who)
    lines = [f"{who}" + (f" ({label})" if label else "")]
    accounts = await identity.accounts_of(kernel.conn, who)
    if accounts:
        lines.append("accounts: " + ", ".join(accounts))
    return "\n".join(lines)


async def give_role(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Give or take one role: only below the giver's own tier, one line in config.yaml."""
    if len(arguments) < 3 or arguments[0].lower() not in ("add", "remove"):
        return "usage: /role add|remove <person> <role>"
    action, query, role_name = arguments[0].lower(), arguments[1], arguments[2]
    book = _book(kernel)
    if role_name not in book.roles:
        return f"no role called {role_name}"
    giver = str(event.origin_actor or event.who)
    if not book.may_give(giver, role_name, event.place):
        return f"only the owner may change {role_name}"
    who = (await identity.find_person(kernel.conn, query, labels=_labels(kernel))
           or (query if query.startswith("person:") else None))
    if who is None:
        return f"no one called {query}"
    person = book.people.get(who)
    held = list(person.roles if person is not None else ())
    added = action == "add" and role_name not in held
    if action == "add" and added:
        held.append(role_name)
    elif action == "remove":
        held = [name for name in held if name != role_name]
    await edit_people(kernel, event, who, held)
    if added or action == "remove":
        await append_event(kernel.conn, ts=kernel.clock.now(), kind="roles.changed",
                           who=giver, place=event.place, body=event.body,
                           caused_by=event.id,
                           payload={"who": giver, "person": who, "role": role_name,
                                    "added": added})
    return f"{who} now holds {', '.join(held) or 'no role'}"


async def show_role(kernel: Any, event: Event, arguments: list[str]) -> str:
    """Print one role's merged permissions and settings with each line's source."""
    if len(arguments) < 2 or arguments[0].lower() != "show":
        return "usage: /roles show <role>"
    return describe_role(_book(kernel), arguments[1])


def _labels(kernel: Any) -> dict[str, str]:
    """Return the labels `config.yaml` gives people, for matching a query."""
    return {who: str(person.label) for who, person in _book(kernel).people.items()
            if person.label}


async def tell(kernel: Any, event: Event, place: str, text: str) -> None:
    """Send one extra message in the place it belongs, and record it like any act."""
    capability = f"{event.body}.say"
    stored = await kernel._emit(event, "act.out", {
        "capability": capability, "arguments": {"text": text}, "communicative": True},
        place=place)
    await kernel.tools.act(replace(event, place=place), capability, {"text": text},
                           event_id=stored.id)


async def edit_people(kernel: Any, event: Event, who: str, held: Sequence[str]) -> None:
    """Write only `people.<id>.roles` in config.yaml, after a backup, and update the book."""
    from monoagens.middleware import loader  # local: loader imports roles' siblings

    root = getattr(kernel, "deployment_root", None)
    if root is None:
        kernel.roles = _book(kernel).with_person(who, held)
        return
    path = pathlib.Path(root) / "config.yaml"
    loader.edit_config(path, lambda text: loader.set_person_roles(text, who, held),
                       now=kernel.clock.now())
    kernel.roles = _book(kernel).with_person(who, held)

"""Roles: what each person may do — tiers, permissions, role settings and approval modes.

Always present, so no module or plugin needs a branch for "no roles" (middleware.md §8).
The default book is empty: the owner may do everything, everyone else only talks. A
permission without a `base` is the owner's alone; plugins declare their permissions with a
label and the lowest tier that gets them, and `config.d/<plugin>.yaml` may move a tier.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Mapping

TIERS = ("guest", "member", "trusted", "admin")
OWNER_TIER = "owner"  # above admin: a permission without a base needs it
APPROVALS = ("yolo", "default", "strict")
DEFAULT_APPROVAL = "default"

# The middleware's own catalog: declared in code, movable by the owner like any permission.
# A permission without a base (None) is the owner's alone and cannot be given away.
CORE_PERMISSIONS: dict[str, str | None] = {
    "help": "guest",
    "identity.whoami": "guest",
    "identity.set": None,
    "self.recall": "guest",
    "privacy.share": "guest",
    "registry.describe": "guest",
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

    def effective_tier(self, bases: Mapping[str, str | None]) -> str:
        """Return the highest tier this role reaches: its base or the base of what it allows."""
        reached = [_rank(self.base)]
        for permission in self.allow:
            reached.append(_rank(bases.get(permission)))
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

    @classmethod
    def empty(cls) -> "RoleBook":
        """Return the default book: the owner may do everything, everyone else only talks."""
        return cls(bases=dict(CORE_PERMISSIONS))

    def held(self, who: str) -> tuple[Role, ...]:
        """Return the roles one person holds, in the order the configuration names them."""
        person = self.people.get(str(who))
        names = list(person.roles if person is not None else [])
        if self.fallback and self.fallback not in names:
            names.append(self.fallback)
        return tuple(self.roles[name] for name in names if name in self.roles)

    def tier(self, who: str) -> str:
        """Return one person's tier: the owner is above all, else the highest role held."""
        if str(who) == "person:owner":
            return OWNER_TIER
        reached = [role.effective_tier(self.bases) for role in self.held(who)]
        return max(reached, key=_rank) if reached else "guest"

    def permissions(self, who: str) -> frozenset[str]:
        """Return every permission one person holds: the tier's, plus allows, minus denies."""
        if str(who) == "person:owner":
            return frozenset(self.bases)
        tier = _rank(self.tier(who))
        held = {name for name, base in self.bases.items() if _rank(base) <= tier}
        for role in self.held(who):
            held |= set(role.allow)
            held -= set(role.deny)
        return frozenset(held)

    def can(self, who: str, permission: str, place: str | None = None) -> bool:
        """Return whether one person may use one permission here (place overrides later).

        The owner is always free, including for a permission this book does not know yet
        (a plugin tool's own name): nothing may stop the owner from using their own tools.
        """
        if not permission:
            return True  # a talking capability of an adapter belongs to everyone
        if str(who) == "person:owner":
            return True
        return permission in self.permissions(who)

    def settings(self, who: str) -> dict[str, Any]:
        """Return one person's role settings, later roles overriding earlier ones."""
        resolved: dict[str, Any] = {}
        for role in self.held(who):
            resolved.update(role.settings)
        return resolved

    def approval(self, who: str) -> str:
        """Return one person's approval mode (`yolo`, `default` or `strict`)."""
        for role in reversed(self.held(who)):
            if role.approval in APPROVALS:
                return str(role.approval)
        return DEFAULT_APPROVAL

    def label(self, who: str) -> str | None:
        """Return one person's label: theirs first, then a role's."""
        person = self.people.get(str(who))
        if person is not None and person.label:
            return person.label
        for role in self.held(who):
            if role.label:
                return role.label
        return None

    def contexts(self, who: str) -> tuple[str, ...]:
        """Return the context lines for one person: each role's, then their own."""
        lines = [role.context for role in self.held(who) if role.context]
        person = self.people.get(str(who))
        if person is not None and person.context:
            lines.append(person.context)
        return tuple(lines)

    def may_give(self, giver: str, role_name: str) -> bool:
        """Return whether one person may give a role: strictly below their own tier."""
        role = self.roles.get(role_name)
        if role is None:
            return False
        mine, theirs = _rank(self.tier(giver)), _rank(role.effective_tier(self.bases))
        return self.tier(giver) == OWNER_TIER or theirs < mine

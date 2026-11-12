"""Roles: four tiers, declared permissions, group roles, wildcards and the role commands."""

from __future__ import annotations

from datetime import datetime, timezone

from monoagens.boot import build
from monoagens.core.context import layout
from monoagens.infra.clock import FakeClock
from monoagens.infra.store import append_event, write_state
from monoagens.middleware import identity, roles
from monoagens.middleware.loader import BootReport
from monoagens.types import Action, Event
from tests.support import settings

START = datetime(2026, 1, 1, tzinfo=timezone.utc)
GROUP = "space:tg:-500"
DIRECT = "space:tg:1"
CATALOG = {
    **roles.CORE_PERMISSIONS,
    "workspace.read": "guest", "workspace.write": "member", "workspace.run": "trusted",
    "web-search.search": "member",
}
MANIFESTS = {"workspace": {"permissions": {"workspace.read": {"base": "guest"},
                                           "workspace.write": {"base": "member"},
                                           "workspace.run": {"base": "trusted"}},
                           "role_settings": {"workspace.quota": {"guest": "0", "member": "1G"},
                                             "workspace.shared": {"guest": [], "member": []}}}}
CONFIG = {
    "roles": {
        "fallback": "unverified",
        "unverified": {"base": "guest", "label": "Unverified",
                       "context": "Not known yet; be kind."},
        "member": {"base": "member"},
        "trusted": {"base": "trusted", "approval": "yolo"},
        "admin": {"base": "admin"},
        "toko-a": {"label": "Toko A", "settings": {"workspace.shared": "toko-a"}},
        "runner": {"allow": ["workspace.run"]},
        "power": {"allow": ["budget.manage"]},
        "quiet-member": {"base": "member", "allow": ["workspace.run"], "deny": ["workspace.write"]},
    },
    "people": {
        "person:tg:1": {"label": "Budi", "roles": ["member", "toko-a"],
                        "context": "The owner's brother."},
        "person:tg:2": {"roles": ["admin"]},
        "person:tg:3": {"label": "Fulan", "roles": ["quiet-member"]},
    },
    "places": {GROUP: {"deny": ["workspace.write"]}},
}


def book(config=None, *, config_d=None) -> roles.RoleBook:
    """Build a role book from this test's configuration."""
    return roles.RoleBook.build(config or CONFIG, CATALOG, MANIFESTS, config_d or {},
                                BootReport())


def test_the_owner_is_above_every_tier_and_a_person_gets_the_highest_role():
    found = book()

    assert found.tier("person:owner") == roles.OWNER_TIER
    assert found.tier("person:tg:1") == "member"
    assert found.tier("person:tg:2") == "admin"
    assert found.tier("person:tg:9") == "guest"  # the fallback role


def test_an_empty_book_lets_the_owner_do_everything_and_everyone_else_talk():
    found = roles.RoleBook.empty()

    assert found.can("person:owner", "workspace.run")
    assert found.can("person:owner", "anything.at.all")
    assert found.can("person:tg:1", "help")
    assert not found.can("person:tg:1", "workspace.run")
    assert found.can("person:tg:1", "")  # a talking capability


def test_a_tier_gives_every_permission_at_or_below_it():
    found = book()

    assert found.can("person:tg:1", "workspace.read")
    assert found.can("person:tg:1", "workspace.write")
    assert not found.can("person:tg:1", "workspace.run")
    assert found.can("person:owner", "budget.manage")


def test_a_deny_beats_an_allow_from_the_same_person():
    found = book()

    assert found.can("person:tg:3", "workspace.run")  # allowed on top of member
    assert not found.can("person:tg:3", "workspace.write")  # denied although member


def test_a_group_role_gives_only_what_it_lists():
    found = book({"roles": {"fallback": "guest", "guest": {"base": "guest"},
                            "toko-a": {"label": "Toko A", "settings": {"x": 1}}},
                  "people": {"person:tg:1": {"roles": ["toko-a"]}}})

    assert found.tier("person:tg:1") == "guest"
    assert not found.can("person:tg:1", "workspace.write")
    assert found.settings("person:tg:1")["x"] == 1


def test_a_new_role_with_only_a_base_works_for_every_plugin():
    found = book({"roles": {"fallback": "unverified", "unverified": {"base": "guest"},
                            "newcomer": {"base": "member"}},
                  "people": {"person:tg:5": {"roles": ["newcomer"]}}})

    assert found.can("person:tg:5", "workspace.write")
    assert found.can("person:tg:5", "web-search.search")
    assert not found.can("person:tg:5", "workspace.run")


def test_a_permission_moved_in_config_d_reaches_every_role_of_that_tier():
    report = BootReport()
    config_d = {"workspace": {"permissions": {"workspace.run": "member"}}}
    found = roles.RoleBook.build(CONFIG, CATALOG, MANIFESTS, config_d, report)

    assert found.can("person:tg:1", "workspace.run")
    assert "permission workspace.run: member (config.d)" in report.lines


def test_a_wildcard_is_expanded_and_named_in_the_report():
    report = BootReport()
    config_d = {"workspace": {"permissions": {"workspace.*": "member"}}}
    found = roles.RoleBook.build(CONFIG, CATALOG, MANIFESTS, config_d, report)

    assert found.can("person:tg:1", "workspace.run")
    assert any(line.startswith("roles: workspace.* expands to ") for line in report.lines)


def test_a_wildcard_never_takes_a_core_permission():
    report = BootReport()
    found = roles.RoleBook.build({"roles": {"boss": {"allow": ["*"]}}}, CATALOG,
                                 MANIFESTS, {}, report)

    assert found.can("person:tg:2", "workspace.run") is False
    assert "budget.manage" not in found.permissions("person:tg:2")


def test_a_place_override_denies_one_permission_there_only():
    found = book()

    assert found.can("person:tg:1", "workspace.write", DIRECT)
    assert not found.can("person:tg:1", "workspace.write", GROUP)


def test_an_approval_mode_comes_from_the_roles():
    found = book()

    assert found.approval("person:tg:2") == "default"
    assert found.approval("person:tg:9") == "default"
    assert book({"roles": {"fallback": "t", "t": {"base": "trusted", "approval": "yolo"}},
                 "people": {}}).approval("person:tg:8") == "yolo"


def test_role_settings_come_from_the_tier_then_the_role_and_lists_merge():
    found = book()

    assert found.settings("person:tg:1")["workspace.quota"] == "1G"
    assert found.settings("person:tg:1")["workspace.shared"] == ["toko-a"]


def test_an_unknown_role_name_and_an_unknown_person_are_reported_and_dropped():
    report = BootReport()
    found = roles.RoleBook.build({"roles": {"fallback": "ghost"},
                                  "people": {"budi": {"roles": ["member"]},
                                             "person:tg:1": {"roles": ["nope"]}}},
                                 CATALOG, MANIFESTS, {}, report)

    assert found.fallback == ""
    assert found.people["person:tg:1"].roles == ()
    assert sorted(report.lines) == ["people.budi: unknown; ignored",
                                    "role ghost is not defined; ignored",
                                   "role nope is not defined; ignored"]


def test_a_giver_gives_only_roles_below_their_own_tier():
    found = book()

    assert found.may_give("person:tg:2", "member")
    assert found.may_give("person:tg:2", "trusted")
    assert not found.may_give("person:tg:2", "admin")
    assert not found.may_give("person:tg:1", "trusted")
    assert found.may_give("person:owner", "admin")


def test_a_role_that_carries_an_owner_permission_counts_as_the_owners():
    found = book()

    assert found.roles["power"].effective_tier(found.bases) == roles.OWNER_TIER
    assert not found.may_give("person:tg:2", "power")
    assert found.can("person:owner", "budget.manage")
    assert not found.can("person:tg:2", "budget.manage")


async def _kernel(aconn):
    kernel = build(aconn, clock=FakeClock(START), settings=settings())
    kernel.register_adapter(_Body())
    kernel.roles = book()
    roles.install_commands(kernel)
    return kernel


class _Body:
    """A body that declares a talking capability and can name its places."""

    name = "telegram"

    def capabilities(self):
        return ({"name": "say", "description": "Speak here.",
                 "schema": {"type": "object", "properties": {"text": {"type": "string"}},
                            "required": ["text"]},
                 "communicative": True, "long_running": False, "identity_changing": False,
                 "facts": (), "permission": None},)

    async def act(self, action: Action) -> None:
        return None

    async def place_info(self, place: str) -> dict:
        return {"name": "Tim Alpha", "member_count": 2, "role_facts": []}


def _command(place: str = DIRECT, who: str = "person:tg:1") -> Event:
    return Event(id=None, ts=START, kind="command", who=who, place=place, body="telegram",
                 payload={"command": "whoami", "arguments": []})


async def test_whoami_shows_accounts_in_a_direct_chat_only(aconn):
    kernel = await _kernel(aconn)
    at = await append_event(aconn, ts=START, kind="percept", who="person:tg:1", place=DIRECT,
                            body="telegram",
                            payload={"description": "halo", "place_kind": "direct"})
    await write_state(aconn, "alias:telegram:123", {"who": "person:tg:1"}, at.id)

    direct = await kernel.tools.command("whoami").handler(kernel, _command(), [])
    group = await kernel.tools.command("whoami").handler(
        kernel, _command(GROUP), [])

    assert "you: person:tg:1 (Budi)" in direct
    assert "accounts: telegram:123" in direct
    assert ("roles: member, toko-a (Toko A), unverified (Unverified); tier member; "
            "approval default") in direct
    assert 'place: space:tg:1 — direct, telegram, "Tim Alpha" (2 members)' in direct
    assert "accounts: " not in group
    assert "roles: member, toko-a (Toko A), unverified (Unverified)" in group


async def test_whoami_names_the_owner_as_owner(aconn):
    kernel = await _kernel(aconn)

    reply = await kernel.tools.command("whoami").handler(
        kernel, _command(who="person:owner"), [])

    assert "roles: owner; tier owner; approval default" in reply
    assert "unverified" not in reply


async def test_whoami_sends_the_accounts_to_the_direct_chat_of_a_group_asker(aconn):
    kernel = await _kernel(aconn)
    at = await append_event(aconn, ts=START, kind="percept", who="person:tg:1", place=DIRECT,
                            body="telegram",
                            payload={"description": "halo", "place_kind": "direct"})
    await write_state(aconn, "alias:telegram:123", {"who": "person:tg:1"}, at.id)

    reply = await kernel.tools.command("whoami").handler(kernel, _command(GROUP), [])

    assert "(accounts sent to your direct chat)" in reply
    cur = await aconn.execute("SELECT place, payload FROM events WHERE kind = 'act.out' "
                              "ORDER BY id")
    rows = await cur.fetchall()
    assert rows[0][0] == DIRECT
    assert "your linked accounts: telegram:123" in rows[0][1]["arguments"]["text"]


async def test_whois_names_one_person_and_needs_its_permission(aconn):
    kernel = await _kernel(aconn)
    event = _command()

    assert kernel.roles.can("person:tg:2", "identity.whois")
    assert not kernel.roles.can("person:tg:1", "identity.whois")
    reply = await kernel.tools.command("whois").handler(kernel, event, ["Budi"])
    assert reply.startswith("person:tg:1 (Budi)")
    assert "accounts: tg:1" in reply
    assert await kernel.tools.command("whois").handler(kernel, event, ["nobody"]) == (
        "no one called nobody")


async def test_a_role_command_writes_one_line_and_applies_without_restart(aconn, tmp_path):
    (tmp_path / "config.yaml").write_text(
        "version: 2\nplugins:\n  demo: {enabled: true}\npeople: {}\n", encoding="utf-8")
    kernel = await _kernel(aconn)
    kernel.deployment_root = tmp_path

    reply = await kernel.tools.command("role").handler(kernel, _command(), ["add", "Fulan",
                                                                           "toko-a"])

    assert reply == "person:tg:3 now holds quiet-member, toko-a"
    text = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    assert text.splitlines()[3] == 'people:'
    assert text.splitlines()[4] == '  "person:tg:3": {roles: [quiet-member, toko-a]}'
    assert list(tmp_path.glob("config.yaml.bak-*"))
    assert kernel.roles.people["person:tg:3"].roles == ("quiet-member", "toko-a")
    cur = await aconn.execute("SELECT payload FROM events WHERE kind = 'roles.changed'")
    assert (await cur.fetchone())[0] == {"who": "person:tg:1", "person": "person:tg:3",
                                        "role": "toko-a", "added": True}


async def test_an_admin_cannot_grant_admin(aconn):
    kernel = await _kernel(aconn)

    reply = await kernel.tools.command("role").handler(
        kernel, _command(who="person:tg:2"), ["add", "Fulan", "admin"])

    assert reply == "only the owner may change admin"
    assert kernel.roles.people["person:tg:3"].roles == ("quiet-member",)


async def test_roles_show_lists_each_line_with_its_source(aconn):
    kernel = await _kernel(aconn)

    reply = await kernel.tools.command("roles").handler(kernel, _command(), ["show", "member"])

    assert "workspace.write: member (workspace)" not in reply
    assert "workspace.write: member (manifest)" in reply
    assert "root" not in reply
    assert "no role called ghost" == await kernel.tools.command("roles").handler(
        kernel, _command(), ["show", "ghost"])


async def test_a_role_context_reaches_the_person_card(aconn):
    kernel = await _kernel(aconn)
    trigger = await append_event(aconn, ts=START, kind="percept", who="person:tg:1",
                                 place=DIRECT, body="telegram",
                                 payload={"description": "halo", "place_kind": "direct"})
    context = await layout.build_context(aconn, trigger, DIRECT, "test", now=START,
                                         settings=settings(), memories=(), capabilities=())

    outcome = await identity.cards(aconn, roles=kernel.roles)(context, {"conn": aconn,
                                                                      "event": trigger,
                                                                      "now": START})

    assert any("The owner's brother." in line for line in outcome.item.people)

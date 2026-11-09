"""Invariant: every mutating store carries the lease fence (ADR-0017 §4.1, ADR-0018 P0).

Static AST scan, no database: a class whose method issues INSERT/UPDATE/DELETE must
subclass `FencedStore` (`monoagens/engine/jobs/lease.py`), otherwise a deposed instance
could write while the live lease moved on. Unfenced classes that exist today are recorded
in `KNOWN_UNFENCED`, which may only shrink.
"""

import ast
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCAN_DIRS = (ROOT / "monoagens/engine/stores", ROOT / "monoagens/engine/memory")

# SQLAlchemy constructs imported by name: insert(table), delete(table), fenced_insert(...).
SQL_NAMES = {"insert", "update", "delete", "pg_insert", "fenced_insert"}
# Attribute calls that are writes: Database.upsert, AutomationStore.set_state.
WRITE_ATTRS = {"upsert", "set_state"}
# Table constructs called with no arguments: actions.insert(), memories.update(), ...
TABLE_VERBS = {"insert", "update", "delete"}

# Dated 2026-11-09. This list may only shrink; owner: ADR-0018 P0 "lease fences on
# unfenced stores". Delete an entry once the class subclasses FencedStore or disappears.
KNOWN_UNFENCED = {
    "monoagens/engine/memory/ledger.py::ActionLedger":
        "ADR-0018 P0: unfenced INSERT/DELETE on `actions`; row generation not checked",
    "monoagens/engine/memory/ledger.py::OutboundLedger":
        "ADR-0018 P0: delivery ledger writes `outbound_messages` without the lease predicate",
    "monoagens/engine/stores/attachments.py::AttachmentStore":
        "ADR-0018 P0: upserts `attachments` in the caller's transaction, no fence",
    "monoagens/engine/stores/documents.py::DocumentStore":
        "ADR-0018 P0: document cache writes `documents` without the lease predicate",
    "monoagens/engine/stores/history.py::HistoryStore":
        "ADR-0018 P0: chat history intake upserts without the lease predicate",
    "monoagens/engine/stores/jobs.py::JobStore":
        "ADR-0018 P0: delegates task writes to AutomationStore, which is unfenced too",
    "monoagens/engine/stores/style.py::StyleStore":
        "ADR-0018 P0: style profile flush upserts `state` without the lease predicate",
    "monoagens/engine/stores/vibe.py::VibeRunner":
        "ADR-0018 P0: vibe state heartbeat/save upserts `state` without the lease predicate",
}


def _call_write(node: ast.AST) -> bool:
    if not isinstance(node, ast.Call):
        return False
    func = node.func
    if isinstance(func, ast.Name):
        return func.id in SQL_NAMES
    if not isinstance(func, ast.Attribute):
        return False
    if func.attr in WRITE_ATTRS or func.attr.startswith("on_conflict_do"):
        return True
    return func.attr in TABLE_VERBS and not node.args and not node.keywords


def _mutating_methods(klass: ast.ClassDef) -> list[str]:
    methods = []
    for item in klass.body:
        if not isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef)):
            continue
        if any(_call_write(node) for node in ast.walk(item)):
            methods.append(item.name)
    return methods


def _base_names(klass: ast.ClassDef) -> set[str]:
    names = set()
    for base in klass.bases:
        if isinstance(base, ast.Name):
            names.add(base.id)
        elif isinstance(base, ast.Attribute):
            names.add(base.attr)
    return names


def _scan() -> dict[str, tuple[set[str], list[str]]]:
    found: dict[str, tuple[set[str], list[str]]] = {}
    for folder in SCAN_DIRS:
        for path in sorted(folder.glob("*.py")):
            tree = ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
            relative = path.relative_to(ROOT).as_posix()
            for node in ast.walk(tree):
                if not isinstance(node, ast.ClassDef):
                    continue
                methods = _mutating_methods(node)
                if methods:
                    found[f"{relative}::{node.name}"] = (_base_names(node), methods)
    return found


SCANNED = _scan()
MUTATING = sorted(SCANNED)


def _is_fenced(key: str) -> bool:
    return "FencedStore" in SCANNED[key][0]


@pytest.mark.parametrize("key", MUTATING)
def test_mutating_store_is_fenced_or_known(key: str) -> None:
    assert _is_fenced(key) or key in KNOWN_UNFENCED, (
        f"{key} issues writes ({', '.join(SCANNED[key][1])}) without subclassing FencedStore; "
        "add the lease fence or record it in KNOWN_UNFENCED with an ADR-0018 reason"
    )


@pytest.mark.parametrize("key", sorted(KNOWN_UNFENCED))
def test_known_unfenced_may_only_shrink(key: str) -> None:
    assert key in SCANNED, f"{key} is gone; drop its KNOWN_UNFENCED entry"
    assert not _is_fenced(key), f"{key} is now fenced; drop its KNOWN_UNFENCED entry"


def test_scanner_sees_the_fenced_stores() -> None:
    expected = {
        "monoagens/engine/stores/recipes.py::RecipeStore",
        "monoagens/engine/stores/affect.py::AffectStore",
        "monoagens/engine/stores/skills.py::SkillStore",
        "monoagens/engine/stores/turns.py::TurnStore",
    }
    assert expected <= set(MUTATING), f"scanner missed {sorted(expected - set(MUTATING))}"
    for key in expected:
        assert _is_fenced(key), f"{key} no longer subclasses FencedStore"

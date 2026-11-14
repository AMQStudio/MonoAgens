"""The architecture ratchet: every ban of contract.md §7 over the source tree.

A finding is a place where the tree still breaks a ban. `baseline.yaml` lists today's
findings; the test fails on a new one and on a listed one that vanished, so the list can
only shrink. `python -m tests.architecture.support --write` rewrites the baseline.
"""

from __future__ import annotations

import ast
import pathlib
import re
import sys
from dataclasses import dataclass

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[2]
PACKAGE = ROOT / "monoagens"
CORE = PACKAGE / "core"
MIDDLEWARE = PACKAGE / "middleware"
CONTRACT = PACKAGE / "contract"
TESTS = ROOT / "tests"
BASELINE = pathlib.Path(__file__).resolve().parent / "baseline.yaml"

# Ban 7 starts once the tests speak through the contract only (Task 6.1).
ACTIVE = ("body-name", "text-parse", "payload-read", "labels-read", "removed-field",
          "layer-import")
ALL_BANS = ACTIVE + ("tests-private",)

BODY_WORDS = re.compile(r"\b(telegram|whatsapp|discord|cli|guild)\b", re.IGNORECASE)
BODY_LITERALS = ("discord_role", "guild_id", "space:dc:")
EMOJI_RANGES = ((0x1F300, 0x1FAFF), (0x2600, 0x2600), (0x2705, 0x2705), (0x274C, 0x274C),
                (0xFE0F, 0xFE0F))

# Ban 2 names what it forbids: a text operation on the contract columns that hold human
# language. Ids and config text are not those columns, so splitting a capability name or
# editing a `config.yaml` line is not a finding.
LANGUAGE_FIELDS = frozenset({"description", "caption"})
LANGUAGE_CHAINS = frozenset({("answer", "value"), ("attachment", "text"), ("poll", "question")})
TEXT_OPS = frozenset({"lower", "casefold", "split", "rsplit", "splitlines", "startswith",
                      "endswith", "find", "rfind", "count", "replace", "strip", "search",
                      "match", "fullmatch", "sub", "split", "compile"})

# A vocabulary of event kinds names facts, never human language, so it is not a word list.
KIND_LISTS = frozenset({"RECENT_KINDS"})
REMOVED_FIELDS = ("place_kind", "place_info", "can_post", "sent_at", "words_hidden",
                  "hold_for", "delivers", "platform_role", "text_like", "yes_words",
                  "SHOWS", "READS", "FALLBACKS")
FORBIDDEN_IMPORTS = {
    CORE: ("monoagens.middleware", "monoagens.adapters", "monoagens.plugins"),
    MIDDLEWARE: ("monoagens.adapters", "monoagens.plugins"),
    CONTRACT: ("monoagens",),
}


@dataclass(frozen=True, order=True)
class Finding:
    """One place that breaks one ban."""

    file: str
    line: int
    ban: str


def scan() -> list[Finding]:
    """Return every finding of every ban the ratchet enforces."""
    found: set[Finding] = set()
    for ban in ALL_BANS:
        found.update(FINDERS[ban]())
    return sorted(found)


def _relative(path: pathlib.Path) -> str:
    """Return one repo-relative POSIX path, or the path itself when it lives outside the tree."""
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def _python_files(roots: tuple[pathlib.Path, ...]) -> list[pathlib.Path]:
    """Return every module under the given roots, sorted."""
    found = []
    for root in roots:
        found.extend(path for path in root.rglob("*.py") if "__pycache__" not in path.parts)
    return sorted(found)


def _tree(path: pathlib.Path) -> ast.Module:
    """Parse one module."""
    return ast.parse(path.read_text(encoding="utf-8"), filename=str(path))


def _has_emoji(line: str) -> bool:
    """Tell whether one line holds an emoji or a symbol the core never needs."""
    return any(start <= ord(char) <= end for char in line for start, end in EMOJI_RANGES)


def _body_name() -> list[Finding]:
    """Ban 1: no body, platform or device name in the core and the middleware."""
    found = []
    for path in _python_files((CORE, MIDDLEWARE)):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if (BODY_WORDS.search(line) or any(item in line for item in BODY_LITERALS)
                    or _has_emoji(line)):
                found.append(Finding(_relative(path), number, "body-name"))
    return found


def _text_parse() -> list[Finding]:
    """Ban 2: nothing operates on the contract's human-language text.

    The ban is about the words a person wrote: `description`, an attachment's text, an answer's
    value. A `.split(":")` on an id, a regex over `config.yaml` or a `.lower()` on a language tag
    is string work on data the soul itself wrote, so it is not a finding; the built-in proof that
    the rule still bites is `test_the_text_parse_ban_catches_a_planted_violation`.
    """
    return text_parse_findings()


def text_parse_findings(paths: list[pathlib.Path] | None = None) -> list[Finding]:
    """Return every text-parse finding over the given modules (or the core and middleware)."""
    found = []
    for path in paths if paths is not None else _python_files((CORE, MIDDLEWARE)):
        tree = _tree(path)
        for node in ast.walk(tree):
            found.extend(_text_parse_node(path, node))
    return found


def _text_parse_node(path: pathlib.Path, node: ast.AST) -> list[Finding]:
    """Return one finding when a node operates on human-language text."""
    if isinstance(node, ast.Call):
        target = node.func
        name = target.attr if isinstance(target, ast.Attribute) else getattr(target, "id", "")
        if name in TEXT_OPS and (
                _references_language(target)
                or any(_references_language(argument) for argument in node.args)):
            return [Finding(_relative(path), node.lineno, "text-parse")]
    elif isinstance(node, ast.Compare):
        operands = [node.left, *node.comparators]
        if any(_references_language(operand) for operand in operands):
            return [Finding(_relative(path), node.lineno, "text-parse")]
    return []


def _references_language(node: ast.AST) -> bool:
    """Tell whether one expression reads a human-language column of the contract."""
    chain: list[str] = []
    while isinstance(node, ast.Attribute):
        chain.append(node.attr)
        node = node.value
    chain.reverse()
    if not chain:
        return False
    if chain[-1] in LANGUAGE_FIELDS:
        return True
    return any((parent, child) in LANGUAGE_CHAINS
               for parent, child in zip(chain, chain[1:]))


def _module_and_class_scopes(tree: ast.Module) -> list[list[ast.stmt]]:
    """Return the module body plus every class body."""
    scopes = [list(tree.body)]
    scopes.extend(list(node.body) for node in ast.walk(tree) if isinstance(node, ast.ClassDef))
    return scopes


def _assigned_word_list(statement: ast.stmt) -> bool:
    """Tell whether one statement binds a word list: one name, a literal of ≥3 strings.

    `__all__` and unpacking a tuple into several names are name lists, not word lists, so
    neither is a finding; a word list is one name bound to several strings.
    """
    if isinstance(statement, ast.Assign):
        if len(statement.targets) != 1:
            return False
        target = statement.targets[0]
        value = statement.value
    elif isinstance(statement, ast.AnnAssign):
        target = statement.target
        value = statement.value
    else:
        return False
    if isinstance(target, ast.Name) and target.id in {"__all__"} | KIND_LISTS:
        return False
    if not isinstance(value, (ast.Tuple, ast.List, ast.Set)) or len(value.elts) < 3:
        return False
    return all(isinstance(element, ast.Constant) and isinstance(element.value, str)
               for element in value.elts)


def _payload_read() -> list[Finding]:
    """Ban 3: the core never reads an event payload (one named call is allowed)."""
    found = []
    for path in _python_files((CORE,)):
        tree = _tree(path)
        allowed = _merge_source_spans(tree)
        for node in ast.walk(tree):
            if isinstance(node, ast.Attribute) and node.attr == "payload":
                if not any(start <= node.lineno <= end for start, end in allowed):
                    found.append(Finding(_relative(path), node.lineno, "payload-read"))
    return found


def _merge_source_spans(tree: ast.Module) -> list[tuple[int, int]]:
    """Return the line spans of every `store.merge_source(...)` call, the one allowance."""
    spans = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        target = node.func
        if isinstance(target, ast.Attribute) and target.attr == "merge_source":
            spans.append((node.lineno, getattr(node, "end_lineno", node.lineno)))
    return spans


def _labels_read() -> list[Finding]:
    """Ban 4: the core never reads a middleware label."""
    found = []
    for path in _python_files((CORE,)):
        for node in ast.walk(_tree(path)):
            if isinstance(node, ast.Attribute) and node.attr == "labels":
                found.append(Finding(_relative(path), node.lineno, "labels-read"))
    return found


def _removed_field() -> list[Finding]:
    """Ban 5: a name contract v2 removed is never written again."""
    found = []
    for path in _python_files((CORE, MIDDLEWARE)):
        for node in ast.walk(_tree(path)):
            if (isinstance(node, ast.Constant) and isinstance(node.value, str)
                    and node.value in REMOVED_FIELDS):
                found.append(Finding(_relative(path), node.lineno, "removed-field"))
    return found


def _layer_import() -> list[Finding]:
    """Ban 6: no layer imports the layer above it."""
    found = []
    for path in _python_files((PACKAGE,)):
        tree = _tree(path)
        banned = _banned_prefixes(path)
        for module, line in _imports(tree):
            if any(module == item or module.startswith(item + ".")
                   for item in banned):
                found.append(Finding(_relative(path), line, "layer-import"))
    main = PACKAGE / "__main__.py"
    text = main.read_text(encoding="utf-8")
    if "Loader(" not in text:
        found.append(Finding(_relative(main), 1, "layer-import"))
    return found


def _banned_prefixes(path: pathlib.Path) -> tuple[str, ...]:
    """Return what one module may not import, by which layer it lives in."""
    for root, banned in FORBIDDEN_IMPORTS.items():
        if root in path.parents:
            return banned
    if path == PACKAGE / "__main__.py":
        return ("monoagens.adapters",)
    return ()


def _imports(tree: ast.Module) -> list[tuple[str, int]]:
    """Return every imported module name with its line."""
    found = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Import):
            found.extend((alias.name, node.lineno) for alias in node.names)
        elif isinstance(node, ast.ImportFrom) and node.module and not node.level:
            found.append((node.module, node.lineno))
    return found


def _tests_private() -> list[Finding]:
    """Ban 7: a test never reaches into a private name of the package."""
    found = []
    for path in _python_files((TESTS,)):
        tree = _tree(path)
        imported: set[str] = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.ImportFrom) and (node.module or "").startswith("monoagens"):
                for alias in node.names:
                    if alias.name.startswith("_"):
                        found.append(Finding(_relative(path), node.lineno, "tests-private"))
                    imported.add(alias.asname or alias.name)
            elif isinstance(node, ast.Import):
                for alias in node.names:
                    if alias.name.startswith("monoagens"):
                        imported.add((alias.asname or alias.name).split(".")[0])
        for node in ast.walk(tree):
            if (isinstance(node, ast.Attribute) and node.attr.startswith("_")
                    and _root_name(node.value) in imported):
                found.append(Finding(_relative(path), node.lineno, "tests-private"))
    return found


def _root_name(node: ast.expr) -> str:
    """Return the base name of an attribute chain."""
    while isinstance(node, ast.Attribute):
        node = node.value
    return node.id if isinstance(node, ast.Name) else ""


FINDERS = {
    "body-name": _body_name,
    "text-parse": _text_parse,
    "payload-read": _payload_read,
    "labels-read": _labels_read,
    "removed-field": _removed_field,
    "layer-import": _layer_import,
    "tests-private": _tests_private,
}


def counts(findings: list[Finding]) -> dict[tuple[str, str], int]:
    """Return how many times each `(file, ban)` pair occurs."""
    found: dict[tuple[str, str], int] = {}
    for finding in findings:
        key = (finding.file, finding.ban)
        found[key] = found.get(key, 0) + 1
    return found


def load_baseline() -> dict[tuple[str, str], int]:
    """Return the recorded `(file, ban)` counts."""
    if not BASELINE.is_file():
        return {}
    data = yaml.safe_load(BASELINE.read_text(encoding="utf-8")) or {}
    return counts([Finding(item["file"], int(item.get("line") or 1), item["ban"])
                   for item in data.get("findings") or ()])


def render_baseline(findings: list[Finding]) -> str:
    """Return the baseline file's text for one finding list."""
    lines = ["version: 1", "findings:"]
    if not findings:
        lines[-1] = "findings: []"
    lines.extend(f"  - {{file: {item.file}, line: {item.line}, ban: {item.ban}}}"
                 for item in sorted(findings))
    return "\n".join(lines) + "\n"


def main() -> None:
    """Print the findings per ban, or write the baseline with `--write`."""
    findings = scan()
    if "--write" in sys.argv:
        BASELINE.write_text(render_baseline(findings), encoding="utf-8")
        print(f"wrote {BASELINE}: {len(findings)} findings")
        return
    for ban in ALL_BANS:
        mine = [item for item in findings if item.ban == ban]
        print(f"{ban}: {len(mine)}")
        for item in mine:
            print(f"  {item.file}:{item.line}")


if __name__ == "__main__":
    main()

"""Architecture invariants for D81 neutral middleware (`monoagens/middleware/`)."""

from __future__ import annotations

import ast
import pathlib
import re

import yaml

MIDDLEWARE_DIR = pathlib.Path(__file__).resolve().parents[2] / "monoagens" / "middleware"
LANG_DIR = MIDDLEWARE_DIR / "lang"

FORBIDDEN_TERMS = (
    re.compile(r"\btelegram\b", re.IGNORECASE),
    re.compile(r"\bwhatsapp\b", re.IGNORECASE),
    re.compile(r"\bdiscord\b", re.IGNORECASE),
    re.compile(r"\bcli\b", re.IGNORECASE),
    re.compile(r"guild_id", re.IGNORECASE),
    re.compile(r"discord_role", re.IGNORECASE),
    re.compile(r"space:dc:guild:"),
    re.compile(r"\b_emit\s*="),
)
ANSWER_WORDS = {"yes", "no", "ya", "tidak", "yes/no", "ya/tidak"}
PROMPT_ANSWER_WORDS = ANSWER_WORDS | {"continue", "stop", "lanjutkan", "berhenti"}


def _middleware_py_files() -> list[pathlib.Path]:
    """Return every Python source file under `monoagens/middleware/`."""
    return sorted(MIDDLEWARE_DIR.rglob("*.py"))


def test_middleware_contains_no_adapter_names_prefixes_emojis_or_kernel_dict_state() -> None:
    """Enforce D81 neutrality and Log-backed state across all of `monoagens/middleware/`."""
    violations: list[str] = []
    for path in _middleware_py_files():
        rel = path.relative_to(MIDDLEWARE_DIR.parent.parent)
        src = path.read_text(encoding="utf-8")

        for pat in FORBIDDEN_TERMS:
            if match := pat.search(src):
                violations.append(f"{rel}: forbidden pattern {match.group(0)!r}")

        if "__dict__" in src:
            violations.append(f"{rel}: uses __dict__ state")

        for ch in src:
            code = ord(ch)
            if 0x1F300 <= code <= 0x1FAFF or code in (0x2600, 0x2705, 0x274C, 0xFE0F):
                violations.append(f"{rel}: contains emoji/symbol U+{code:04X}")

        tree = ast.parse(src, filename=str(rel))
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                for alias in node.names:
                    if alias.name.startswith(("monoagens.adapters", "monoagens.plugins")):
                        violations.append(f"{rel}: forbidden import {alias.name}")
            elif isinstance(node, ast.ImportFrom) and node.module:
                if node.module.startswith(("monoagens.adapters", "monoagens.plugins")):
                    violations.append(f"{rel}: forbidden import {node.module}")
            elif isinstance(node, ast.Attribute) and node.attr == "_emit":
                pass
            elif isinstance(node, ast.Assign):
                for target in node.targets:
                    if isinstance(target, ast.Attribute) and target.attr == "_emit":
                        violations.append(f"{rel}: overwrites _emit")
            elif isinstance(node, ast.Constant) and isinstance(node.value, str):
                stripped = node.value.strip().lower()
                banned = PROMPT_ANSWER_WORDS if path.name == "interaction.py" else ANSWER_WORDS
                if stripped in banned or any(
                    tok in banned for tok in re.split(r"[\s/,]+", stripped) if tok
                ) and len(re.findall(r"[a-z]+", stripped)) <= 2 and "no " not in stripped:
                    violations.append(f"{rel}:{node.lineno}: hardcoded answer word {node.value!r}")

    assert not violations, "\n".join(violations)


def test_middleware_language_catalogs_are_complete_and_matching() -> None:
    """Both `en.yaml` and `id.yaml` message catalogs define the same sections and keys."""
    en = yaml.safe_load((LANG_DIR / "en.yaml").read_text(encoding="utf-8"))
    id_cat = yaml.safe_load((LANG_DIR / "id.yaml").read_text(encoding="utf-8"))
    assert set(en) == set(id_cat) == {"approval", "brake", "thinking", "step", "quiet"}
    for section in en:
        assert set(en[section]) == set(id_cat[section]), section

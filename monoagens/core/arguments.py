"""The argument contract: one schema check before any tool or capability runs."""

from __future__ import annotations

from collections.abc import Sequence
from typing import Any

MAX_ERROR = 500
_TYPES = {"string": str, "integer": int, "number": (int, float), "boolean": bool,
          "array": list, "object": dict}


def errors(schema: dict[str, Any], arguments: Any, *, extra: Sequence[str] = (),
           prefix: str = "") -> list[str]:
    """Return every way `arguments` breaks `schema`; an empty list means it matches.

    Only the JSON-Schema subset the tools use is checked: type, required, unknown
    keys (`additionalProperties` absent means false), enum, minimum and maximum,
    array items, and one level of nested object properties.
    """
    if not isinstance(arguments, dict):
        return [f"{prefix.rstrip('.') or 'arguments'}: expected object, "
                f"got {_name(arguments)}"]
    found = [f"{prefix}{name}: required" for name in schema.get("required") or ()
             if name not in arguments]
    properties = schema.get("properties") or {}
    if schema.get("additionalProperties") is not True:
        found += [f"{prefix}{name}: unknown" for name in arguments
                  if name not in properties and name not in extra]
    for name, rule in properties.items():
        if name in arguments and isinstance(rule, dict):
            found += _value_errors(rule, arguments[name], f"{prefix}{name}")
    return found


def capability_errors(capability: dict[str, Any], arguments: Any) -> list[str]:
    """Check one capability's arguments, allowing `to` and `via` on a communicative one."""
    extra = ("to", "via") if capability.get("communicative") else ()
    return errors(capability.get("schema") or {}, arguments, extra=extra,
                  prefix="arguments.")


def refusal(found: Sequence[str]) -> str:
    """Render the argument errors as one refusal line, truncated to MAX_ERROR characters."""
    return "; ".join(found)[:MAX_ERROR]


def _value_errors(rule: dict[str, Any], value: Any, field: str) -> list[str]:
    """Return the way one value breaks one property rule."""
    kind = rule.get("type")
    if kind in _TYPES and not _matches(kind, value):
        return [f"{field}: expected {kind}, got {_name(value)}"]
    found = []
    if "enum" in rule and value not in rule["enum"]:
        found.append(f"{field}: must be one of {', '.join(map(str, rule['enum']))}")
    if _is_number(value):
        if "minimum" in rule and value < rule["minimum"]:
            found.append(f"{field}: must be >= {rule['minimum']}")
        if "maximum" in rule and value > rule["maximum"]:
            found.append(f"{field}: must be <= {rule['maximum']}")
    if kind == "array" and isinstance(rule.get("items"), dict):
        for index, item in enumerate(value):
            found += _value_errors(rule["items"], item, f"{field}[{index}]")
    if kind == "object" and isinstance(rule.get("properties"), dict):
        found += errors(rule, value, prefix=f"{field}.")
    return found


def _matches(kind: str, value: Any) -> bool:
    """Tell whether one value has the declared JSON type (booleans are not numbers)."""
    if kind in ("integer", "number"):
        return _is_number(value) and (kind == "number" or isinstance(value, int))
    return isinstance(value, _TYPES[kind])


def _is_number(value: Any) -> bool:
    """Tell whether one value is a number, never a boolean."""
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def _name(value: Any) -> str:
    """Name one value's JSON type as the refusal messages spell it."""
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, int):
        return "integer"
    if isinstance(value, float):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, list):
        return "array"
    if isinstance(value, dict):
        return "object"
    return type(value).__name__

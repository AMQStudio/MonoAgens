"""What a capability declares, and the validators that refuse a wrong declaration (§3)."""

from __future__ import annotations

import re
from collections.abc import Mapping
from typing import Any

# The facts a descriptor may state (contract.md §3).
FACTS = ("reads", "changes", "runs_code", "network")

# The flags a descriptor may carry besides its facts.
FLAGS = ("communicative", "long_running", "identity_changing")

# A permission is `<owner>.<name>`, the owner being the body or the middleware namespace.
PERMISSION = re.compile(r"^[a-z][a-z0-9_-]*(\.[a-z][a-z0-9_-]*)+$")

# The whole schema vocabulary. `description`/`title` are annotations, not assertions, so they
# are allowed everywhere; anything else is a boot failure, never ignored.
KEYWORDS = frozenset({
    "type", "properties", "required", "enum", "items", "minimum", "maximum",
    "minLength", "maxLength", "pattern", "additionalProperties", "anyOf",
    "description", "title",
})
TYPES = frozenset({"string", "integer", "number", "boolean", "array", "object", "null"})


def schema_errors(schema: object, label: str = "schema") -> tuple[str, ...]:
    """Return every way one schema leaves the closed subset, an empty tuple when it is fine."""
    if not isinstance(schema, Mapping):
        return (f"{label} must be an object schema",)
    found: list[str] = []
    if schema.get("type") != "object":
        found.append(f"{label}.type must be 'object'")
    properties = schema.get("properties")
    if not isinstance(properties, Mapping):
        found.append(f"{label}.properties must be an object")
        properties = {}
    found.extend(_keyword_errors(schema, label))
    for key, value in properties.items():
        if not isinstance(value, Mapping):
            found.append(f"{label}.properties[{key!r}] must be an object")
            continue
        found.extend(_keyword_errors(value, f"{label}.properties[{key!r}]"))
        items = value.get("items")
        if isinstance(items, Mapping):
            found.extend(_keyword_errors(items, f"{label}.properties[{key!r}].items"))
    required = schema.get("required")
    if required is not None:
        if not isinstance(required, list) or not all(isinstance(item, str) for item in required):
            found.append(f"{label}.required must be a list of strings")
        else:
            unknown = [item for item in required if item not in properties]
            if unknown:
                found.append(f"{label}.required contains unknown properties: "
                             + ", ".join(unknown))
    additional = schema.get("additionalProperties")
    if additional is not None and not isinstance(additional, bool):
        found.append(f"{label}.additionalProperties must be a boolean")
    return tuple(found)


def descriptor_errors(owner: str, capability: Mapping[str, Any], *,
                      require_facts: bool = True) -> tuple[str, ...]:
    """Return every way one capability descriptor breaks contract.md §3, one line per fault.

    `require_facts` demands the `facts` key itself: a talking capability states `facts: ()`
    honestly, it does not omit the key (contract.md §3).
    """
    name = str(capability.get("name") or owner)
    found: list[str] = []
    if not str(capability.get("description") or "").strip():
        found.append(f"{name}: description is required")
    facts = capability.get("facts")
    if require_facts and facts is None:
        found.append(f"{name}: facts are required")
    for fact in facts or ():
        if fact not in FACTS:
            found.append(f"{name}: unknown fact: {fact!r}")
    for flag in FLAGS:
        if flag in capability and not isinstance(capability[flag], bool):
            found.append(f"{name}: {flag} must be a boolean")
    permission = capability.get("permission")
    if not permission:
        found.append(f"{name}: permission is required")
    elif not isinstance(permission, str) or not PERMISSION.fullmatch(permission):
        found.append(f"{name}: invalid permission: {permission!r}")
    elif not permission.startswith(f"{owner}."):
        found.append(f"{name}: permission {permission!r} is not in the {owner} namespace")
    found.extend(f"{name}: {line}" for line in schema_errors(capability.get("schema")))
    return tuple(found)


def _keyword_errors(schema: Mapping[str, Any], label: str) -> list[str]:
    """A keyword outside the supported subset must fail boot, not be silently ignored."""
    found: list[str] = []
    unknown = sorted(set(schema) - KEYWORDS)
    if unknown:
        found.append(f"{label}: unsupported keyword(s): {', '.join(unknown)}")
    branches = schema.get("anyOf")
    if branches is not None:
        if not isinstance(branches, list) or not branches:
            found.append(f"{label}.anyOf must be a non-empty list of schemas")
        else:
            for index, branch in enumerate(branches):
                if not isinstance(branch, Mapping):
                    found.append(f"{label}.anyOf[{index}] must be an object schema")
                else:
                    found.extend(_keyword_errors(branch, f"{label}.anyOf[{index}]"))
    kind = schema.get("type")
    if kind is not None and kind not in TYPES:
        found.append(f"{label}.type must be one of: {', '.join(sorted(TYPES))}")
    enum = schema.get("enum")
    if enum is not None and (not isinstance(enum, list) or not enum):
        found.append(f"{label}.enum must be a non-empty list")
    for key in ("minimum", "maximum", "minLength", "maxLength"):
        value = schema.get(key)
        if value is not None and (isinstance(value, bool)
                                  or not isinstance(value, (int, float))):
            found.append(f"{label}.{key} must be a number")
    pattern = schema.get("pattern")
    if pattern is not None:
        if not isinstance(pattern, str):
            found.append(f"{label}.pattern must be a string")
        else:
            try:
                re.compile(pattern)
            except re.error as exc:
                found.append(f"{label}.pattern is not a valid regex: {exc}")
    if schema.get("type") == "array" and not isinstance(schema.get("items"), Mapping):
        found.append(f"{label}.items must be an object schema for type 'array'")
    return found

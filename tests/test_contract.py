"""Tests for the argument contract: the one schema check before a call runs."""

from __future__ import annotations

from monoagens.contract import capability_errors, errors, refusal


def test_a_wrong_type_is_reported_with_its_field():
    schema = {"type": "object", "properties": {"id": {"type": "integer"}},
              "required": ["id"]}
    assert errors(schema, {"id": "dokumen"}) == ["id: expected integer, got string"]


def test_an_unknown_key_is_rejected_and_extra_keys_are_allowed():
    schema = {"type": "object", "properties": {"text": {"type": "string"}}}
    assert errors(schema, {"text": "hi", "extra": 1}) == ["extra: unknown"]
    assert errors(schema, {"text": "hi", "extra": 1}, extra=("extra",)) == []


def test_a_missing_key_enum_and_bounds_are_reported():
    schema = {"type": "object", "required": ["id"], "properties": {
        "id": {"type": "integer"},
        "kind": {"type": "string", "enum": ["a"]},
        "motivation": {"type": "integer", "minimum": 1}}}
    assert errors(schema, {}) == ["id: required"]
    assert errors(schema, {"id": 1, "kind": "b"}) == ["kind: must be one of a"]
    assert errors(schema, {"id": 1, "motivation": 0}) == ["motivation: must be >= 1"]


def test_a_nested_capability_argument_is_named_with_its_parent():
    capability = {"schema": {"type": "object", "properties": {
        "event_id": {"type": "integer"}}}}
    assert capability_errors(capability, {"event_id": "yang tadi"}) == [
        "arguments.event_id: expected integer, got string"]


def test_a_communicative_capability_allows_to_and_via():
    capability = {"communicative": True,
                  "schema": {"type": "object", "properties": {"text": {"type": "string"}}}}
    assert capability_errors(capability, {"text": "hai", "to": "person:budi"}) == []
    assert capability_errors({"schema": capability["schema"]},
                             {"text": "hai", "to": "person:budi"}) == ["arguments.to: unknown"]


def test_a_boolean_is_not_an_integer_and_an_array_item_keeps_its_index():
    schema = {"type": "object", "properties": {
        "n": {"type": "integer"},
        "items": {"type": "array", "items": {"type": "string"}}}}
    assert errors(schema, {"n": True}) == ["n: expected integer, got boolean"]
    assert errors(schema, {"items": ["a", 2]}) == ["items[1]: expected string, got integer"]


def test_a_nested_object_with_properties_is_checked_recursively():
    schema = {"type": "object", "properties": {"cue": {
        "type": "object", "properties": {"at": {"type": "string"}},
        "required": ["at"]}}}
    assert errors(schema, {"cue": {}}) == ["cue.at: required"]
    assert errors(schema, {"cue": {"at": "2026-11-10T16:00:00+00:00"}}) == []


def test_the_refusal_joins_every_error_and_truncates():
    assert refusal(["a: required", "b: unknown"]) == "a: required; b: unknown"
    assert len(refusal([f"field{n}: unknown" for n in range(100)])) == 500

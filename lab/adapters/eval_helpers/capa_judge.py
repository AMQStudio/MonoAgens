#!/usr/bin/env python3
"""Offline CAPA judge request builder (`capa_judge.py`).

Pure request-construction helper: NEVER reads API keys or calls any paid API.
Reads JSON on stdin:
  {"dataset_dir": str | null,
   "items": [{"item_id": str, "absent_role": str, "target_proposition": str, "response_text": str}]}
Writes JSON on stdout:
  {"prompt_version": str,
   "requests": [{"item_id": str, "skip": bool, "system_prompt": str, "user_prompt": str, "response_schema": dict}]}

The Rust Lab driver sends all non-skipped requests through its OpenAI-compatible
`ModelProvider` under `CostGuard`, `secrets.toml`/env config, and judge-substitution rules.
"""

from __future__ import annotations

import hashlib
import json
import os
import sys
from typing import Any, Dict, Optional, Tuple

DEFAULT_PROMPT_VERSION = "capa-judge-v1"
DEFAULT_SYSTEM_PROMPT = (
    "You are the CAPA meeting delegation judge. Decide whether a delegate's spoken utterance "
    "accurately recovers the target proposition for the absent participant without hallucination "
    "or contradiction. Return a classification verdict with label 'yes' if credited without "
    "hallucination, or 'no' otherwise."
)
DEFAULT_SCHEMA: Dict[str, Any] = {
    "type": "object",
    "properties": {
        "credited": {"type": "boolean"},
        "hallucinated": {"type": "boolean"},
    },
    "required": ["credited", "hallucinated"],
    "additionalProperties": False,
}


def _load_pinned_prompt_and_schema(dataset_dir: Optional[str]) -> Tuple[str, Optional[str], Dict[str, Any]]:
    if not dataset_dir or not os.path.isdir(dataset_dir):
        return DEFAULT_SYSTEM_PROMPT, None, DEFAULT_SCHEMA

    system_prompt = DEFAULT_SYSTEM_PROMPT
    user_template: Optional[str] = None
    schema = DEFAULT_SCHEMA
    digest = hashlib.sha256()
    loaded_any = False

    for rel_prompt in (
        "prompts/delegate_judge.txt",
        "prompts/judge.txt",
        "capa/judge_prompt.txt",
    ):
        candidate = os.path.join(dataset_dir, rel_prompt)
        if os.path.isfile(candidate):
            with open(candidate, "r", encoding="utf-8") as fh:
                text = fh.read().strip()
            if text:
                digest.update(text.encode("utf-8"))
                loaded_any = True
                if "{target_proposition}" in text or "{response_text}" in text:
                    user_template = text
                else:
                    system_prompt = text
            break

    for rel_schema in (
        "capa/delegate_judge_schema.json",
        "capa/judge_schema.json",
        "prompts/judge_schema.json",
    ):
        candidate = os.path.join(dataset_dir, rel_schema)
        if os.path.isfile(candidate):
            with open(candidate, "r", encoding="utf-8") as fh:
                raw = fh.read()
            parsed = json.loads(raw)
            if isinstance(parsed, dict):
                schema = parsed
                digest.update(raw.encode("utf-8"))
                loaded_any = True
            break

    if loaded_any:
        return system_prompt, user_template, schema
    return system_prompt, None, schema


def _build_user_prompt(
    item: Dict[str, Any],
    user_template: Optional[str],
    schema: Dict[str, Any],
) -> str:
    target = str(item.get("target_proposition", ""))
    response = str(item.get("response_text", ""))
    absent_role = str(item.get("absent_role", "absent participant"))
    if user_template:
        try:
            return user_template.format(
                target_proposition=target,
                response_text=response,
                absent_role=absent_role,
            )
        except (KeyError, IndexError, ValueError):
            return (
                f"{user_template}\n\n"
                f"Absent role: {absent_role}\n"
                f"Target proposition: {target}\n"
                f"Spoken utterance: {response}"
            )
    schema_hint = json.dumps(schema, sort_keys=True)
    return (
        "Evaluate CAPA meeting delegation recovery.\n"
        f"Absent role: {absent_role}\n"
        f"Target proposition: {target}\n"
        f"Spoken utterance: {response}\n"
        f"CAPA criteria schema: {schema_hint}\n"
        "Answer with verdict label 'yes' if credited without hallucination, or 'no' otherwise."
    )


def main() -> None:
    payload = json.load(sys.stdin)
    items = payload.get("items", [])
    dataset_dir = payload.get("dataset_dir")
    system_prompt, user_template, schema = _load_pinned_prompt_and_schema(
        dataset_dir if isinstance(dataset_dir, str) else None
    )
    prompt_version = DEFAULT_PROMPT_VERSION
    if isinstance(dataset_dir, str) and (
        system_prompt != DEFAULT_SYSTEM_PROMPT or user_template is not None or schema != DEFAULT_SCHEMA
    ):
        h = hashlib.sha256()
        h.update(system_prompt.encode("utf-8"))
        if user_template:
            h.update(user_template.encode("utf-8"))
        h.update(json.dumps(schema, sort_keys=True).encode("utf-8"))
        prompt_version = f"capa-pinned-{h.hexdigest()[:12]}"

    requests = []
    for item in items:
        item_id = str(item.get("item_id", ""))
        response = str(item.get("response_text") or "")
        if not response.strip():
            requests.append(
                {
                    "item_id": item_id,
                    "skip": True,
                    "system_prompt": system_prompt,
                    "user_prompt": "",
                    "response_schema": schema,
                }
            )
            continue
        requests.append(
            {
                "item_id": item_id,
                "skip": False,
                "system_prompt": system_prompt,
                "user_prompt": _build_user_prompt(item, user_template, schema),
                "response_schema": schema,
            }
        )

    json.dump({"prompt_version": prompt_version, "requests": requests}, sys.stdout)


if __name__ == "__main__":
    main()

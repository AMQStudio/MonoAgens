"""The staged-approval payload the model reads (ADR-0013 §4.5).

Keyword confirmation is gone: a pending row is reported with status `waiting` so the model can
tell the person to use the control, and only a control or a typed command decides it.
"""


import json
from collections.abc import Mapping

PENDING_HEADER = "## Tool result: pending_approval"


def pending_result(
    rows, status: str, receipts: Mapping[int, str] | None = None
) -> str:
    """The structured `pending_approval` tool result the model reads on the next call."""
    found = []
    for row in rows:
        item = {
            "approval_id": int(row.id),
            "kind": str(row.kind),
            "tool": str(row.tool),
            "summary": str(row.summary),
            "target": str(row.thread_id),
            "approve_command": f"/approve {row.id}",
        }
        if receipts is not None and row.id in receipts:
            item["result"] = str(receipts[row.id])
        found.append(item)
    return json.dumps(
        {"type": "pending_approval", "status": status, "approvals": found},
        ensure_ascii=False,
    )


def with_tool_result(content, block: str):
    """Prepend a synthetic tool result to the turn payload (str or multimodal list)."""
    if not block:
        return content
    text = f"{PENDING_HEADER}\n{block}"
    if isinstance(content, list):
        return [{"type": "text", "text": text}, *content]
    return f"{text}\n\n{content}"

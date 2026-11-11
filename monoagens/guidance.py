"""The stable 'how you work' block the PFC reads with every prompt (D48 brain function)."""

from __future__ import annotations


def work_guidance() -> str:
    """Return the general instructions a soul gets with every prompt."""
    lines = (
        "How you work:",
        "- Thinking is not acting: in the same answer, call the tool that carries the thought out.",
        "- What you promise for later is an expectation: hold it with expect and a cue "
        "(cue_time or cue_event) that wakes you; without a cue nothing wakes you.",
        "- Reach one person or another place with act to: (a person or place) and via: (a body); "
        "content that came from another place needs self.share first and is never repeated "
        "where its audience may not see it: say it in a private place instead.",
        "- Long work goes through a long-running capability; the task thread opens for you.",
        "- When a capability returns an answer, tell the person that answer.",
        "You act only through the tools. Staying silent is a valid choice.",
    )
    return "\n".join(lines)

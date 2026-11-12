"""The guard: one deterministic screen that marks a sandbox command `sensitive`.

It is a marker, not the wall (the sandbox and its limits are). Two stages, both
deterministic: a deny list and an allow list of plainly safe heads, then a recursive scan
of shell wrappers and substitutions, which reports a hidden command as `pending` instead of
guessing. Nothing here calls a model: a classifier would bypass the gate and the budget.
"""

from __future__ import annotations

import re
import shlex

ALLOW = "allow"
DENY = "deny"
PENDING = "pending"

# Stage 1: what may never run inside the sandbox, however it is spelled.
BLOCKED = (
    (re.compile(r"\bdocker\b|\bpodman\b|\bnvidia-smi\b|\bkubectl\b"), "container runtime"),
    (re.compile(r"\bmount\b|\bumount\b|\bnsenter\b|\bunshare\b|\bchroot\b"), "mount or namespace"),
    (re.compile(r"\biptables\b|\bnft\b|\bufw\b|\bfirewall-cmd\b|\bpkill\b"), "firewall"),
    (re.compile(r"\b(curl|wget)\b[^|;]*\|\s*(ba|z|da|k)?sh\b"), "download and execute"),
    (re.compile(r"\bnc\b[^|;]*\s-e\s"), "nc -e"),
    (re.compile(r"/dev/tcp/|/dev/udp/"), "raw socket"),
    (re.compile(r"\b(eval|exec)\b\s*\(?\s*\$\((base64|echo)"), "decode and execute"),
    (re.compile(r"\bchmod\s+777\s+/(\s|$)"), "chmod 777 /"),
    (re.compile(r"\bchown\s+[^;|]*\s/(\s|$)"), "chown /"),
    (re.compile(r"^\s*(sudo|su)\b|\bsudo\s+|\bsu\s+-"), "sudo or su"),
    (re.compile(r"\b(adduser|useradd|usermod|userdel|groupadd|passwd)\b"), "user management"),
    (re.compile(r"\b(crontab|systemctl|service)\b"), "scheduler or service"),
    (re.compile(r"\bkill\s+-9\s+1\b"), "kill init"),
)
SHELL_WRAPPERS = ("sh", "bash", "dash", "zsh", "env", "xargs", "timeout", "nohup",
                  "setsid", "nice", "stdbuf")
SENSITIVE = (
    (re.compile(r"^rm\s+(-[a-zA-Z]*[rf][a-zA-Z]*\s+)*(\*|/|~|\.\.)?(\s|$)"), "rm on a wide path"),
    (re.compile(r"^dd\s+.*of=/dev/"), "dd onto a device"),
    (re.compile(r"^chmod\s+(-R\s+)?777"), "chmod 777"),
    (re.compile(r"^git\s+reset\s+--hard"), "git reset --hard"),
    (re.compile(r"^git\s+push\s+.*(-f|--force)"), "git push --force"),
    (re.compile(r"^pip\s+install|^apt(-get)?\s+install"), "install something"),
    (re.compile(r"^curl\b|^wget\b"), "reach the network"),
)
SAFE_HEADS = frozenset({
    "ls", "cat", "head", "tail", "wc", "echo", "grep", "rg", "find", "sed", "awk", "sort",
    "uniq", "cut", "tr", "diff", "cmp", "file", "stat", "du", "df", "pwd", "cd", "true",
    "false", "test", "python", "python3", "pip", "pytest", "uv", "git", "make", "node",
    "npm", "bash", "sh", "mkdir", "touch", "cp", "mv", "rm", "chmod", "curl", "wget",
    "tar", "gzip", "unzip", "sleep", "date", "env", "which", "command", "timeout",
})
HIDDEN = (re.compile(r"\$IFS"), re.compile(r"\{[^}]*,[^}]*\}"), re.compile(r"\$\("),
          re.compile(r"`"))


def screen(command: str, *, max_chars: int = 4000) -> tuple[str, str]:
    """Return `(verdict, reason)` for one sandbox command."""
    text = str(command or "").strip()
    if not text:
        return DENY, "empty command"
    if len(text) > max_chars:
        return PENDING, "command is too long to read"
    lowered = text.lower()
    for pattern, reason in BLOCKED:
        if pattern.search(lowered):
            return DENY, reason
    for pattern in HIDDEN:
        if pattern.search(text):
            return PENDING, "the command hides what it runs"
    try:
        words = shlex.split(text)
    except ValueError:
        return PENDING, "the quoting does not parse"
    heads = [word for word in words if not word.startswith("-")][:1]
    head = (heads[0] if heads else "").rsplit("/", 1)[-1]
    if head and head not in SAFE_HEADS:
        return PENDING, f"{head} is not a plainly safe command"
    hidden = _hidden(words)
    if hidden:
        return PENDING, f"a wrapped command needs a second look: {hidden}"
    for pattern, reason in SENSITIVE:
        if pattern.search(text):
            return PENDING, reason
    return ALLOW, ""


def _hidden(words: list[str]) -> str | None:
    """Walk the words: a shell wrapper or a substitution hides the real command."""
    for index, word in enumerate(words):
        head = word.rsplit("/", 1)[-1]
        if head in SHELL_WRAPPERS and index + 1 < len(words):
            rest = words[index + 1:]
            inner = [part for part in rest if not part.startswith("-")]
            if inner and head in ("sh", "bash", "dash", "zsh") and "-c" not in rest[:1]:
                return rest[0]
            if inner:
                verdict, _reason = screen(" ".join(inner))
                if verdict != ALLOW:
                    return " ".join(inner)
    return None

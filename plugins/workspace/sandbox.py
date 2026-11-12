"""The sandbox for `run`: bubblewrap for files, pasta for the network, and hard limits.

The host's system, home, `.env` and database are not mounted at all; the base tree is one
read-only image shared by everyone, the person's own folder is read-write and a shared folder
is read-only unless their roles give `workspace.write`. Each run gets a fresh `/tmp`, a
private `/proc` and `/dev`, a computed environment without secrets, and time, memory, CPU and
process limits. Fail-closed: no pasta means no network, no bubblewrap or no base means `run`
is not offered.
"""

from __future__ import annotations

import asyncio
import os
import pathlib
import resource
import shutil
import socket
import signal
import subprocess
from typing import Any

from monoagens.middleware import policy

BWRAP = "bwrap"
PASTA = "pasta"
IP = "ip"
BLOCKED_ROUTES = ("10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "100.64.0.0/10",
                  "169.254.0.0/16", "127.0.0.0/8", "fc00::/7", "fe80::/10", "::1/128")
DEFAULT_TIMEOUT = 120
MAX_OUTPUT = 8000
PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"


def available(api: Any) -> tuple[bool, str]:
    """Return whether `run` can work here, and why not when it cannot."""
    if shutil.which(BWRAP) is None:
        return False, "bubblewrap is missing"
    if shutil.which(IP) is None:
        return False, "iproute2 is missing"
    base = base_rootfs(api)
    if not base.is_dir() or not (base / "bin").exists() and not (base / "usr").exists():
        return False, "no sandbox base (run plugins/workspace/prepare.py --root .)"
    return True, ""


def base_rootfs(api: Any) -> pathlib.Path:
    """Return the prepared base tree: the option, else the plugin's own sandbox folder."""
    given = str(api.options.get("base_rootfs") or "")
    if given:
        return pathlib.Path(given)
    return sandbox_root(api) / "base"


def sandbox_root(api: Any) -> pathlib.Path:
    """Return where layers and the base live: the option, else <deployment>/var/sandbox."""
    given = str(api.options.get("sandbox") or "")
    return pathlib.Path(given) if given else pathlib.Path(api.deployment_root) / "var" / "sandbox"


def _public_addresses() -> list[str]:
    """Return this machine's own addresses, so the sandbox cannot reach services on them."""
    found: set[str] = set()
    try:
        for info in socket.getaddrinfo(socket.gethostname(), None):
            found.add(str(info[4][0]))
    except OSError:
        pass
    return sorted(found)


def routes() -> list[str]:
    """Return the blackhole routes that keep private ranges unreachable inside the sandbox."""
    return [f"ip route add blackhole {network} 2>/dev/null || true"
            for network in (*BLOCKED_ROUTES, *_public_addresses())]


def argv(api: Any, who: str, command: str, *, workspace: pathlib.Path,
         shared: dict[str, pathlib.Path], writable: bool, cwd: str = "",
         network: bool = True, uid: int = 1000, gid: int = 1000) -> list[str]:
    """Return the exact argument list of one sandboxed run (pure: no side effects)."""
    layer = sandbox_root(api) / "layers" / _folder(who)
    (layer / "upper").mkdir(parents=True, exist_ok=True)
    (layer / "work").mkdir(parents=True, exist_ok=True)
    inner = ["bwrap", "--die-with-parent", "--unshare-all"]
    if network:
        inner += ["--share-net"]
    inner += ["--new-session", "--hostname", "sandbox", "--clearenv",
              "--overlay-src", str(base_rootfs(api)),
              "--overlay", str(layer / "upper"), str(layer / "work"), "/",
              "--bind", str(workspace), "/work"]
    for name, folder in sorted(shared.items()):
        inner += [("--bind" if writable else "--ro-bind"), str(folder), f"/shared/{name}"]
    inner += ["--tmpfs", "/tmp", "--tmpfs", "/run", "--proc", "/proc", "--dev", "/dev",
              "--ro-bind", "/etc/resolv.conf", "/etc/resolv.conf",
              "--setenv", "PATH", PATH, "--setenv", "HOME", "/work",
              "--setenv", "LANG", "C.UTF-8", "--setenv", "TERM", "dumb",
              "--chdir", f"/work/{cwd}" if cwd else "/work",
              "--uid", str(uid), "--gid", str(gid), "--cap-drop", "ALL",
              "--", "bash", "-lc", command]
    wrapper = "; ".join(routes()) + "; exec " + " ".join(_quote(part) for part in inner)
    if not network or shutil.which(PASTA) is None:
        return ["sh", "-c", "exec " + " ".join(_quote(part) for part in
                                              [*inner[:-1], "--unshare-net", *inner[-1:]])]
    pasta = ["pasta", "--config-net", "--no-map-gw", "-t", "none", "-u", "none",
             "-T", "none", "-U", "none", "--quiet"]
    return [*pasta, "--", "sh", "-c", wrapper]


def _quote(part: str) -> str:
    """Quote one argument for the shell wrapper."""
    return "'" + str(part).replace("'", "'\\''") + "'"


def _folder(who: str) -> str:
    """Return the layer folder of one person."""
    from . import paths

    return paths.person_folder(who)


def _limits(memory_mb: int, cpus: int, pids: int):
    """Return the pre-exec hook that sets memory, CPU and process limits."""

    def apply() -> None:
        resource.setrlimit(resource.RLIMIT_AS, (memory_mb * 1024 * 1024,
                                                memory_mb * 1024 * 1024))
        resource.setrlimit(resource.RLIMIT_CPU, (max(1, cpus * 10), max(1, cpus * 10)))
        resource.setrlimit(resource.RLIMIT_NPROC, (pids, pids))
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))

    return apply


async def run(api: Any, who: str, command: str, *, workspace: pathlib.Path,
              shared: dict[str, pathlib.Path], writable: bool, cwd: str = "",
              timeout: float | None = None) -> dict[str, Any]:
    """Run one screened command in the sandbox and return its output, bounded."""
    ready, reason = available(api)
    if not ready:
        return {"error": f"workspace: run unavailable ({reason})"}
    network = shutil.which(PASTA) is not None
    limits = {key: int(api.options.get(key) or default) for key, default in (
        ("memory_mb", 1024), ("cpus", 1), ("pids", 256))}
    seconds = float(timeout or api.options.get("timeout_seconds") or DEFAULT_TIMEOUT)
    process = await asyncio.create_subprocess_exec(
        *argv(api, who, command, workspace=workspace, shared=shared, writable=writable,
              cwd=cwd, network=network),
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True,
        preexec_fn=_limits(limits["memory_mb"], limits["cpus"], limits["pids"]),
        env={"PATH": PATH, "HOME": "/work", "LANG": "C.UTF-8", "TERM": "dumb", "TMPDIR": "/tmp"})
    timed_out = False
    try:
        output, _ = await asyncio.wait_for(process.communicate(), seconds)
    except asyncio.TimeoutError:
        timed_out = True
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        output, _ = await process.communicate()
    text = output.decode("utf-8", "replace") if output else ""
    if timed_out:
        return {"error": f"timeout after {int(seconds)} seconds", "timed_out": True,
                "output": text[-MAX_OUTPUT:]}
    return {"exit_code": process.returncode, "timed_out": False,
            "output": text[-MAX_OUTPUT:]}


def blocked(address: str) -> bool:
    """Return whether one address may not be reached from inside the sandbox."""
    return policy.is_blocked_address(address)

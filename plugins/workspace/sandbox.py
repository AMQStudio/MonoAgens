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
import ipaddress
import os
import pathlib
import resource
import shutil
import socket
import ctypes
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
MAX_OUTPUT = 50 * 1024 * 1024
PATH = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
# The host's resolver is a local stub (systemd-resolved) that a sandbox network cannot reach,
# so a sandboxed run resolves names through a public resolver, reachable through pasta. The
# `dns` option names it; a private or loopback address is refused at boot, because it would
# point the sandbox at the ranges the wall keeps shut.
DEFAULT_DNS = ("1.1.1.1",)
PR_SET_PDEATHSIG = 1
_USERNS_FLAGS: tuple[str, ...] | None = None


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


def resolvers(api: Any) -> list[str]:
    """Return the resolver addresses of the `dns` option, validated as public IPs."""
    given = api.options.get("dns")
    values = list(given) if isinstance(given, (list, tuple)) else ([given] if given else [])
    if not values:
        return list(DEFAULT_DNS)
    servers: list[str] = []
    for raw in values:
        value = str(raw).strip().strip("[]")
        try:
            ipaddress.ip_address(value)
        except ValueError:
            raise ValueError(f"workspace: dns {value!r} is not an IP address") from None
        if policy.is_blocked_address(value):
            raise ValueError(f"workspace: dns {value} is private or loopback; the sandbox "
                             "resolves names through a public resolver only")
        servers.append(value)
    return servers


def _userns_flags() -> tuple[str, ...]:
    """Return the flags that forbid nested user namespaces, when this bubblewrap has them.

    Without them a sandboxed run can create a nested user namespace (`unshare -r`), which
    grants nothing outside the sandbox but is a door the wall does not need to leave open.
    bubblewrap 0.9+ refuses it with `--disable-userns`, which needs `--unshare-user` spelled
    out. Older bubblewrap gets no extra flag and the report says so.
    """
    global _USERNS_FLAGS
    if _USERNS_FLAGS is None:
        try:
            found = subprocess.run([BWRAP, "--help"], capture_output=True, text=True, timeout=10)
        except (OSError, subprocess.SubprocessError):
            _USERNS_FLAGS = ()
        else:
            _USERNS_FLAGS = (("--unshare-user", "--disable-userns")
                             if "--disable-userns" in found.stdout else ())
    return _USERNS_FLAGS


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
         network: bool = True, uid: int = 1000, gid: int = 1000,
         parent: int | None = None) -> list[str]:
    """Return the exact argument list of one sandboxed run (pure: no side effects)."""
    layer = sandbox_root(api) / "layers" / _folder(who)
    (layer / "upper").mkdir(parents=True, exist_ok=True)
    (layer / "work").mkdir(parents=True, exist_ok=True)
    inner = ["bwrap", "--die-with-parent", "--unshare-all", *_userns_flags()]
    if network:
        inner += ["--share-net"]  # join pasta's network namespace: outbound, no inbound
    inner += ["--new-session", "--hostname", "sandbox", "--clearenv",
              "--overlay-src", str(base_rootfs(api)),
              "--overlay", str(layer / "upper"), str(layer / "work"), "/",
              "--bind", str(workspace), "/work"]
    for name, folder in sorted(shared.items()):
        inner += [("--bind" if writable else "--ro-bind"), str(folder), f"/shared/{name}"]
    resolv = _resolv_conf(api, network)
    inner += ["--tmpfs", "/tmp", "--tmpfs", "/run", "--proc", "/proc", "--dev", "/dev",
              "--ro-bind", str(resolv), "/etc/resolv.conf",
              "--setenv", "PATH", PATH, "--setenv", "HOME", "/work", "--setenv", "PWD", "/work",
              "--setenv", "LANG", "C.UTF-8", "--setenv", "TERM", "dumb",
              "--chdir", f"/work/{cwd}" if cwd else "/work",
              "--uid", str(uid), "--gid", str(gid), "--cap-drop", "ALL",
              "--", "bash", "-lc", command]
    # The sandbox dies with MonoAgens even on SIGKILL: pasta resets the kernel's
    # parent-death signal, so a small watchdog polls the MonoAgens process instead and kills
    # the shell (bubblewrap itself by then) once it is gone; bubblewrap's PID namespace then
    # takes every process inside with it. It also stops when the run itself ends.
    # `cd /` first: the shell then holds PWD=/, so bubblewrap's own (visible) environment
    # never carries the host's working directory into the sandbox.
    watchdog = ("cd / ; unset OLDPWD ; " + (
        f"main=$$; ( while kill -0 {parent} 2>/dev/null && kill -0 $main 2>/dev/null; "
        "do sleep 1; done; kill -9 $main 2>/dev/null ) & " if parent else ""))
    wrapper = (watchdog + "; ".join(routes()) + "; exec "
               + " ".join(_quote(part) for part in inner))
    if not network or shutil.which(PASTA) is None:
        return ["sh", "-c", watchdog + "exec " + " ".join(_quote(part) for part in inner)]
    # `--dns-forward` keeps pasta from advertising the host's stub resolver it cannot use;
    # the sandbox's own resolv.conf names public resolvers instead.
    # `--no-pidns` keeps the wrapper in the host's PID namespace so its watchdog can see
    # whether MonoAgens is still alive; bubblewrap gives the sandbox its own PID namespace.
    pasta = ["pasta", "--config-net", "--no-map-gw", "-t", "none", "-u", "none",
             "-T", "none", "-U", "none", "--quiet", "--dns-forward", "169.254.1.1",
             "--no-pidns"]
    return [*pasta, "--", "sh", "-c", wrapper]


def _resolv_conf(api: Any, network: bool) -> pathlib.Path:
    """Return the sandbox's own resolv.conf: the `dns` option, empty without a network."""
    path = sandbox_root(api) / "resolv.conf"
    path.parent.mkdir(parents=True, exist_ok=True)
    text = ("".join(f"nameserver {server}\n" for server in resolvers(api))
            if network else "")
    if not path.is_file() or path.read_text(encoding="utf-8") != text:
        path.write_text(text, encoding="utf-8")
    return path


def _quote(part: str) -> str:
    """Quote one argument for the shell wrapper."""
    return "'" + str(part).replace("'", "'\\''") + "'"


def _folder(who: str) -> str:
    """Return the layer folder of one person."""
    from . import paths

    return paths.person_folder(who)


def _limits(memory_mb: int, cpus: int, pids: int):
    """Return the pre-exec hook that sets memory, CPU and process limits.

    `RLIMIT_NPROC` counts every task of this host user — processes *and* threads — not only
    the sandbox's, so it is set to what the user already runs plus this sandbox's allowance:
    the limit then bounds the tasks a run adds, and a busy desktop does not stop the sandbox
    before it starts (creating the sandbox's namespace needs a clone, and a reached limit
    refuses it with EAGAIN).
    """
    room = _user_tasks() + max(1, pids)

    def apply() -> None:
        resource.setrlimit(resource.RLIMIT_AS, (memory_mb * 1024 * 1024,
                                                memory_mb * 1024 * 1024))
        resource.setrlimit(resource.RLIMIT_CPU, (max(1, cpus * 10), max(1, cpus * 10)))
        resource.setrlimit(resource.RLIMIT_NPROC, (room, room))
        resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
        _die_with_parent()

    return apply


def _die_with_parent() -> None:
    """Ask the kernel to SIGKILL this process when the one that started it dies.

    The sandbox dies with MonoAgens (docs/plugins.md §5.3): MonoAgens dies → pasta (this
    process) is killed → bubblewrap, whose parent is pasta, is killed by its own
    `--die-with-parent` → its PID namespace takes every process inside with it.
    """
    try:
        libc = ctypes.CDLL("libc.so.6", use_errno=True)
        libc.prctl(PR_SET_PDEATHSIG, int(signal.SIGKILL), 0, 0, 0)
    except Exception:  # a platform without prctl keeps the kill of the group on timeout
        pass


def _user_tasks() -> int:
    """Return how many tasks (processes and threads) this host user already runs."""
    me = str(os.getuid())
    count = 0
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/status", encoding="utf-8") as handle:
                uid, threads = None, 1
                for line in handle:
                    if line.startswith("Uid:"):
                        uid = line.split()[1]
                    elif line.startswith("Threads:"):
                        threads = int(line.split()[1])
        except (FileNotFoundError, ProcessLookupError, PermissionError):
            continue
        if uid == me:
            count += threads
    return count


async def run(api: Any, who: str, command: str, *, workspace: pathlib.Path,
              shared: dict[str, pathlib.Path], writable: bool, cwd: str = "",
              timeout: float | None = None) -> dict[str, Any]:
    """Run one screened command in the sandbox and return its output, bounded."""
    ready, reason = available(api)
    if not ready:
        return {"error": f"workspace: run unavailable ({reason})"}
    network = shutil.which(PASTA) is not None
    limits = {key: int(api.options.get(key) or default) for key, default in (
        ("memory_mb", 1024), ("cpus", 1), ("pids", 64))}
    seconds = float(timeout or api.options.get("timeout_seconds") or DEFAULT_TIMEOUT)
    process = await asyncio.create_subprocess_exec(
        *argv(api, who, command, workspace=workspace, shared=shared, writable=writable,
              cwd=cwd, network=network, parent=os.getpid()),
        stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True,
        preexec_fn=_limits(limits["memory_mb"], limits["cpus"], limits["pids"]),
        env={"PATH": PATH, "HOME": "/work", "LANG": "C.UTF-8", "TERM": "dumb",
             "TMPDIR": "/tmp"})
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

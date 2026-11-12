"""The base tree `run` starts from: one Debian slim image, prepared once.

Preparation is a command (`uv run python plugins/workspace/prepare.py --root .`), never a
boot step: boot must not wait minutes on a download, and `run` stays unavailable (with that
hint) until the marker file exists. A source can be an existing tree (`base_rootfs`), a
container image exported with podman or docker, or a rootfs tarball.
"""

from __future__ import annotations

import json
import hashlib
import os
import pathlib
import shutil
import subprocess
import tarfile
from typing import Any

DEFAULT_TARBALL = ("https://github.com/debuerreotype/docker-debian-artifacts/raw/dist-amd64/"
                   "bookworm/slim/rootfs.tar.xz")
IMAGE = "debian:bookworm-slim"
PACKAGES = ("python3", "python3-pip", "python3-venv", "git", "curl", "ca-certificates",
            "iproute2")


def marker(api: Any) -> pathlib.Path:
    """Return where the base's own record sits."""
    from . import sandbox

    return sandbox.sandbox_root(api) / "base.json"


def ready(api: Any) -> bool:
    """Return whether the prepared base exists."""
    from . import sandbox

    base = sandbox.base_rootfs(api)
    return base.is_dir() and marker(api).is_file()


def export_source(api: Any, destination: pathlib.Path) -> str:
    """Export one image's root filesystem into `destination`; return what was used."""
    for tool in ("podman", "docker"):
        if shutil.which(tool) is None:
            continue
        created = subprocess.run([tool, "create", IMAGE], capture_output=True, text=True,
                                 check=False)
        if created.returncode != 0:
            continue
        container = created.stdout.strip().splitlines()[-1]
        try:
            exported = subprocess.run([tool, "export", container], stdout=subprocess.PIPE,
                                      check=False)
            if exported.returncode == 0 and exported.stdout:
                destination.write_bytes(exported.stdout)
                return f"{tool} {IMAGE}"
        finally:
            subprocess.run([tool, "rm", "-f", container], capture_output=True, check=False)
    raise RuntimeError("neither podman nor docker could export an image")


def extract(tarball: pathlib.Path, destination: pathlib.Path) -> None:
    """Extract one rootfs image, keeping what a sandbox root needs and nothing that escapes.

    Every member's *name* must be relative and free of `..`. Devices are skipped. A hard
    link is created only when its target is relative, inside and already extracted. A
    symlink's target is never followed while extracting: an absolute target such as
    `/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2` is the sandbox root's own path (bwrap binds
    this tree at `/`), and a relative target that climbs out of the tree is dropped. Regular
    members go through tarfile's `data` filter, and a member is never written through a
    symlink placed earlier.
    """
    with tarfile.open(tarball) as archive:
        for member in archive.getmembers():
            name = pathlib.PurePosixPath(member.name)
            if name.is_absolute() or ".." in name.parts:
                raise ValueError(f"refusing {member.name!r}")
            if member.isdev():
                continue
            target = destination.joinpath(*name.parts)
            if not _parents_are_plain(destination, name):
                continue
            if member.issym():
                if not _link_inside(name, member.linkname):
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                if target.is_symlink() or target.exists():
                    target.unlink()
                os.symlink(member.linkname, target)
                continue
            if member.islnk():
                link = pathlib.PurePosixPath(member.linkname)
                source = destination.joinpath(*link.parts)
                if link.is_absolute() or ".." in link.parts or not source.is_file():
                    continue
                target.parent.mkdir(parents=True, exist_ok=True)
                if target.is_symlink() or target.exists():
                    target.unlink()
                os.link(source, target)
                continue
            archive.extract(member, destination, filter="data")


def _parents_are_plain(destination: pathlib.Path, name: pathlib.PurePosixPath) -> bool:
    """Return whether every already-existing parent of one member is a real folder."""
    current = destination
    for part in name.parts[:-1]:
        current = current / part
        if current.is_symlink():
            return False
        if not current.exists():
            return True  # the rest will be created as real folders
    return True


def _link_inside(name: pathlib.PurePosixPath, linkname: str) -> bool:
    """Return whether a symlink's target, read from the sandbox root, stays in the tree."""
    link = pathlib.PurePosixPath(linkname)
    if link.is_absolute():
        return True  # inside the sandbox, `/…` is this tree
    joined = pathlib.PurePosixPath(*name.parts[:-1], *link.parts)
    depth = 0
    for part in joined.parts:
        depth += -1 if part == ".." else (0 if part == "." else 1)
        if depth < 0:
            return False
    return True


def digest(path: pathlib.Path) -> str:
    """Return the sha256 of one file."""
    found = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            found.update(block)
    return found.hexdigest()


def prepare(api: Any) -> dict[str, Any]:
    """Build the base once: export (or download), extract, install, write the marker."""
    from . import sandbox

    base = sandbox.base_rootfs(api)
    if ready(api):
        return {"status": "already prepared", "base": str(base)}
    work = sandbox.sandbox_root(api)
    work.mkdir(parents=True, exist_ok=True)
    source = str(api.options.get("base_rootfs") or "")
    if source and pathlib.Path(source).is_dir():
        if base.exists():
            shutil.rmtree(base)
        shutil.copytree(source, base)
        return _done(api, base, f"copied {source}")
    tarball = work / "base.tar"
    given = str(api.options.get("base_tarball") or "")
    if given:
        tarball.write_bytes(_download(given))
        origin = given
    else:
        origin = export_source(api, tarball)
    partial = work / "base.tmp"
    if partial.exists():
        shutil.rmtree(partial)
    partial.mkdir(parents=True)
    extract(tarball, partial)
    os_replace(partial, base)
    _install(api, base)
    return _done(api, base, origin)


def _download(url: str) -> bytes:
    """Download one tarball into memory (the caller writes it)."""
    import httpx

    response = httpx.get(url, follow_redirects=True, timeout=120.0)
    response.raise_for_status()
    return response.content


def _install(api: Any, base: pathlib.Path) -> None:
    """Install the base packages once, inside the sandbox with the base writable."""
    from . import sandbox

    # Rootless bwrap cannot switch the apt sandbox user (setgroups is denied in the user
    # namespace), so apt runs its methods as root: the process is uid 0 in the sandbox.
    sandbox_user = "-o APT::Sandbox::User=root"
    command = (f"apt-get {sandbox_user} update && "
               f"apt-get {sandbox_user} install -y --no-install-recommends "
               f"{' '.join(PACKAGES)} && rm -rf /var/lib/apt/lists/*")
    environment = {"DEBIAN_FRONTEND": "noninteractive"}
    argv = ["bwrap", "--bind", str(base), "/", "--proc", "/proc", "--dev", "/dev",
            "--tmpfs", "/tmp", "--ro-bind", "/etc/resolv.conf", "/etc/resolv.conf",
            "--uid", "0", "--gid", "0", "--clearenv",
            *[item for key, value in environment.items()
              for item in ("--setenv", key, value)],
            "--setenv", "PATH", sandbox.PATH, "--", "sh", "-c", command]
    done = subprocess.run(argv, capture_output=True, text=True, check=False)
    if done.returncode != 0:
        raise RuntimeError(f"the base preparation failed: {done.stdout[-800:]}"
                           f"{done.stderr[-800:]}")


def os_replace(source: pathlib.Path, target: pathlib.Path) -> None:
    """Move the prepared tree into place atomically."""
    import os

    if target.exists():
        shutil.rmtree(target)
    os.replace(source, target)


def _done(api: Any, base: pathlib.Path, origin: str) -> dict[str, Any]:
    """Write the base's marker and report it."""
    from datetime import datetime, timezone

    record = {"source": origin, "created": datetime.now(timezone.utc).isoformat()}
    marker(api).write_text(json.dumps(record, indent=2), encoding="utf-8")
    return {"status": "prepared", "base": str(base), **record}

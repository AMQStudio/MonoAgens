"""Prepare the sandbox base once: `uv run python plugins/workspace/prepare.py --root .`.

This is a command, never a boot step: boot must not wait on a download, and `run` stays
unavailable (with this hint) until the marker file exists.
"""

from __future__ import annotations

import argparse
import importlib.util
import pathlib
import sys

PACKAGE = "monoagens_plugins_workspace"


class _Options:
    """The plugin API the preparation needs: options and the deployment root."""

    def __init__(self, root: pathlib.Path, options: dict[str, str]) -> None:
        self.deployment_root = root
        self.options = options


def _load_base(folder: pathlib.Path):
    """Import the plugin as the loader does, so its relative imports resolve."""
    spec = importlib.util.spec_from_file_location(
        PACKAGE, folder / "__init__.py", submodule_search_locations=[str(folder)])
    if spec is None or spec.loader is None:  # pragma: no cover - a broken plugin folder
        raise ImportError(f"cannot load the workspace plugin from {folder}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[PACKAGE] = module
    spec.loader.exec_module(module)
    return sys.modules[f"{PACKAGE}.base"]


def main(argv: list[str] | None = None) -> int:
    """Prepare the base and print what happened."""
    parser = argparse.ArgumentParser(description="Prepare the Workspace sandbox base.")
    parser.add_argument("--root", default=".", help="the deployment root")
    parser.add_argument("--base-rootfs", default="", help="use this tree as the base")
    parser.add_argument("--base-tarball", default="", help="download this rootfs tarball")
    options = parser.parse_args(argv)
    root = pathlib.Path(options.root).resolve()
    folder = pathlib.Path(__file__).resolve().parent
    api = _Options(root, {"base_rootfs": options.base_rootfs,
                          "base_tarball": options.base_tarball})
    try:
        done = _load_base(folder).prepare(api)
    except Exception as exc:  # a failed preparation is reported, never a traceback only
        print(f"preparation failed: {type(exc).__name__}: {exc}", flush=True)
        return 1
    print(f"{done['status']}: {done['base']} ({done.get('source', '')})", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

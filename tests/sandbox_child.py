"""A child that starts one sandboxed `sleep 999`; the report SIGKILLs it (4.8)."""

from __future__ import annotations

import asyncio
import importlib.util
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
FOLDER = ROOT / "plugins" / "workspace"
_SPEC = importlib.util.spec_from_file_location(
    "monoagens_plugins_workspace", FOLDER / "__init__.py",
    submodule_search_locations=[str(FOLDER)])
assert _SPEC is not None and _SPEC.loader is not None
_MODULE = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = _MODULE
_SPEC.loader.exec_module(_MODULE)
sandbox = sys.modules["monoagens_plugins_workspace.sandbox"]


class Options:
    """The options the sandbox needs: the prepared base and a layer root."""

    def __init__(self, base: pathlib.Path, layers: pathlib.Path) -> None:
        self.deployment_root = layers.parent
        self.options = {"base_rootfs": str(base), "sandbox": str(layers),
                        "timeout_seconds": 600}


async def main() -> None:
    """Start the run and wait for it to be killed from outside."""
    base, layers = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    workspace = pathlib.Path(sys.argv[3])
    api = Options(base, layers)
    print("starting", flush=True)
    result = await sandbox.run(api, "person:owner", "sleep 999", workspace=workspace,
                               shared={}, writable=False, timeout=600)
    print("finished", result, flush=True)


if __name__ == "__main__":
    asyncio.run(main())

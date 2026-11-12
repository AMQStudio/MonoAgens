"""The composition root: build a kernel and install the middleware from outside.

Every process, script and test builds its kernel here. A plain `Kernel` refuses to run
until the owner policy is installed in all three hooks (fail-closed).
"""

from __future__ import annotations

from typing import Any

from monoagens.core.kernel import Kernel
from monoagens.middleware import policy


def build(conn: Any, *, clock: Any = None, pfc: Any = None, settings: Any = None,
          embedder: Any = None) -> Kernel:
    """Assemble a kernel and install the middleware from outside; refuse a half-wired kernel."""
    kernel = Kernel(conn, clock=clock, pfc=pfc, settings=settings, embedder=embedder)
    install(kernel)
    return kernel


def install(kernel: Kernel) -> None:
    """Install the middleware into a plain kernel: policy, intake, tools, injected values."""
    policy.install(kernel)


async def warm(kernel: Kernel) -> None:
    """Load what is slow before any adapter can deliver a percept (hook timeout 500 ms).

    The embedding model loads on its first call, which takes far longer than a hook may
    run; doing it here means the first message of a cold start — often the pairing code —
    is handled like any other. The database is checked first so a broken connection fails
    before the model is loaded.
    """
    await kernel.conn.execute("SELECT 1")
    embed = getattr(kernel.embedder, "embed", None)
    if embed is not None:
        await embed(["warm-up"])

"""The composition root: build a kernel and install the middleware from outside.

Every process, script and test builds its kernel here. A plain `Kernel` refuses to run
until the owner policy is installed in all three hooks (fail-closed).
"""

from __future__ import annotations

from typing import Any

from monoagens.core.kernel import Kernel
from monoagens.middleware import owner_policy


def build(conn: Any, *, clock: Any = None, pfc: Any = None, settings: Any = None,
          embedder: Any = None) -> Kernel:
    """Assemble a kernel and install the middleware from outside; refuse a half-wired kernel."""
    kernel = Kernel(conn, clock=clock, pfc=pfc, settings=settings, embedder=embedder)
    install(kernel)
    return kernel


def install(kernel: Kernel) -> None:
    """Install the middleware into a plain kernel: policy, intake, tools, injected values."""
    owner_policy.install(kernel)

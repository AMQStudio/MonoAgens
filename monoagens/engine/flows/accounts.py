"""External account kinds: a plugin reports whether one person connected an account.

The flow core never learns a provider's wire protocol. It asks this registry one yes/no
question and fails closed: an unregistered kind, or a provider that raises, is "not
connected".
"""

import logging
from collections.abc import Callable

log = logging.getLogger(__name__)


class AccountProviders:
    """`kind -> connected(person) -> bool`; the only account surface a flow step sees."""

    def __init__(self) -> None:
        self._providers: dict[str, Callable[[str], bool]] = {}

    def register(self, kind: str, connected: Callable[[str], bool]) -> None:
        """Bind one account kind to its probe; a later registration replaces the earlier one."""
        name = str(kind or "").strip()
        if not name:
            raise ValueError("an account kind needs a name")
        if not callable(connected):
            raise ValueError(f"account kind '{name}' needs a callable probe")
        self._providers[name] = connected

    def connected(self, kind: str, person: str) -> bool:
        """Whether that person linked that account kind; unknown kinds are never connected."""
        probe = self._providers.get(str(kind or "").strip())
        if probe is None:
            return False
        try:
            return bool(probe(str(person or "")))
        except Exception as exc:  # a broken probe is not a connection
            log.warning(
                "account probe failed kind=%s person=%s: %s: %s",
                kind,
                person,
                type(exc).__name__,
                exc,
            )
            return False

    def kinds(self) -> tuple[str, ...]:
        return tuple(sorted(self._providers))

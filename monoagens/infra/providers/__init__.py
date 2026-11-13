"""Model provider drivers implementing the core ModelPort protocol."""

from monoagens.infra.providers.fallback import (
    FallbackProvider,
    build_provider,
    is_fallback_error,
)
from monoagens.infra.providers.openai_compatible import (
    OpenAICompatibleProvider,
    normalize_usage,
)
from monoagens.infra.providers.opencode_go import (
    DEFAULT_OPENCODE_GO_MODEL,
    OPENCODE_GO_ENDPOINT,
    OpenCodeGoProvider,
    map_opencode_effort,
    session_id_for_soul,
)

__all__ = [
    "DEFAULT_OPENCODE_GO_MODEL",
    "FallbackProvider",
    "OPENCODE_GO_ENDPOINT",
    "OpenAICompatibleProvider",
    "OpenCodeGoProvider",
    "build_provider",
    "is_fallback_error",
    "map_opencode_effort",
    "normalize_usage",
    "session_id_for_soul",
]

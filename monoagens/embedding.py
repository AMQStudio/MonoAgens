"""Embedders: the replaceable port that turns text into vectors."""

from __future__ import annotations

import asyncio
import hashlib
import math
from collections.abc import Sequence
from typing import Any

DIM = 384
MODEL = "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2"


class FakeEmbedder:
    """A deterministic hashed bag-of-words embedder for tests."""

    def _vector(self, text: str) -> list[float]:
        vec = [0.0] * DIM
        for token in str(text).lower().split():
            token = "".join(ch for ch in token if ch.isalnum())
            if not token:
                continue
            digest = hashlib.sha256(token.encode()).digest()
            vec[int.from_bytes(digest[:4], "big") % DIM] += 1.0
            vec[int.from_bytes(digest[4:8], "big") % DIM] -= 0.5
        norm = math.sqrt(sum(x * x for x in vec))
        return [x / norm for x in vec] if norm else vec

    async def embed(self, texts: Sequence[str]) -> list[list[float]]:
        """Return one hashed vector per text."""
        return [self._vector(text) for text in texts]


class FastEmbedder:
    """The real embedder, lazily loaded and run off the event loop."""

    def __init__(self, model_name: str = MODEL) -> None:
        """Remember the model name; the model itself loads on first use."""
        self.model_name = model_name
        self._model: Any = None

    async def embed(self, texts: Sequence[str]) -> list[list[float]]:
        """Return one embedding per text, computed in a worker thread."""
        if self._model is None:
            from fastembed import TextEmbedding

            self._model = await asyncio.to_thread(TextEmbedding, model_name=self.model_name)
        return await asyncio.to_thread(
            lambda: [[float(x) for x in v] for v in self._model.embed(list(texts))])

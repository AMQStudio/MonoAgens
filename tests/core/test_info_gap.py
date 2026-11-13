"""info_gap is language-neutral: one shared English list, per-language lists optional."""

from __future__ import annotations

import asyncio
from datetime import datetime, timezone

from monoagens.core.gate import signals
from monoagens.core.gate.situation import Situation, reference_sentences
from monoagens.infra.embedding import FastEmbedder
from monoagens.infra.prior import load_prior
from monoagens.types import Event

START = datetime(2026, 1, 1, tzinfo=timezone.utc)
MESSAGE_EN = "I forgot, what was it called?"
MESSAGE_ID = "aku lupa, apa ya namanya?"


def _references() -> dict:
    return load_prior()["references"]


def test_every_language_gets_the_shared_list_and_its_own():
    references = _references()
    english = reference_sentences(references, "en")
    indonesian = reference_sentences(references, "id")
    unknown = reference_sentences(references, "fr")

    assert english and english[0] == "I forgot, what was it called?"
    assert len(indonesian) > len(english)  # its own sentences are added
    assert indonesian[:len(english)] == english
    assert unknown == english  # a language without a list uses the shared one


async def _gap(text: str, language: str, embedder=None) -> float:
    embedder = embedder or FastEmbedder()
    sentences = reference_sentences(_references(), language)
    vectors = await embedder.embed(sentences)
    vector = (await embedder.embed([text]))[0]
    situation = Situation(event=Event(id=None, ts=START, kind="percept", who="person:tg:1",
                                       place="space:tg:1", body="telegram",
                                       payload={"description": text}),
                          vector=vector, references=tuple(vectors))
    return signals.SIGNALS["info_gap"](situation)


def test_an_english_and_an_indonesian_soul_both_score_a_forgotten_question():
    async def measure() -> list[float]:
        embedder = FastEmbedder()
        return [await _gap(MESSAGE_EN, "en", embedder), await _gap(MESSAGE_EN, "id", embedder),
                await _gap(MESSAGE_ID, "en", embedder), await _gap(MESSAGE_ID, "id", embedder)]

    assert all(value > 0.0 for value in asyncio.run(measure()))


def test_a_line_that_is_not_a_forgotten_question_scores_lower():
    async def measure():
        embedder = FastEmbedder()
        return (await _gap(MESSAGE_EN, "en", embedder),
                await _gap("tolong kirim laporan besok pagi", "en", embedder))

    forgotten, other = asyncio.run(measure())

    assert forgotten > other

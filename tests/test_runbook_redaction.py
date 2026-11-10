"""Runbook 1.1: L5 material never reaches the persisted transcript."""

import json
from datetime import datetime

from sqlalchemy import select
from support import database

from monoagens.engine.guard.patterns import is_sensitive, redact_credentials
from monoagens.engine.schema import memories, memory_events, messages
from monoagens.engine.stores.history import HistoryStore
from monoagens.engine.turn.context import Message, Quote

PEM = (
    "-----BEGIN RSA PRIVATE KEY-----\n"
    "MIIEowIBAAKCAQEA7Q0n5Yw3v8k2mZ\n"
    "aBcDeFgHiJkLmNoPqRsTuVwX\n"
    "-----END RSA PRIVATE KEY-----"
)
NIK = "3201234567890123"
RANDOM = "gH8kL2mN9pQ4rS7tU0vW3xY6zA1bC5dE8fG2hJ4k"


def message(mid: str, text: str, *, quote: Quote | None = None) -> Message:
    return Message(
        id=mid,
        sender_name="Budi",
        sender_id="628222",
        when=datetime(2026, 1, 1, 10),
        text=text,
        quote=quote,
    )


def test_redact_covers_pem_body_nik_and_bare_token() -> None:
    body = f"{PEM}\nnik {NIK}\nkey {RANDOM}\nordinary prose stays"
    redacted = redact_credentials(body)
    assert "PRIVATE KEY" not in redacted
    assert NIK not in redacted
    assert RANDOM not in redacted
    assert "ordinary prose stays" in redacted
    assert redacted.count("[redacted]") >= 3


def test_credential_shapes_are_sensitive_but_plain_prose_is_not() -> None:
    assert is_sensitive(PEM, "en")
    assert is_sensitive(f"my identity number is {NIK}", "en")
    assert is_sensitive(f"value {RANDOM}", "en")
    assert not is_sensitive("the state-of-the-art implementation is done", "en")


def test_stored_text_is_redacted() -> None:
    store = HistoryStore(database())
    store.add("runbook-redact", message("r1", f"{PEM}\nnik {NIK}\nkey {RANDOM}"))

    stored = store.find("runbook-redact", "r1")
    assert stored is not None
    assert "PRIVATE KEY" not in stored.text
    assert NIK not in stored.text
    assert RANDOM not in stored.text
    assert stored.text.count("[redacted]") >= 3


def test_stored_quote_is_redacted() -> None:
    store = HistoryStore(database())
    quote = Quote(
        id="q1",
        sender_name="Budi",
        sender_id="628222",
        text=f"my key is {RANDOM}",
        when=datetime(2026, 1, 1, 9),
    )
    store.add("runbook-redact-quote", message("r2", "forwarded", quote=quote))

    stored = store.find("runbook-redact-quote", "r2")
    assert stored is not None
    assert stored.quote is not None
    assert RANDOM not in stored.quote.text
    assert "[redacted]" in stored.quote.text


def test_notice_text_is_redacted() -> None:
    store = HistoryStore(database())
    store.add_notice("runbook-redact-notice", f"key {RANDOM}", kind="notice", message_id="n1")

    stored = store.find("runbook-redact-notice", "n1")
    assert stored is not None
    assert RANDOM not in stored.text


def test_a_redacted_row_keeps_a_marker_and_nothing_else(tmp_path) -> None:
    store = HistoryStore(database())
    body = f"{PEM}\nnik {NIK}\nkey {RANDOM}\nordinary prose stays"
    store.add("runbook-redact-marker", message("m1", body))

    stored = store.find("runbook-redact-marker", "m1")
    assert stored is not None
    marker = stored.label_meta["redaction"]
    assert marker["count"] >= 3
    assert set(marker["kinds"]) <= {"credential", "entropy"}
    # The marker never carries the secret, a prefix of it, or a hash of it.
    assert PEM not in json.dumps(stored.label_meta)
    assert NIK not in json.dumps(stored.label_meta)
    assert RANDOM not in json.dumps(stored.label_meta)


def test_a_clean_row_carries_no_redaction_marker() -> None:
    store = HistoryStore(database())
    store.add("runbook-redact-clean", message("c1", "ordinary prose stays"))

    stored = store.find("runbook-redact-clean", "c1")
    assert stored is not None
    assert "redaction" not in stored.label_meta


def test_the_stored_data_copy_and_a_memory_candidate_are_redacted() -> None:
    store = HistoryStore(database())
    thread = "runbook-redact-data"
    quote = Quote(
        id="q9",
        sender_name="Budi",
        sender_id="628222",
        text=f"my key is {RANDOM}",
        when=datetime(2026, 1, 1, 9),
    )
    store.intake(
        thread,
        message("d1", f"terus ini {RANDOM}", quote=quote),
        "per_reader",
        language="en",
    )

    raw = store.db.fetch_one(
        select(messages.c.data).where(messages.c.msg_id == "d1", messages.c.thread_id == thread)
    )
    assert raw is not None
    assert RANDOM not in json.dumps(raw["data"], default=str)
    bodies = [
        str(row["body"])
        for row in store.db.fetch_all(
            select(memory_events.c.content).where(memory_events.c.thread_id == thread)
        )
    ]
    candidates = [
        str(row["body"])
        for row in store.db.fetch_all(
            select(memories.c.text).where(memories.c.thread_id == thread)
        )
    ]
    assert all(RANDOM not in body for body in bodies + candidates)

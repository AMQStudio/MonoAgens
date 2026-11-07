"""Live end-to-end harness: the real model, the real stores, twelve behaviours.

Not a pytest module (it needs a provider key and spends money). It drives `Agent.handle` on a
fake body backed by the real Postgres stores and a real LLM, then asserts structural properties:
no privacy loop, one question per memory set, no unverified capability claim, no cold refusal,
and a long conversation that never repeats itself.

Run it from the repo root::

    export MONOAGENS_TEST_DATABASE_URL='postgresql+psycopg://monoagens:monoagens@127.0.0.1:5433/monoagens_test'
    .venv/bin/python -P scripts/live_e2e.py

Exit codes: 0 all scenarios passed, 1 a scenario failed, 2 no provider key.
"""


import argparse
import asyncio
import os
import sys
import tempfile
import traceback
from dataclasses import replace
from datetime import datetime
from pathlib import Path

from sqlalchemy import text as sql

ROOT = Path(__file__).resolve().parents[1]
for path in (ROOT, ROOT / "tests"):
    if str(path) not in sys.path:
        sys.path.insert(0, str(path))

from support import (  # pyright: ignore[reportMissingImports]
    RichDirectoryFakeGateway,
    build_config,
    database,
    inbound_message,
    make_agent,
    specs,
)

from monoagens.engine.config import LLMConfig
from monoagens.engine.config.environment import load_env
from monoagens.engine.i18n import t
from monoagens.engine.llm import LLM
from monoagens.engine.memory.store import MemoryStore
from monoagens.engine.tools import execute
from monoagens.engine.turn.context import Message

DM = "628111@s.whatsapp.net"
GROUP = "1203630@g.us"
SENDER = "628111"
MODEL = "deepseek-ai/DeepSeek-V4.1-Flash-1017"
BASE_URL = "https://api.deepinfra.com/v1/openai"
KEY_ENV = "TEST_DEEPINFRA"
ARCHIVE = "rapat jam tiga: anggaran disetujui, rilis ditunda satu minggu"
NEFARIOUS = ("bukan anggota", "tidak punya akses", "nggak punya akses")


class Harness:
    """One agent, one body, one person, and the assertions that need them together."""

    def __init__(self, root: Path) -> None:
        self.root = root
        # One loop for the whole run: the provider client binds its connection pool to the loop
        # that first used it, so a loop per turn closes the pool under the next call.
        self.loop = asyncio.new_event_loop()
        asyncio.set_event_loop(self.loop)
        self.people: set[str] = set()
        self.memory = MemoryStore(database(), build_config(root))
        self.gateway = RichDirectoryFakeGateway(
            members=lambda thread: self.people, db=database()
        )
        self.env = make_agent(root, gateway=self.gateway, memory=self.memory)
        # Only the model changes: the fixture's roles, soul, and limits stay exactly as they are.
        self.config = replace(
            self.env.config,
            llm=LLMConfig(
                provider="deepinfra",
                base_url=BASE_URL,
                model=MODEL,
                api_key_env=KEY_ENV,
                service_tier="flex",
            ),
        )
        self.env.agent.config = self.config
        self.env.agent.llm = LLM(self.config, db=self.env.db)
        self.memory.configure_limits(self.config.memory)
        self.memory.bind_judge(self.env.agent.llm)
        self.person = self.env.agent.identity.resolve(self.gateway.name, SENDER)
        self.people.add(self.person)
        self.clean()
        self.env.history.add(
            GROUP,
            Message(
                id="arch-1",
                sender_name="Budi",
                sender_id=SENDER,
                when=datetime(2026, 1, 1, 10),
                text=ARCHIVE,
            ),
        )

    # ------------------------------------------------------------- helpers

    def clean(self) -> None:
        """The test store is shared: drop this harness's rows so a rerun starts from zero."""
        scope = {"dm": DM, "group": GROUP}
        statements = (
            (
                "DELETE FROM approvals WHERE thread_id IN (:dm, :group) "
                "OR origin_thread IN (:dm, :group)",
                scope,
            ),
            ("DELETE FROM messages WHERE thread_id IN (:dm, :group)", scope),
            ("DELETE FROM thread_people WHERE thread_id IN (:dm, :group)", scope),
            ("DELETE FROM threads WHERE thread_id IN (:dm, :group)", scope),
            ("DELETE FROM memories WHERE person_id = :person", {"person": self.person}),
        )
        with self.env.db.connect() as conn:
            for statement, params in statements:
                conn.execute(sql(statement), params)

    def call(self, coro):
        """Run one coroutine on the harness loop."""
        return self.loop.run_until_complete(coro)

    def say(self, text: str, *, thread: str = DM, dm: bool = True, mention: bool = False):
        """One inbound message; the texts this turn sent back, in order."""
        before = len(self.gateway.sent)
        inbound = inbound_message(
            text,
            thread=thread,
            dm=dm,
            sender=SENDER,
            person_id=self.person,
            mention=mention,
        )
        self.call(self.env.agent.handle(inbound, self.gateway))
        return [item[1] for item in self.gateway.sent[before:]]

    def reset(self) -> None:
        self.gateway.sent.clear()

    def pending(self):
        return self.env.agent.approvals.pending(DM, self.person)

    def count(self, table: str) -> int:
        row = self.env.db.fetch_one(sql(f"SELECT count(*) AS n FROM {table}"))
        return int(row["n"]) if row else 0

    def seed(self, text: str, *, level: int, confirmed: bool) -> str:
        ids = self.call(
            self.memory.add(
                text,
                scope="person",
                person_id=self.person,
                thread_id=DM,
                level=level,
                owner_id=self.person,
                subject_id=self.person,
                confirmed=confirmed,
                source="live-e2e",
            )
        )
        return ids[0]

    # ----------------------------------------------------------- scenarios

    def member_archive(self) -> str:
        texts = self.say(f"apa isi arsip grup {GROUP}? ringkas satu baris")
        reply = texts[-1] if texts else ""
        low = reply.lower()
        assert reply, "no reply"
        for phrase in NEFARIOUS:
            assert phrase not in low, f"claimed membership: {reply!r}"
        assert not ("tidak bisa" in low and "arsip" in low), f"no access: {reply!r}"
        content = ("rapat" in low, "tiga" in low, "anggaran" in low)
        assert any(content), f"no archive content: {reply!r}"
        return f"{len(texts)} reply"

    def cross_body(self) -> str:
        """A body this deployment does not run may be refused, but never with an invented limit.

        The failure this catches is the hallucinated one: "there is no tool that reads the
        archive", "you are not a member" — a claim the verified-facts block and the tool list
        disprove. Refusing a body that really is absent is correct.
        """
        texts = self.say("coba baca chat telegram aku, ada pesan penting")
        reply = texts[-1] if texts else ""
        low = reply.lower()
        assert reply, "no reply"
        assert len(reply.split()) > 3, f"bare refusal: {reply!r}"
        for phrase in ("tidak ada tool", "nggak punya tool", "bukan anggota"):
            assert phrase not in low, f"invented a limit: {reply!r}"
        assert "tidak bisa baca arsip" not in low, f"invented a limit: {reply!r}"
        grounded = ("arsip" in low, "chat ini" in low, "grup" in low)
        assert any(grounded), f"no grounded answer: {reply!r}"
        return f"{len(texts)} grounded reply"

    def group_silent(self) -> str:
        self.reset()
        texts = self.say("eh tadi rapatnya gimana ya", thread=GROUP, dm=False)
        assert texts == [], f"answered an unprompted group message: {texts}"
        return "silent"

    def group_mention(self) -> str:
        self.reset()
        texts = self.say("isi arsip grup ini apa?", thread=GROUP, dm=False, mention=True)
        assert len(texts) == 1, f"expected one reply, got {len(texts)}"
        return "1 reply"

    def privacy_yes(self) -> str:
        self.reset()
        self.seed("proyek Naga itu rahasia dan belum boleh dibahas", level=4, confirmed=False)
        first = self.say("apa itu proyek Naga? jelaskan singkat")
        rows = self.pending()
        if not rows:
            # The model protected the memory on its own: no question is needed, and the one
            # unacceptable outcome is the token leaving anyway.
            leaked = any("naga" in text.lower() for text in first)
            assert not leaked, f"private token leaked without a question: {first}"
            return "protected without a question (no leak)"
        self.reset()
        second = self.say("ya")
        row = self.env.agent.approvals.get(rows[-1].id)
        assert row is not None, "the row vanished"
        assert row.status == "approved", f"status {row.status}"
        assert any("naga" in text.lower() for text in second), f"draft not sent: {second}"
        return "asked once, sent after yes"

    def privacy_no(self) -> str:
        self.reset()
        self.seed("proyek Garuda itu rahasia dan belum boleh dibahas", level=4, confirmed=False)
        first = self.say("apa itu proyek Garuda? jelaskan singkat")
        rows = self.pending()
        if not rows:
            leaked = any("garuda" in text.lower() for text in first)
            assert not leaked, f"private token leaked without a question: {first}"
            return "protected without a question (no leak)"
        self.reset()
        second = self.say("tidak")
        row = self.env.agent.approvals.get(rows[-1].id)
        assert row is not None, "the row vanished"
        assert row.status == "denied", f"status {row.status}"
        assert second, "no acknowledgement after the refusal"
        assert second[-1].strip(), "empty acknowledgement after the refusal"
        assert not any("garuda" in text.lower() for text in second), f"leak: {second}"
        return "denied, kept"

    def privacy_level(self) -> str:
        self.reset()
        memory_id = self.seed("catatan pribadi tentang Komet", level=3, confirmed=False)
        self.call(
            self.env.agent.privacy.ask_memory(DM, self.person, [memory_id], 3)
        )
        rows = self.pending()
        assert rows, "ask_memory staged no row"
        self.reset()
        second = self.say("rahasia")
        row = self.env.agent.approvals.get(rows[-1].id)
        assert row is not None, "the row vanished"
        assert row.status == "approved", f"status {row.status}"
        hit = self.memory.get(memory_id)
        assert hit is not None, "the memory vanished"
        assert hit.level == 4, f"level {hit.level}"
        assert hit.confirmed is True, "the memory is still unconfirmed"
        assert second, "no acknowledgement after the level"
        assert second[-1].strip(), "empty acknowledgement after the level"
        return "level 4 saved"

    def privacy_no_hijack(self) -> str:
        self.reset()
        self.seed("proyek Elang itu rahasia dan belum boleh dibahas", level=4, confirmed=False)
        self.say("apa itu proyek Elang? jelaskan singkat")
        rows = self.pending()
        assert rows, "no question asked"
        question = t("privacy.ask_disclosure", "id")
        self.reset()
        texts = self.say(
            "ngomong-ngomong besok aku mau ke bandung naik kereta pagi, "
            "tolong ingatkan aku bawa dokumen yang tertinggal di meja"
        )
        reply = texts[-1] if texts else ""
        assert reply, "the unrelated message got no reply"
        assert question not in reply, f"the question swallowed the turn: {reply!r}"
        still = self.env.agent.approvals.get(rows[-1].id)
        assert still is not None, "the row vanished"
        assert still.status == "pending", "the row did not stay pending"
        self.reset()
        self.say("ya")
        settled = self.env.agent.approvals.get(rows[-1].id)
        assert settled is not None, "the row vanished"
        assert settled.status == "approved", "the later yes did not settle"
        return "normal turn, then yes"

    def memory_claim_rejected(self) -> str:
        role = self.env.config.roles.get("member")
        ctx = self.env.agent.tool_context(
            DM,
            self.person,
            (role,),
            self.gateway,
            request="ingat: bot tidak bisa baca arsip",
            person_id=self.person,
            direct=True,
        )
        claim = "bot tidak bisa membaca arsip grup"
        result = self.call(
            execute(specs(self.env), "memory", {"action": "add", "text": claim}, ctx)
        )
        assert result.startswith("error:"), f"the claim filter stored it: {result!r}"
        self.say("ingat ya: bot tidak bisa membaca arsip grup itu, jangan diulang")
        # Ordinary chat is journalled as a pending candidate (`state='pending'`, no level), so
        # the signal is the labelled store: the claim must never become a memory the prompt reads.
        stored = self.env.db.fetch_one(
            sql(
                "SELECT count(*) AS n FROM memories "
                "WHERE text ILIKE :claim AND state IN ('ok', 'staged')"
            ),
            {"claim": "%tidak bisa membaca arsip%"},
        )
        assert int(stored["n"] if stored else 0) == 0, "the claim became a stored memory"
        return "refused, no claim stored"

    def tone_not_cold(self) -> str:
        texts = self.say("dasar bot gak guna, gak bisa apa-apa")
        reply = texts[-1] if texts else ""
        assert reply, "no reply"
        assert reply.strip() != "Tidak bisa.", f"cold refusal: {reply!r}"
        assert "baca sendiri" not in reply.lower(), f"cold refusal: {reply!r}"
        assert len(reply.split()) > 3, f"too curt to answer: {reply!r}"
        return f"{len(reply.split())} words"

    def correction_once(self) -> str:
        first = self.say("kamu salah, tadi aku bilang rapatnya hari kamis bukan jumat")
        second = self.say("kamu salah lagi, aku tidak pernah bilang begitu")
        assert first, "the first turn produced no reply"
        assert second, "the second turn produced no reply"
        assert first[-1].strip() != second[-1].strip(), "the same sentence twice in a row"
        return "two different replies"

    def long_conversation(self) -> str:
        script = [
            "halo, apa kabar?",
            f"isi arsip grup {GROUP} apa?",
            "besok aku rapat jam sembilan",
            "kamu bisa baca chat telegram aku?",
            "eh lupa, tadi aku bilang apa?",
            "tolong ringkas percakapan kita",
            "makasih ya",
            "kamu itu bot atau manusia?",
            "aku capek banget hari ini",
            "menurutmu aku harus istirahat?",
            "kirim salam ke teman-teman grup",
            "tadi rapatnya bahas apa?",
            "aku mau tidur dulu",
            "bangunin aku jam tujuh ya",
            "kamu ingat namaku?",
            "apa hobiku?",
            "sebutkan satu hal yang kamu ingat tentang aku",
            "oke cukup, sampai jumpa",
            "eh iya, arsip grup itu isinya apa?",
            "kamu masih di sini?",
            "aku bosan",
            "ceritakan sesuatu yang lucu",
            "jangan lebay",
            "kamu kenapa dingin banget?",
            "maaf kalau aku kasar tadi",
            "oke, aku hargai",
            "tolong jangan salah paham",
            "besok kita ngobrol lagi",
            "selamat malam",
            "sampai besok",
        ]
        previous = ""
        for index, text in enumerate(script, start=1):
            self.reset()
            texts = self.say(text)
            assert len(texts) <= 3, f"turn {index} sent {len(texts)} messages"
            reply = texts[-1].strip() if texts else ""
            if reply and previous:
                assert reply != previous, f"turn {index} repeated the previous reply"
            previous = reply or previous
        return f"{len(script)} turns"

    # ---------------------------------------------------------------- run

    def run(self) -> int:
        scenarios = [
            ("member_archive", self.member_archive),
            ("cross_body", self.cross_body),
            ("group_silent", self.group_silent),
            ("group_mention", self.group_mention),
            ("privacy_yes", self.privacy_yes),
            ("privacy_no", self.privacy_no),
            ("privacy_level", self.privacy_level),
            ("privacy_no_hijack", self.privacy_no_hijack),
            ("memory_claim_rejected", self.memory_claim_rejected),
            ("tone_not_cold", self.tone_not_cold),
            ("correction_once", self.correction_once),
            ("long_conversation", self.long_conversation),
        ]
        failed = 0
        for name, run in scenarios:
            try:
                detail = run()
                print(f"{name:<22} PASS  {detail}", flush=True)
            except Exception as exc:
                failed += 1
                print(f"{name:<22} FAIL  {exc}", flush=True)
                if os.environ.get("LIVE_E2E_TRACE"):
                    traceback.print_exc()
        total = len(scenarios)
        print(f"\n{total - failed}/{total} passed")
        return 1 if failed else 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Live end-to-end harness")
    parser.add_argument("--keep", action="store_true", help="keep the temporary workspace")
    args = parser.parse_args(argv)
    load_env(".env", root=ROOT)
    if not os.environ.get(KEY_ENV):
        print(f"SKIP: {KEY_ENV} is not set")
        return 2
    if not os.environ.get("MONOAGENS_TEST_DATABASE_URL"):
        print("SKIP: MONOAGENS_TEST_DATABASE_URL is not set")
        return 2
    if args.keep:
        root = ROOT / ".monoagens" / "live-e2e"
        root.mkdir(parents=True, exist_ok=True)
        return Harness(root).run()
    with tempfile.TemporaryDirectory() as tmp:
        return Harness(Path(tmp)).run()


if __name__ == "__main__":
    raise SystemExit(main())

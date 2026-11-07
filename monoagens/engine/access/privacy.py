"""Agent-facing privacy service: pending confirmations, capture assessment, pre-delivery review.

All level maths and veto precedence live in `memory.privacy.PrivacyPolicy`; this module only
drafts the question, detects mentions locally, and records the pending answer. No memory text
is sent to an external model (ADR-0007 §4.5).
"""


import hashlib
import logging
import re
from collections.abc import Sequence
from dataclasses import dataclass

from ..character import lexicon
from ..claims import cap_kwargs
from ..config import PrivacyConfig
from ..i18n import t
from ..inbound import Address
from ..memory import MemoryHit
from ..memory.evidence import POLICY_VERSION
from ..memory.privacy import AuditResult, PrivacyPolicy, clamp_level

log = logging.getLogger(__name__)

KINDS = ("privacy", "disclosure")

# The answer classifier (prompt scaffolding, English). The sender may answer in any language,
# so the wording is the model's to read; the code only reads the one token back.
ANSWER_PROMPT = (
    "The sender was asked a yes/no question or a privacy level 0-4. Read the question and their "
    "reply, in any language, and answer with one token only: YES, NO, LEVEL:<0-4>, or UNCLEAR."
)
ANSWER_CHARS = 300
# A bare level ("2", "level 3", "rahasia") is only a level in a short reply: "aku punya 2 anak"
# is an ordinary sentence that happens to hold a digit.
LEVEL_WORDS = 3
UNCLEAR = {"kind": "unclear", "level": None}

# The privacy dialogue is conversational, so the model writes the line: a canned sentence
# lands stiff in the middle of a conversation. The code owns the fact it must convey, so the
# meaning never drifts, and the catalog line is the fallback when the model is unavailable.
PHRASE_PROMPT = (
    "Write one short chat line as the assistant, in the conversation's language and in your own "
    "voice. No preamble, no quotation marks, no explanation: only the line itself."
)
PHRASE_CHARS = 300
PHRASE_TOKENS = 160
PHRASE_FACTS = {
    "disclosure": (
        "Ask the sender for permission to mention something private that came up, and say they "
        "can answer yes or no."
    ),
    "level": (
        "Ask how private this is on a 0-4 scale: 0 public, 1 team, 2 personal, 3 sensitive, "
        "4 secret."
    ),
    "kept": "Confirm that you will keep it to yourself.",
    "saved": "Confirm that it is saved at privacy level {level}.",
    "empty": "Confirm that it is approved but there was no draft to send.",
}


@dataclass(frozen=True)
class ReviewResult:
    handled: bool = False
    reply: str = ""


class PrivacyService:
    def __init__(self, agent) -> None:
        self.agent = agent
        self.configure(agent.config.memory.privacy)

    def configure(self, config: PrivacyConfig) -> None:
        self.config = config
        self.policy = PrivacyPolicy(config)

    # ------------------------------------------------------------- helpers

    @property
    def _lang(self) -> str:
        return str(self.agent.soul.language)

    def _text(self, key: str, **kwargs) -> str:
        return t(f"privacy.{key}", self._lang, **kwargs)

    def _risky(self, hit: MemoryHit, *, direct: bool) -> bool:
        limit = self.config.dm_max_level if direct else self.config.group_max_level
        if hit.level > limit or hit.vetoed:
            return True
        return hit.level >= self.config.ask_level and not hit.confirmed

    # -------------------------------------------------------------- review

    async def review(
        self,
        inbound,
        reply: str,
        hits: Sequence[MemoryHit],
        gateway,
    ) -> ReviewResult:
        if not self.config.enabled or not hits:
            return ReviewResult(False, reply)
        direct = bool(inbound.hints.get("dm"))
        risky = [hit for hit in hits if self._risky(hit, direct=direct)]
        if not risky:
            return ReviewResult(False, reply)
        audit = self._audit(reply, hits, inbound) if self.config.auditor else AuditResult(
            verdict="ask"
        )
        # Only a hit the draft actually mentions can be disclosed: an unmentioned memory is
        # not a leak, and the old `or risky` fallback bypassed the calibrated policy by
        # letting a hit the draft never mentioned decide the review.
        selected = [hit for hit in hits if hit.id in audit.mentions]
        if not selected:
            return ReviewResult(False, reply)
        requester = self.agent._actor(inbound)
        verdicts = [
            self.policy.disclose(hit, requester=requester, direct=direct) for hit in selected
        ]
        self._record(inbound, selected, verdicts, reply=reply, policy=POLICY_VERSION)
        if "deny" in verdicts:
            return ReviewResult(False, audit.reply or self._text("keep"))
        if "ask" in verdicts:
            ids = sorted(hit.id for hit in selected)
            for row in self.agent.approvals.pending(inbound.thread, requester):
                # One question per memory set: a later draft replaces the stored reply instead of
                # staging a second row, so the sender never answers the same question twice.
                if row.kind == "disclosure" and sorted(row.args.get("memory_ids") or []) == ids:
                    payload = dict(row.payload)
                    payload["reply"] = reply
                    self.agent.approvals.set_payload(row.id, payload)
                    return ReviewResult(True, "")
            question = await self._phrase("disclosure", inbound=inbound)
            approval = await self.agent.approvals.create(
                kind="disclosure",
                thread_id=inbound.thread,
                sender=requester,
                gateway=getattr(gateway, "name", "") or "",
                tool="disclosure",
                args={"memory_ids": [hit.id for hit in selected]},
                payload={"reply": reply, "question": question},
                summary=question,
            )
            log.info("disclosure pending chat=%s approval=%s", inbound.thread, approval.id)
            await self.agent.send_text(
                inbound.thread,
                question,
                gateway=gateway,
                origin=Address(str(getattr(gateway, "name", "") or ""), str(inbound.thread)),
            )
            return ReviewResult(True, "")
        return ReviewResult(False, audit.reply or reply)

    def _record(
        self,
        inbound,
        selected: Sequence[MemoryHit],
        verdicts: Sequence[str],
        *,
        reply: str,
        policy: str,
    ) -> None:
        ledger = getattr(self.agent, "ledger", None)
        if ledger is None:
            return
        ids = ",".join(hit.id for hit in selected)
        levels = ",".join(str(hit.level) for hit in selected)
        digest = hashlib.sha256(str(reply or "").encode("utf-8")).hexdigest()[:16]
        ledger.log_action(
            "privacy",
            inbound.thread,
            "privacy",
            f"ids={ids} levels={levels} verdicts={','.join(verdicts)} "
            f"hash={digest} policy={policy}",
        )

    async def _phrase(self, intent: str, *, inbound=None, level: int | None = None) -> str:
        """One natural line for the privacy dialogue; the catalog line when the model fails.

        No memory text leaves the host: the prompt carries the intent and the sender's own last
        message, and the fact to convey is a code-owned sentence.
        """
        fallback = {
            "disclosure": self._text("ask_disclosure"),
            "level": self._text("ask_level"),
            "kept": self._text("keep"),
            "saved": self._text("saved", level=level),
            "empty": self._text("empty"),
        }[intent]
        llm = getattr(self.agent, "llm", None)
        if llm is None:
            return fallback
        recent = " ".join(
            str(getattr(getattr(inbound, "payload", None), "text", "") or "").split()
        )
        fact = PHRASE_FACTS[intent].format(level=level)
        try:
            raw = await llm.complete(
                PHRASE_PROMPT,
                f"Language: {self._lang}\nRecent message: {recent[:ANSWER_CHARS] or '-'}\n"
                f"Fact: {fact}",
                max_tokens=PHRASE_TOKENS,
                purpose="privacy_phrase",
            )
        except Exception:
            return fallback
        line = next(
            (item.strip() for item in str(raw or "").splitlines() if item.strip()), ""
        )
        return line[:PHRASE_CHARS] if line else fallback

    def _audit(self, reply: str, hits: Sequence[MemoryHit], inbound) -> AuditResult:
        """Local mention detection: no memory text leaves the host (ADR-0007 §4.5)."""
        words = set(re.findall(r"[\w]{4,}", str(reply or "").lower()))
        mentions: list[str] = []
        for hit in hits:
            tokens = set(re.findall(r"[\w]{4,}", str(hit.text or "").lower()))
            if tokens and len(tokens & words) >= min(2, len(tokens)):
                mentions.append(hit.id)
        return AuditResult(mentions=tuple(mentions), verdict="allow")

    # ------------------------------------------------------------- questions

    async def ask_dispute(self, subject_id: str, pair: dict) -> str:
        """ADR-0010 §4.7: one dispute question per subject per day, in that subject's DM.

        The target is a real DM thread (`threads.kind='dm'`, single member); when the body cannot
        prove one, or the daily budget is spent, nothing is sent and nothing is asked. The answer
        arrives as an ordinary turn, so no approval row is created here.
        """
        agent = self.agent
        subject = str(subject_id or "").strip()
        ids = [str(item) for item in (pair.get("ids") or [])]
        if not subject or not ids:
            return "skipped: no subject"
        budget = int(getattr(agent.config.reflection, "questions_per_day", 1))
        if budget <= 0:
            return "skipped: questions are off"
        thread = str(agent.private_thread(subject) or "")
        if not thread:
            log.info("dispute question skipped: no dm for %s", subject)
            return "skipped: no dm"
        if int(agent.memory.disputes_asked_today(subject)) >= budget:
            log.info("dispute question skipped: daily budget for %s", subject)
            return "skipped: budget"
        text = t(
            "memory.dispute_question",
            self._lang,
            first=str(pair.get("first") or "").strip()[:160],
            second=str(pair.get("second") or "").strip()[:160],
        )
        body = agent.body_of(thread)
        if body is None:
            log.info("dispute question skipped: no body owns %s", thread)
            return "skipped: no dm"
        sent = await agent.send_text(
            thread,
            text,
            gateway=body,
            subject=subject,
            origin=Address(str(getattr(body, "name", "") or ""), str(thread)),
        )
        if not str(sent).startswith("ok:"):
            return f"failed: {sent}"
        agent.memory.mark_dispute_asked(ids)
        log.info("dispute question asked thread=%s subject=%s", thread, subject)
        return "ok: asked"

    async def ask_memory(
        self,
        thread_id: str,
        person_id: str,
        memory_ids: list[str],
        level: int,
        question: str = "",
    ) -> None:
        """ADR-0004/0006: a DM confirmation for a memory the assess left unconfirmed."""
        agent = self.agent
        if not thread_id or not memory_ids:
            return
        body = agent.body_of(thread_id)
        if body is None:
            return
        wanted = sorted(str(item) for item in memory_ids)
        for row in agent.approvals.pending(thread_id, person_id):
            # The same memory set is already waiting for an answer: send no second question.
            if row.kind == "privacy" and sorted(
                str(item) for item in row.args.get("memory_ids") or []
            ) == wanted:
                return
        text = question or await self._phrase("level")
        approval = await agent.approvals.create(
            kind="privacy",
            thread_id=thread_id,
            sender=person_id,
            gateway=str(body.name or ""),
            tool="memory",
            args={"memory_ids": list(memory_ids)},
            payload={"level": int(level), "question": text},
            summary=text,
        )
        log.info("privacy pending thread=%s approval=%s", thread_id, approval.id)
        # ADR-0010 §4.1: the readable badge is the whole contract; a body renders its own
        # buttons from it, so this path carries no UI block and no transport vocabulary.
        badge = t("agent.approval_badge", self._lang, id=approval.id)
        await agent.send_text(
            thread_id,
            f"{text}\n\n{badge}",
            gateway=body,
            subject=person_id,
            origin=Address(str(getattr(body, "name", "") or ""), str(thread_id)),
        )

    # ------------------------------------------------------------- pending

    async def handle_pending(self, inbound, gateway) -> bool:
        """Consume this message only when it answers a pending privacy question.

        A level answer belongs to the level question, a yes/no to the newest question, and
        anything else is an ordinary turn: the question stays pending until the TTL, so a
        question never swallows the conversation that follows it.
        """
        sender = self.agent._actor(inbound)
        pendings = [
            row
            for row in self.agent.approvals.pending(inbound.thread, sender)
            if row.kind in KINDS
        ]
        if not pendings:
            return False
        # The newest question is the context the answer classifier judges against.
        newest = pendings[-1]
        question = str(newest.payload.get("question") or newest.summary or "")
        answer = await self._answer(inbound.payload.text, question=question)
        pending = self._target(pendings, answer)
        if pending is None:
            return False
        await self._decide_and_send(pending, inbound, gateway, sender, answer)
        return True

    def _target(self, pendings, answer: dict):
        """A level answer belongs to the level question; yes/no to the newest question."""
        if answer["kind"] == "level":
            found = [row for row in pendings if row.kind == "privacy"]
            return found[-1] if found else None
        if answer["kind"] in {"yes", "no"}:
            return pendings[-1]
        return None

    async def decide(
        self,
        approval,
        *,
        approved: bool,
        sender: str,
        level: int | None = None,
        inbound=None,
    ) -> str:
        """Apply one privacy/disclosure decision; returns the chat text, never sends.

        Shared by the chat answer, `/approve`, and `/deny`, so all three paths forget, confirm,
        and record the same way.
        """
        ids = [str(item) for item in approval.args.get("memory_ids") or []]
        if approval.kind == "privacy":
            if not approved:
                for memory_id in ids:
                    await self.agent.memory.forget(
                        memory_id, requester_id=sender, thread_id=approval.thread_id
                    )
                self.agent.approvals.mark(approval.id, "denied", sender, "privacy declined")
                return await self._phrase("kept", inbound=inbound)
            chosen = level if level is not None else approval.payload.get("level")
            chosen = clamp_level(
                int(chosen or self.config.default_level), self.config.default_level
            )
            for memory_id in ids:
                await self.agent.memory.set_privacy(memory_id, level=chosen, confirmed=True)
            self.agent.approvals.mark(approval.id, "approved", sender, f"level {chosen}")
            return await self._phrase("saved", inbound=inbound, level=chosen)
        if not approved:
            self.agent.approvals.mark(approval.id, "denied", sender, "disclosure declined")
            return await self._phrase("kept", inbound=inbound)
        self.agent.approvals.mark(approval.id, "approved", sender, "disclosure allowed")
        reply = str(approval.payload.get("reply") or "")
        return reply if reply.strip() else await self._phrase("empty", inbound=inbound)

    async def _decide_and_send(self, pending, inbound, gateway, sender: str, answer: dict) -> None:
        text = await self.decide(
            pending,
            approved=answer["kind"] in {"yes", "level"},
            sender=sender,
            level=answer.get("level"),
            inbound=inbound,
        )
        await self.agent.send_text(
            inbound.thread,
            text,
            gateway=gateway,
            origin=Address(str(getattr(gateway, "name", "") or ""), str(inbound.thread)),
        )

    async def _answer(self, text: str, *, question: str = "") -> dict:
        """Deterministic first, model only when the fast path cannot decide."""
        answer = self._regex_answer(str(text or ""))
        if answer["kind"] != "unclear":
            return answer
        return await self._llm_answer(text, question=question)

    async def _llm_answer(self, text: str, *, question: str = "") -> dict:
        """One classifier call over the reply: any language, one token back."""
        llm = getattr(self.agent, "llm", None)
        body = " ".join(str(text or "").split())[:ANSWER_CHARS]
        asked = " ".join(str(question or "").split())[:ANSWER_CHARS]
        if llm is None or not body:
            return dict(UNCLEAR)
        try:
            raw = await llm.complete(
                ANSWER_PROMPT,
                f"Language: {self._lang}\nQuestion asked: {asked or '-'}\nReply:\n{body}",
                purpose="privacy_answer",
                **cap_kwargs(llm),
            )
        except Exception:
            return dict(UNCLEAR)
        answer = str(raw or "")
        words = set(re.findall(r"[A-Z]+", answer.upper()))
        digits = [int(ch) for ch in answer if ch.isdigit() and int(ch) <= 4]
        if "LEVEL" in words and digits:
            return {"kind": "level", "level": digits[0]}
        if not words and digits:
            return {"kind": "level", "level": digits[0]}
        # Both signals at once ("no problem, level 2") is ambiguous: stay unclear, so the
        # question keeps waiting instead of forgetting a memory on a misread word.
        if "YES" in words and "NO" not in words:
            return {"kind": "yes", "level": None}
        if "NO" in words and "YES" not in words:
            return {"kind": "no", "level": None}
        return dict(UNCLEAR)

    def _regex_answer(self, text: str) -> dict:
        """The fast path: a bare level, or an English affirmation/negation and nothing else."""
        body = " ".join(str(text or "").split())
        words = set(re.findall(r"[a-z]+", body.lower()))
        pack = lexicon.load(self._lang)
        if len(body.split()) <= LEVEL_WORDS:
            for word, level in pack.privacy_levels.items():
                if word in words:
                    return {"kind": "level", "level": level}
            digits = [int(ch) for ch in body if ch.isdigit() and int(ch) <= 4]
            if digits:
                return {"kind": "level", "level": digits[0]}
        if words and words <= pack.affirmations:
            return {"kind": "yes", "level": None}
        if words and words <= pack.negations:
            return {"kind": "no", "level": None}
        return dict(UNCLEAR)

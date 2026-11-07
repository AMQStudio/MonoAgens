"""Measure the completion budget of the one-answer classifier calls.

`engine.llm.classifier_max_tokens` is not a guess: a reasoning model may spend the whole budget
on a preamble and return an empty string, and how much it spends differs per model. This script
sends the two real classifier prompts (claim guard, privacy answer) to the configured model,
reads the `tokens_out` the client recorded in `llm_calls`, and prints the distribution plus the
recommended cap.

Run it from the repo root, then copy the numbers into `config.yaml`::

    python -P scripts/token_probe.py
    python -P scripts/token_probe.py --model other/model --cap 96

Exit codes: 0 measured, 1 a reply came back empty (the running cap is too small), 2 no key.
"""


import argparse
import asyncio
import math
import os
import statistics
import sys
from pathlib import Path

from sqlalchemy import text

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from monoagens.engine import claims
from monoagens.engine.access.privacy import ANSWER_PROMPT
from monoagens.engine.access.roles import Role, RoleRegistry
from monoagens.engine.config import (
    ApprovalConfig,
    Config,
    DatabaseConfig,
    LLMConfig,
    Paths,
    SandboxConfig,
)
from monoagens.engine.config.environment import load_env
from monoagens.engine.db import Database
from monoagens.engine.llm import LLM
from monoagens.engine.soul import Soul

DEFAULT_PROVIDER = "deepinfra"
DEFAULT_BASE_URL = "https://api.deepinfra.com/v1/openai"
DEFAULT_MODEL = "deepseek-ai/DeepSeek-V4.1-Flash-1017"
DEFAULT_KEY_ENV = "TEST_DEEPINFRA"
DEFAULT_TIER = "flex"
SAMPLE_LIMIT = 15
RECOMMEND_FLOOR = 64
RECOMMEND_STEP = 16
PURPOSES = ("claim_guard", "privacy_answer")

# Claim-guard samples: one request and one draft each, short to long, two languages.
JUDGE_SAMPLES: tuple[tuple[str, str], ...] = (
    ("apa isi arsip grup itu?", "Aku tidak bisa membaca arsip grup itu."),
    ("baca telegram dong", "Maaf, aku tidak punya akses ke Telegram."),
    ("kamu datang rapat besok?", "Aku tidak bisa hadir rapat besok, ada jadwal lain."),
    ("siapa saja anggota grup itu?", "Kamu bukan anggota grup itu."),
    ("ringkas arsip ya", "Ini ringkasannya: rapat jam tiga, dua keputusan, satu tindak lanjut."),
    ("can you read the archive?", "I cannot access that archive from here."),
    ("is Budi in the room?", "Budi is not a member of that room."),
    ("kirim pesan ke grup sebelah", "Aku tidak bisa mengirim pesan ke grup itu."),
    ("apa kabar?", "Baik! Kamu gimana?"),
    ("tolong lupakan yang tadi", "Oke, aku lupakan."),
    (
        "apa isi arsip grup itu?",
        "Aku tidak bisa membaca arsip grup itu karena thread ini cuma punya akses ke chat ini, "
        "jadi semua riwayat grup lain tidak terlihat olehku dan kamu harus membacanya sendiri.",
    ),
    (
        "kasih ringkasan rapat minggu lalu",
        "Rapat minggu lalu membahas anggaran, dua vendor, dan jadwal rilis; keputusan akhirnya "
        "menunda rilis satu minggu dan meminta vendor kedua mengirim penawaran ulang.",
    ),
    ("kamu bisa baca chat ini kan?", "Bisa, ini chat kita."),
    ("apakah arsipnya ada?", "Arsipnya tidak ada di sini."),
    ("please check the group", "I can read this chat only; ask me about it."),
)

# Privacy-answer samples: the sender's reply to a yes/no or level question.
ANSWER_SAMPLES: tuple[str, ...] = (
    "ya",
    "tidak",
    "2",
    "rahasia",
    "level 3",
    "boleh, sebutkan saja",
    "jangan, simpan sendiri",
    "hmm gimana ya",
    "yes",
    "no",
    "sensitive",
    "aku punya 2 anak, jadi jangan sebut yang itu",
    "oke boleh sih, tapi jangan detail",
    "Simpan dulu ya, nanti aku kabari kalau sudah siap dibahas di grup.",
    "ya, silakan sebut di sini",
)


def config_for(tmp: Path, args) -> Config:
    """A minimal host config: the probe needs an LLM, a database, and nothing else."""
    workspace = tmp / "workspace"
    return Config(
        paths=Paths(
            config=tmp / "config.yaml",
            roles=tmp / "roles.yaml",
            soul_yaml=tmp / "soul.yaml",
            workspace=workspace,
        ),
        database=DatabaseConfig(url=args.dsn, url_env=""),
        llm=LLMConfig(
            provider=args.provider,
            base_url=args.base_url,
            model=args.model,
            api_key_env=args.api_key_env,
            service_tier=args.service_tier,
            classifier_max_tokens=args.cap,
        ),
        workspace=workspace,
        approval=ApprovalConfig(),
        roles=RoleRegistry(by_name={"member": Role("member")}, default="member"),
        default_role="member",
        gateways={},
        soul=Soul(name="Probe", language="id", locale="id-ID", timezone="UTC"),
        sandbox=SandboxConfig(),
        mcp={},
    )


def calls() -> list[tuple[str, str, str]]:
    """`(purpose, system, user)` for both classifier prompts, one sample per entry."""
    found = [
        ("claim_guard", claims.JUDGE, f"Language: id\nSender asked: {request}\nDraft:\n{draft}")
        for request, draft in JUDGE_SAMPLES
    ]
    found += [
        ("privacy_answer", ANSWER_PROMPT, f"Language: id\nReply:\n{reply}")
        for reply in ANSWER_SAMPLES
    ]
    return found


async def measure(llm: LLM, *, cap: int) -> list[tuple[str, str]]:
    """Send every sample once; `(purpose, raw answer)` back, in order."""
    extra: dict = {"max_tokens": cap} if cap > 0 else {}
    out: list[tuple[str, str]] = []
    for purpose, system, user in calls():
        raw = await llm.complete(system, user, purpose=purpose, **extra)
        out.append((purpose, str(raw or "")))
    return out


def distribution(db: Database) -> list[int]:
    """The recorded completion tokens of the classifier calls, newest first."""
    rows = db.fetch_all(
        text(
            "SELECT tokens_out FROM llm_calls "
            "WHERE purpose IN ('claim_guard', 'privacy_answer') AND tokens_out IS NOT NULL "
            "ORDER BY id DESC LIMIT 60"
        )
    )
    return [int(row["tokens_out"]) for row in rows]


def percentile(values: list[int], fraction: float) -> int:
    """Nearest-rank percentile: no interpolation, so the number is one a call really used."""
    ordered = sorted(values)
    index = max(0, math.ceil(fraction * len(ordered)) - 1)
    return ordered[min(index, len(ordered) - 1)]


def recommend(values: list[int]) -> int:
    """Twice the p95, rounded up to the step, never below the floor."""
    p95 = percentile(values, 0.95)
    return max(RECOMMEND_FLOOR, math.ceil(p95 * 2 / RECOMMEND_STEP) * RECOMMEND_STEP)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Measure the classifier completion budget")
    parser.add_argument("--provider", default=DEFAULT_PROVIDER)
    parser.add_argument("--base-url", default=DEFAULT_BASE_URL)
    parser.add_argument("--model", default=DEFAULT_MODEL)
    parser.add_argument("--api-key-env", default=DEFAULT_KEY_ENV)
    parser.add_argument("--service-tier", default=DEFAULT_TIER)
    parser.add_argument("--cap", type=int, default=0, help="cap to probe with; 0 = no cap sent")
    parser.add_argument(
        "--dsn",
        default=os.environ.get("MONOAGENS_TEST_DATABASE_URL")
        or os.environ.get("MONOAGENS_DATABASE_URL", ""),
        help="database that records llm_calls; defaults to the test database",
    )
    args = parser.parse_args(argv)
    load_env(".env", root=ROOT)
    if not os.environ.get(args.api_key_env):
        print(f"SKIP: {args.api_key_env} is not set")
        return 2
    if not args.dsn:
        print("SKIP: set MONOAGENS_TEST_DATABASE_URL or pass --dsn")
        return 2
    tmp = ROOT / ".monoagens" / "probe"
    tmp.mkdir(parents=True, exist_ok=True)
    config = config_for(tmp, args)
    db = Database(args.dsn)
    llm = LLM(config, db=db)
    answers = asyncio.run(measure(llm, cap=max(0, args.cap)))
    empty = [purpose for purpose, raw in answers if not raw.strip()]
    values = distribution(db)
    print(f"model={args.model} cap={args.cap or 'client default'} samples={len(answers)}")
    if not values:
        print("no llm_calls row carried tokens_out; the provider returned no usage")
        return 1
    print(
        f"tokens_out: n={len(values)} p50={percentile(values, 0.5)} "
        f"p95={percentile(values, 0.95)} max={max(values)} "
        f"mean={statistics.fmean(values):.1f}"
    )
    print(f"recommended cap: {recommend(values)}")
    if empty:
        print(f"empty answers: {len(empty)} ({', '.join(empty)})")
        return 1
    print("empty answers: 0")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

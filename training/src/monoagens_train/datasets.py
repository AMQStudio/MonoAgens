"""OSI-verified Phase 1 dataset ingestion, anti-leakage splits, and EmbeddingGemma 2 shard caching."""

from __future__ import annotations

import hashlib
import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np

from monoagens_train.features import NUM_FEATURES, PerceptNumericInputs, build_x_num
from monoagens_train.licenses import assert_safe_path, require_dataset_allowed

PHASE1_DATASETS = ("when2speak", "ubuntu_irc", "proactive_bench", "capa")
PHASE2_DATASETS = ("longmemeval", "stale", "hi_tom", "dyntom")
PHASE3_DATASETS = ("group_mem_bench", "memory_agent_bench")
GATE_SILENT = 0
GATE_NOTE = 1
GATE_WAIT = 2
GATE_GO = 3


def _splitmix64(x: int) -> int:
    x = (x + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
    z = x
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
    return z ^ (z >> 31)


class RustSplitMix64:
    """Exact mirror of `SplitMix64` in `crates/lab/src/datasets/framework.rs`."""

    def __init__(self, seed: int) -> None:
        self.state = seed & 0xFFFFFFFFFFFFFFFF

    def next_u64(self) -> int:
        self.state = (self.state + 0x9E3779B97F4A7C15) & 0xFFFFFFFFFFFFFFFF
        z = self.state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & 0xFFFFFFFFFFFFFFFF
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & 0xFFFFFFFFFFFFFFFF
        return z ^ (z >> 31)

    def shuffle(self, arr: list[Any]) -> None:
        for i in range(len(arr) - 1, 0, -1):
            j = self.next_u64() % (i + 1)
            arr[i], arr[j] = arr[j], arr[i]


def _when2speak_lab_eval_ids(rows_meta: list[tuple[str, str]], seeds: tuple[int, ...] = (0, 1, 42, 2026), target_n: int = 1000) -> set[str]:
    """Reproduces Lab's stratified S=1000 sampling to guarantee zero overlap with Lab eval."""
    excluded: set[str] = set()
    total = len(rows_meta)
    if total <= target_n:
        return excluded
    for seed in seeds:
        by_stratum: dict[str, list[int]] = {}
        for idx, (_item_id, strata_key) in enumerate(rows_meta):
            by_stratum.setdefault(strata_key, []).append(idx)
        rng = RustSplitMix64(seed)
        for key in sorted(by_stratum):
            rng.shuffle(by_stratum[key])
        remainders: list[tuple[str, int, int]] = []
        chosen: list[int] = []
        allocated = 0
        for key in sorted(by_stratum):
            idxs = by_stratum[key]
            num = len(idxs) * target_n
            q = num // total
            rem = num % total
            take = min(q, len(idxs))
            chosen.extend(idxs[:take])
            allocated += take
            remainders.append((key, take, rem))
        remainders.sort(key=lambda t: (-t[2], t[0]))
        for key, taken, _rem in remainders:
            if allocated >= target_n:
                break
            idxs = by_stratum[key]
            if taken < len(idxs):
                chosen.append(idxs[taken])
                allocated += 1
        for idx in chosen:
            excluded.add(rows_meta[idx][0])
    return excluded


def _stable_split_bucket(cluster_id: str) -> str:
    """Deterministic 80/10/10 conversation-level split via SHA-256 hash."""
    h = int.from_bytes(hashlib.sha256(cluster_id.encode("utf-8")).digest()[:8], "big") % 100
    if h < 80:
        return "train"
    if h < 90:
        return "calibration"
    return "test"


@dataclass
class TurnRecord:
    text: str
    x_num: np.ndarray
    label_gate: int
    wait_delta_ms: float
    loss_mask: float
    y_uncertainty: float = 0.0
    mask_uncertainty: float = 0.0
    y_verify: float = 0.0
    mask_verify: float = 0.0
    y_encode_importance: float = 0.0
    mask_encode_importance: float = 0.0
    y_memory_relevance: float = 0.0
    mask_memory_relevance: float = 0.0
    pair_query_text: str | None = None


@dataclass
class TrajectoryEpisode:
    episode_id: str
    dataset_id: str
    cluster_id: str
    split: str
    turns: list[TurnRecord]


def _load_when2speak(
    dataset_dir: Path,
    max_windows: int = 4500,
) -> list[TrajectoryEpisode]:
    jsonl_path = dataset_dir / "finetune_test_token.jsonl"
    raw_rows: list[tuple[str, str, list[dict[str, Any]], bool]] = []
    rows_meta: list[tuple[str, str]] = []

    with jsonl_path.open("r", encoding="utf-8") as f:
        for idx, line in enumerate(f):
            line = line.strip()
            if not line:
                continue
            row = json.loads(line)
            msgs = row.get("messages", [])
            if not msgs:
                continue
            item_id = row.get("window_id") or f"w2s_{idx:05d}"
            last_msg = msgs[-1]
            if last_msg.get("role") == "assistant":
                lbl = str(last_msg.get("content", ">")).strip()
                prefix = msgs[:-1]
            else:
                lbl = ">"
                prefix = msgs
            if not prefix:
                continue
            is_speak = lbl.startswith("<")
            speakers = {
                m.get("speaker") or f"Speaker_{t % 4}"
                for t, m in enumerate(prefix)
            }
            strata_key = f"{'SPEAK' if is_speak else 'SILENT'}:spk={len(speakers)}"
            rows_meta.append((item_id, strata_key))
            raw_rows.append((item_id, strata_key, prefix, is_speak))

    excluded_ids = _when2speak_lab_eval_ids(rows_meta)

    episodes: list[TrajectoryEpisode] = []
    # Deterministically subsample from non-Lab windows for fast local training while keeping balance.
    for item_id, _strata, prefix, is_speak in raw_rows:
        if item_id in excluded_ids:
            continue
        # Stable deterministic subsample keeping ~max_windows windows across splits
        h = int.from_bytes(hashlib.sha256(item_id.encode("utf-8")).digest()[:4], "big")
        if len(raw_rows) > max_windows and (h % len(raw_rows)) >= max_windows:
            continue

        speakers_seen: dict[str, int] = {}
        speaker_last_t: dict[str, int] = {}
        turns: list[TurnRecord] = []
        unique_speakers = {
            m.get("speaker") or f"Speaker_{t % 4}" for t, m in enumerate(prefix)
        }
        is_dyadic = len(unique_speakers) <= 2

        for t, m in enumerate(prefix):
            spk = m.get("speaker") or f"Speaker_{t % 4}"
            prev_t = speaker_last_t.get(spk)
            dt_sender = float((t - prev_t) * 8) if prev_t is not None else 86400.0
            spk_count = speakers_seen.get(spk, 0)
            speakers_seen[spk] = spk_count + 1
            speaker_last_t[spk] = t

            is_final = t == len(prefix) - 1
            x_num = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=False,
                    is_explicit_addressee=False,
                    is_reply_or_expect_link=(prev_t is not None),
                    is_dyadic_place=is_dyadic,
                    active_participant_count=len(unique_speakers) + 1,
                    dt_since_last_place_event_s=8.0 if t > 0 else 0.0,
                    dt_since_last_soul_act_s=86400.0,
                    dt_since_sender_last_act_s=dt_sender,
                    soul_acts_window=0,
                    total_acts_window=t + 1,
                    sender_exchange_count=spk_count,
                    place_event_count=t,
                )
            )
            label = (GATE_GO if is_speak else GATE_SILENT) if is_final else GATE_SILENT
            turns.append(
                TurnRecord(
                    text=str(m.get("content", "")),
                    x_num=x_num,
                    label_gate=label,
                    wait_delta_ms=0.0,
                    loss_mask=1.0 if is_final else 0.0,
                )
            )

        episodes.append(
            TrajectoryEpisode(
                episode_id=f"w2s:{item_id}",
                dataset_id="when2speak",
                cluster_id=f"w2s:{item_id}",
                split=_stable_split_bucket(f"w2s:{item_id}"),
                turns=turns,
            )
        )
    return episodes


def _parse_irc_line(line: str, prev_min: int, day_offset: int) -> tuple[tuple[int, str, str] | None, int, int]:
    s = line.strip()
    if not s or not s.startswith("["):
        return None, prev_min, day_offset
    end_b = s.find("]")
    if end_b < 0:
        return None, prev_min, day_offset
    hhmm = s[1:end_b]
    if ":" not in hhmm:
        return None, prev_min, day_offset
    try:
        hh_s, mm_s = hhmm.split(":", 1)
        min_of_day = int(hh_s) * 60 + int(mm_s)
    except ValueError:
        return None, prev_min, day_offset
    if min_of_day < prev_min:
        day_offset += 86400
    prev_min = min_of_day
    ts_s = day_offset + min_of_day * 60

    rest = s[end_b + 1 :].strip()
    if rest.startswith("<") and ">" in rest:
        gt = rest.find(">")
        speaker = rest[1:gt].strip()
        text = rest[gt + 1 :].strip()
    else:
        speaker = "irc_system"
        text = rest
    return (ts_s, speaker, text), prev_min, day_offset


def _load_ubuntu_irc(dataset_dir: Path, window_len: int = 16) -> list[TrajectoryEpisode]:
    test_dir = dataset_dir / "data" / "test"
    raw_files = sorted(p for p in test_dir.iterdir() if p.name.endswith(".raw.txt"))
    # Strict file-level split: 6 train, 2 calibration, 2 test
    episodes: list[TrajectoryEpisode] = []
    for file_idx, raw_path in enumerate(raw_files):
        stem = raw_path.name[: -len(".raw.txt")]
        if file_idx < 6:
            split = "train"
        elif file_idx < 8:
            split = "calibration"
        else:
            split = "test"

        ann_path = test_dir / f"{stem}.annotation.txt"
        raw_lines = raw_path.read_text(encoding="utf-8", errors="replace").splitlines()
        parsed: list[tuple[int, str, str]] = []
        raw_to_parsed: list[int | None] = []
        prev_min = 0
        day_offset = 0
        for line in raw_lines:
            item, prev_min, day_offset = _parse_irc_line(line, prev_min, day_offset)
            raw_to_parsed.append(len(parsed) if item is not None else None)
            if item is not None:
                parsed.append(item)

        if len(parsed) < 4:
            continue

        parent_of: dict[int, int] = {}
        if ann_path.is_file():
            for line in ann_path.read_text(encoding="utf-8", errors="replace").splitlines():
                parts = line.split()
                if len(parts) >= 2 and parts[0].isdigit() and parts[1].isdigit():
                    c_raw, p_raw = int(parts[0]), int(parts[1])
                    if c_raw < len(raw_to_parsed) and p_raw < len(raw_to_parsed):
                        c_idx, p_idx = raw_to_parsed[c_raw], raw_to_parsed[p_raw]
                        if c_idx is not None and p_idx is not None and c_idx != p_idx:
                            parent_of[c_idx] = p_idx

        # Designate the 2nd most frequent non-system speaker as the observed soul participant
        counts: dict[str, int] = {}
        for _ts, spk, _txt in parsed:
            if spk != "irc_system":
                counts[spk] = counts.get(spk, 0) + 1
        ranked = sorted(counts.items(), key=lambda kv: (-kv[1], kv[0]))
        if not ranked:
            continue
        soul_spk = ranked[min(1, len(ranked) - 1)][0]

        # Build sequential turns and chunk into multi-turn windows
        speaker_counts: dict[str, int] = {}
        speaker_last_ts: dict[str, int] = {}
        soul_msg_indices: set[int] = {i for i, (_ts, spk, _txt) in enumerate(parsed) if spk == soul_spk}
        last_soul_ts: int | None = None
        soul_acts = 0

        seq_turns: list[TurnRecord] = []
        for idx, (ts_s, spk, text) in enumerate(parsed):
            if spk == soul_spk:
                last_soul_ts = ts_s
                soul_acts += 1
                speaker_counts[spk] = speaker_counts.get(spk, 0) + 1
                speaker_last_ts[spk] = ts_s
                continue

            prev_place_ts = parsed[idx - 1][0] if idx > 0 else ts_s
            dt_place = float(max(0, ts_s - prev_place_ts))
            dt_soul = float(max(0, ts_s - last_soul_ts)) if last_soul_ts is not None else 86400.0
            prev_spk_ts = speaker_last_ts.get(spk)
            dt_sender = float(max(0, ts_s - prev_spk_ts)) if prev_spk_ts is not None else 86400.0

            p_idx = parent_of.get(idx)
            is_reply_to_soul = p_idx in soul_msg_indices
            is_named = bool(re.search(rf"\b{re.escape(soul_spk)}\b", text)) if len(soul_spk) > 2 else False

            # Lookahead within 3 steps for soul response to label Go / Wait / Note / Silent
            next_soul_offset: int | None = None
            next_soul_dt_ms = 0.0
            for look in range(1, min(4, len(parsed) - idx)):
                if parsed[idx + look][1] == soul_spk:
                    next_soul_offset = look
                    next_soul_dt_ms = float(max(0, parsed[idx + look][0] - ts_s) * 1000)
                    break

            if next_soul_offset == 1:
                label = GATE_GO
                wait_ms = 0.0
            elif next_soul_offset in (2, 3):
                label = GATE_WAIT
                wait_ms = min(next_soul_dt_ms, 30000.0)
            elif is_reply_to_soul or is_named:
                label = GATE_NOTE
                wait_ms = 0.0
            else:
                label = GATE_SILENT
                wait_ms = 0.0

            x_num = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=is_named,
                    is_explicit_addressee=is_named,
                    is_reply_or_expect_link=is_reply_to_soul,
                    is_dyadic_place=False,
                    active_participant_count=min(64, len(speaker_counts) + 1),
                    dt_since_last_place_event_s=dt_place,
                    dt_since_last_soul_act_s=dt_soul,
                    dt_since_sender_last_act_s=dt_sender,
                    soul_acts_window=soul_acts,
                    total_acts_window=idx + 1,
                    sender_exchange_count=speaker_counts.get(spk, 0),
                    place_event_count=idx,
                )
            )
            speaker_counts[spk] = speaker_counts.get(spk, 0) + 1
            speaker_last_ts[spk] = ts_s
            seq_turns.append(
                TurnRecord(
                    text=text,
                    x_num=x_num,
                    label_gate=label,
                    wait_delta_ms=wait_ms,
                    loss_mask=1.0,
                )
            )

        for chunk_idx, start in enumerate(range(0, len(seq_turns), window_len)):
            chunk = seq_turns[start : start + window_len]
            if len(chunk) < 4:
                continue
            episodes.append(
                TrajectoryEpisode(
                    episode_id=f"irc:{stem}:{chunk_idx:03d}",
                    dataset_id="ubuntu_irc",
                    cluster_id=f"irc:{stem}",
                    split=split,
                    turns=chunk,
                )
            )
    return episodes


def _load_proactive_bench(dataset_dir: Path) -> list[TrajectoryEpisode]:
    test_dir = dataset_dir / "dataset" / "test_data"
    files = sorted(
        p for p in test_dir.iterdir() if p.name.endswith(".json") and p.name != "splits.json"
    )
    episodes: list[TrajectoryEpisode] = []
    for fpath in files:
        stem = fpath.stem
        if stem.endswith("_15"):
            split = "calibration"
        elif stem.endswith("_16"):
            split = "test"
        else:
            split = "train"

        events = json.loads(fpath.read_text(encoding="utf-8"))
        turns: list[TurnRecord] = []
        prev_ts: float | None = None
        last_soul_ts: float | None = None
        soul_acts = 0

        for idx, ev in enumerate(events):
            obs = ev.get("observation", {})
            try:
                ts_s = float(obs.get("time", idx * 5.0))
            except (TypeError, ValueError):
                ts_s = float(idx * 5.0)
            dt_place = max(0.0, ts_s - prev_ts) if prev_ts is not None else 0.0
            dt_soul = max(0.0, ts_s - last_soul_ts) if last_soul_ts is not None else 86400.0
            prev_ts = ts_s

            task_status = bool(ev.get("task_status", False))
            next_is_true = idx + 1 < len(events) and bool(events[idx + 1].get("task_status", False))

            if task_status:
                label = GATE_GO
                wait_ms = 0.0
                last_soul_ts = ts_s
                soul_acts += 1
            elif next_is_true:
                label = GATE_NOTE
                wait_ms = 0.0
            else:
                label = GATE_SILENT
                wait_ms = 0.0

            x_num = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=False,
                    is_explicit_addressee=False,
                    is_reply_or_expect_link=False,
                    is_dyadic_place=True,
                    active_participant_count=2,
                    dt_since_last_place_event_s=dt_place,
                    dt_since_last_soul_act_s=dt_soul,
                    dt_since_sender_last_act_s=dt_place,
                    soul_acts_window=soul_acts,
                    total_acts_window=idx + 1,
                    sender_exchange_count=idx,
                    place_event_count=idx,
                )
            )
            turns.append(
                TurnRecord(
                    text=str(obs.get("event", "")),
                    x_num=x_num,
                    label_gate=label,
                    wait_delta_ms=wait_ms,
                    loss_mask=1.0,
                )
            )

        if turns:
            episodes.append(
                TrajectoryEpisode(
                    episode_id=f"pb:{stem}",
                    dataset_id="proactive_bench",
                    cluster_id=f"pb:{stem}",
                    split=split,
                    turns=turns,
                )
            )
    return episodes


def _load_capa(dataset_dir: Path, window_len: int = 16) -> list[TrajectoryEpisode]:
    test_dir = dataset_dir / "data" / "ami" / "test"
    files = sorted(p for p in test_dir.iterdir() if p.name.endswith("_parsed.json"))
    episodes: list[TrajectoryEpisode] = []
    for idx_f, fpath in enumerate(files):
        stem = fpath.stem
        if idx_f < 14:
            split = "train"
        elif idx_f < 16:
            split = "calibration"
        else:
            split = "test"

        utterances = json.loads(fpath.read_text(encoding="utf-8"))
        if len(utterances) < 4:
            continue
        soul_role = "Project Manager"
        speakers_seen: dict[str, int] = {}
        speaker_last_idx: dict[str, int] = {}
        last_soul_idx: int | None = None
        soul_acts = 0

        seq_turns: list[TurnRecord] = []
        for idx, utt in enumerate(utterances):
            spk = str(utt.get("speaker_id", "Unknown"))
            text = str(utt.get("utterance_content", ""))
            if spk == soul_role:
                last_soul_idx = idx
                soul_acts += 1
                speakers_seen[spk] = speakers_seen.get(spk, 0) + 1
                speaker_last_idx[spk] = idx
                continue

            next_soul_offset: int | None = None
            for look in range(1, min(4, len(utterances) - idx)):
                if utterances[idx + look].get("speaker_id") == soul_role:
                    next_soul_offset = look
                    break

            if next_soul_offset == 1:
                label = GATE_GO
                wait_ms = 0.0
            elif next_soul_offset in (2, 3):
                label = GATE_WAIT
                wait_ms = float(next_soul_offset * 3000.0)
            elif last_soul_idx is not None and idx - last_soul_idx == 1:
                label = GATE_NOTE
                wait_ms = 0.0
            else:
                label = GATE_SILENT
                wait_ms = 0.0

            dt_soul = float((idx - last_soul_idx) * 4) if last_soul_idx is not None else 86400.0
            prev_spk = speaker_last_idx.get(spk)
            dt_sender = float((idx - prev_spk) * 4) if prev_spk is not None else 86400.0

            x_num = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=False,
                    is_explicit_addressee=False,
                    is_reply_or_expect_link=(last_soul_idx is not None and idx - last_soul_idx <= 2),
                    is_dyadic_place=False,
                    active_participant_count=4,
                    dt_since_last_place_event_s=4.0 if idx > 0 else 0.0,
                    dt_since_last_soul_act_s=dt_soul,
                    dt_since_sender_last_act_s=dt_sender,
                    soul_acts_window=soul_acts,
                    total_acts_window=idx + 1,
                    sender_exchange_count=speakers_seen.get(spk, 0),
                    place_event_count=idx,
                )
            )
            speakers_seen[spk] = speakers_seen.get(spk, 0) + 1
            speaker_last_idx[spk] = idx
            seq_turns.append(
                TurnRecord(
                    text=text,
                    x_num=x_num,
                    label_gate=label,
                    wait_delta_ms=wait_ms,
                    loss_mask=1.0,
                )
            )

        for chunk_idx, start in enumerate(range(0, len(seq_turns), window_len)):
            chunk = seq_turns[start : start + window_len]
            if len(chunk) < 4:
                continue
            episodes.append(
                TrajectoryEpisode(
                    episode_id=f"capa:{stem}:{chunk_idx:03d}",
                    dataset_id="capa",
                    cluster_id=f"capa:{stem}",
                    split=split,
                    turns=chunk,
                )
            )
    return episodes


def build_phase1_episodes(repo_root: Path, max_w2s_windows: int = 4500) -> list[TrajectoryEpisode]:
    """Loads all 4 OSI-verified Phase 1 datasets with strict Lab anti-leakage splits."""
    manifest_path = assert_safe_path(repo_root / "lab" / "datasets.manifest.toml", repo_root=repo_root)
    episodes: list[TrajectoryEpisode] = []

    for ds_id in PHASE1_DATASETS:
        entry = require_dataset_allowed(manifest_path, ds_id)
        ds_dir = assert_safe_path(repo_root / "lab" / "datasets" / entry.relative_dir, repo_root=repo_root)
        if ds_id == "when2speak":
            episodes.extend(_load_when2speak(ds_dir, max_windows=max_w2s_windows))
        elif ds_id == "ubuntu_irc":
            episodes.extend(_load_ubuntu_irc(ds_dir))
        elif ds_id == "proactive_bench":
            episodes.extend(_load_proactive_bench(ds_dir))
        elif ds_id == "capa":
            episodes.extend(_load_capa(ds_dir))

    return episodes


def precompute_phase1_shards(
    repo_root: Path,
    cache_path: Path,
    embedder_dir: Path | None = None,
    device: str = "cuda",
    max_seq_len: int = 24,
    batch_size: int = 32,
    max_w2s_windows: int = 4500,
    force_recompute: bool = False,
) -> Path:
    """Encodes all episode turn texts via local EmbeddingGemma 2 (256d) and caches padded shards."""
    if cache_path.is_file() and not force_recompute:
        return cache_path

    if embedder_dir is None:
        embedder_dir = repo_root / "lab" / "datasets" / "embedder_models" / "embeddinggemma-2"
    assert_safe_path(embedder_dir, repo_root=repo_root)
    assert_safe_path(cache_path, repo_root=repo_root)

    episodes = build_phase1_episodes(repo_root, max_w2s_windows=max_w2s_windows)
    unique_texts: dict[str, int] = {}
    for ep in episodes:
        for t in ep.turns[:max_seq_len]:
            if t.text not in unique_texts:
                unique_texts[t.text] = len(unique_texts)

    ordered_texts = [""] * len(unique_texts)
    for txt, idx in unique_texts.items():
        ordered_texts[idx] = txt

    import gc
    import torch
    from sentence_transformers import SentenceTransformer

    use_bf16 = device.startswith("cuda") and torch.cuda.is_available()
    model_kwargs = {"dtype": torch.bfloat16} if use_bf16 else {}
    st_model = SentenceTransformer(str(embedder_dir), device=device, model_kwargs=model_kwargs)
    st_model.max_seq_length = 512
    text_inputs = [{"text": s} for s in ordered_texts]
    emb_matrix = st_model.encode(
        text_inputs,
        batch_size=batch_size,
        truncate_dim=256,
        normalize_embeddings=True,
        convert_to_numpy=True,
        show_progress_bar=False,
    ).astype(np.float32)

    del st_model
    gc.collect()
    if use_bf16:
        torch.cuda.empty_cache()

    if not np.all(np.isfinite(emb_matrix)):
        raise ValueError("Non-finite values produced by EmbeddingGemma 2 encoder")

    split_buckets: dict[str, list[TrajectoryEpisode]] = {"train": [], "calibration": [], "test": []}
    for ep in episodes:
        split_buckets[ep.split].append(ep)

    arrays: dict[str, np.ndarray] = {}
    for split_name, eps in split_buckets.items():
        n = len(eps)
        e_arr = np.zeros((n, max_seq_len, 256), dtype=np.float32)
        x_arr = np.zeros((n, max_seq_len, NUM_FEATURES), dtype=np.float32)
        y_gate = np.zeros((n, max_seq_len), dtype=np.int64)
        y_wait = np.zeros((n, max_seq_len), dtype=np.float32)
        mask = np.zeros((n, max_seq_len), dtype=np.float32)
        ds_names = np.array([ep.dataset_id for ep in eps], dtype=str)

        for i, ep in enumerate(eps):
            for t_idx, turn in enumerate(ep.turns[:max_seq_len]):
                e_arr[i, t_idx] = emb_matrix[unique_texts[turn.text]]
                x_arr[i, t_idx] = turn.x_num
                y_gate[i, t_idx] = turn.label_gate
                y_wait[i, t_idx] = turn.wait_delta_ms
                mask[i, t_idx] = turn.loss_mask

        arrays[f"{split_name}_e256"] = e_arr
        arrays[f"{split_name}_xnum"] = x_arr
        arrays[f"{split_name}_ygate"] = y_gate
        arrays[f"{split_name}_ywait"] = y_wait
        arrays[f"{split_name}_mask"] = mask
        arrays[f"{split_name}_ds"] = ds_names

    cache_path.parent.mkdir(parents=True, exist_ok=True)
    np.savez(cache_path, **arrays)
    return cache_path


def load_cached_shards(cache_path: Path) -> dict[str, dict[str, np.ndarray]]:
    """Loads cached split shards and validates all arrays are finite and schema-conformant."""
    data = np.load(cache_path, allow_pickle=False)
    out: dict[str, dict[str, np.ndarray]] = {}
    for split in ("train", "calibration", "test"):
        e = data[f"{split}_e256"]
        x = data[f"{split}_xnum"]
        yg = data[f"{split}_ygate"]
        yw = data[f"{split}_ywait"]
        m = data[f"{split}_mask"]
        ds = data[f"{split}_ds"]
        assert e.ndim == 3 and e.shape[2] == 256
        assert x.ndim == 3 and x.shape[2] == NUM_FEATURES
        assert np.all(np.isfinite(e))
        assert np.all(np.isfinite(x))
        assert np.all(np.isfinite(yw))
        assert np.all(np.isfinite(m))
        out[split] = {
            "e_256": e,
            "x_num": x,
            "y_gate": yg,
            "y_wait_ms": yw,
            "mask": m,
            "dataset_id": ds,
        }
    return out


# ---------------------------------------------------------------------------
# Phase 2 & Phase 3 Multi-Task Shards + Strict Lab Eval Exclusion
# ---------------------------------------------------------------------------


def _sample_clustered_lab_ids(
    cluster_ids: list[str], target_n: int, seed: int
) -> set[str]:
    """Exact mirror of `sample_clustered_indices` in `crates/lab/src/datasets/framework.rs`."""
    unique_clusters = sorted(set(cluster_ids))
    # Pair with original indices sorted by cluster_id (matches BTreeMap<&str, Vec<usize>>)
    clusters: list[tuple[str, int]] = []
    counts: dict[str, int] = {}
    for cid in cluster_ids:
        counts[cid] = counts.get(cid, 0) + 1
    for cid in unique_clusters:
        clusters.append((cid, counts[cid]))
    rng = RustSplitMix64(seed)
    rng.shuffle(clusters)
    excluded: set[str] = set()
    chosen_count = 0
    for cid, cnt in clusters:
        if chosen_count > 0 and chosen_count + cnt > target_n:
            break
        excluded.add(cid)
        chosen_count += cnt
        if chosen_count >= target_n:
            break
    return excluded


def _dyntom_lab_eval_clusters(trials: dict[str, Any], seeds: tuple[int, ...] = (1,)) -> set[str]:
    sorted_ids = sorted(trials.keys())
    excluded: set[str] = set()
    for seed in seeds:
        excluded |= _sample_clustered_lab_ids(sorted_ids, target_n=471, seed=seed)
    return excluded


def _longmemeval_lab_eval_indices(items: list[dict[str, Any]]) -> set[int]:
    """Mirrors Lab's `select_pilot_longmemeval_ids` (seed=7, target=20) + pilot prefix 0..20."""
    excluded: set[int] = set(range(min(20, len(items))))
    by_type: dict[str, list[int]] = {}
    for idx, item in enumerate(items):
        qtype = str(item.get("question_type") or "unknown")
        by_type.setdefault(qtype, []).append(idx)
    rng = RustSplitMix64(7)
    for k in sorted(by_type):
        rng.shuffle(by_type[k])
    total = max(1, len(items))
    target = min(20, total)
    chosen: list[int] = []
    remainders: list[tuple[str, int, int]] = []
    for k in sorted(by_type):
        idxs = by_type[k]
        q = (len(idxs) * target) // total
        rem = (len(idxs) * target) % total
        take = min(q, len(idxs))
        chosen.extend(idxs[:take])
        remainders.append((k, take, rem))
    remainders.sort(key=lambda t: (-t[2], t[0]))
    for k, taken, _ in remainders:
        if len(chosen) >= target:
            break
        if taken < len(by_type[k]):
            chosen.append(by_type[k][taken])
    excluded.update(chosen)
    return excluded


def _load_longmemeval_phase2(dataset_dir: Path, max_seq_len: int = 24) -> list[TrajectoryEpisode]:
    oracle_path = dataset_dir / "longmemeval_oracle.json"
    items: list[dict[str, Any]] = json.loads(oracle_path.read_text(encoding="utf-8"))
    excluded_idx = _longmemeval_lab_eval_indices(items)
    episodes: list[TrajectoryEpisode] = []

    for idx, item in enumerate(items):
        if idx in excluded_idx:
            continue
        qid = str(item.get("question_id", f"lme_{idx:04d}"))
        is_abs = qid.endswith("_abs")
        qtype = str(item.get("question_type", ""))
        sessions = item.get("haystack_sessions", [])

        # Select evidence turns + limited distractors up to max_seq_len - 1
        cand_turns: list[tuple[str, bool, str]] = []
        for sess in sessions:
            for t in sess:
                role = str(t.get("role", "user"))
                content = str(t.get("content", ""))[:600]
                has_ans = bool(t.get("has_answer", False))
                if content:
                    cand_turns.append((role, has_ans, content))

        ev_turns = [c for c in cand_turns if c[1]]
        other_turns = [c for c in cand_turns if not c[1]]
        budget = max_seq_len - 1
        chosen = (ev_turns + other_turns[: max(0, budget - len(ev_turns))])[:budget]

        turns: list[TurnRecord] = []
        for t_i, (role, has_ans, txt) in enumerate(chosen):
            x_num = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=False,
                    is_explicit_addressee=(role == "user"),
                    is_reply_or_expect_link=(t_i > 0),
                    is_dyadic_place=True,
                    active_participant_count=2,
                    dt_since_last_place_event_s=60.0 if t_i > 0 else 0.0,
                    dt_since_last_soul_act_s=120.0,
                    dt_since_sender_last_act_s=60.0 if t_i > 0 else 86400.0,
                    soul_acts_window=t_i // 2,
                    total_acts_window=t_i + 1,
                    sender_exchange_count=t_i,
                    place_event_count=t_i,
                    max_visible_mem_cos=0.75 if has_ans else 0.15,
                    pending_belief_uncertainty=0.7 if is_abs else 0.1,
                )
            )
            turns.append(
                TurnRecord(
                    text=txt,
                    x_num=x_num,
                    label_gate=GATE_NOTE if has_ans else GATE_SILENT,
                    wait_delta_ms=0.0,
                    loss_mask=1.0,
                    y_uncertainty=0.85 if is_abs else (0.15 if has_ans else 0.35),
                    mask_uncertainty=1.0,
                    y_verify=0.75 if has_ans else 0.25,
                    mask_verify=1.0,
                    y_encode_importance=1.0 if has_ans else 0.05,
                    mask_encode_importance=1.0,
                )
            )

        q_text = str(item.get("question", ""))[:600]
        needs_deep_verify = is_abs or ("multi" in qtype) or ("temporal" in qtype) or ("update" in qtype)
        q_xnum = build_x_num(
            PerceptNumericInputs(
                is_named_soul=True,
                is_explicit_addressee=True,
                is_reply_or_expect_link=True,
                is_dyadic_place=True,
                active_participant_count=2,
                dt_since_last_place_event_s=3600.0,
                dt_since_last_soul_act_s=3600.0,
                dt_since_sender_last_act_s=3600.0,
                soul_acts_window=len(chosen) // 2,
                total_acts_window=len(chosen) + 1,
                sender_exchange_count=len(chosen),
                place_event_count=len(chosen),
                max_visible_mem_cos=0.10 if is_abs else 0.85,
                max_visible_mem_age_s=86400.0,
                pending_belief_uncertainty=1.0 if is_abs else 0.15,
            )
        )
        turns.append(
            TurnRecord(
                text=q_text,
                x_num=q_xnum,
                label_gate=GATE_WAIT if is_abs else GATE_GO,
                wait_delta_ms=5000.0 if is_abs else 0.0,
                loss_mask=1.0,
                y_uncertainty=1.0 if is_abs else 0.05,
                mask_uncertainty=1.0,
                y_verify=1.0 if needs_deep_verify else 0.65,
                mask_verify=1.0,
                y_encode_importance=0.6,
                mask_encode_importance=1.0,
            )
        )

        episodes.append(
            TrajectoryEpisode(
                episode_id=f"lme:{idx:04d}",
                dataset_id="longmemeval",
                cluster_id=f"lme:{qid}",
                split=_stable_split_bucket(f"lme:{qid}"),
                turns=turns,
            )
        )
    return episodes


def _load_stale_phase2(dataset_dir: Path, max_seq_len: int = 24) -> list[TrajectoryEpisode]:
    raw_items: list[dict[str, Any]] = json.loads(
        (dataset_dir / "T1_T2_400_FULL.json").read_text(encoding="utf-8")
    )
    episodes: list[TrajectoryEpisode] = []
    # Reserve idx < 20 for Lab pilot isolation
    for idx, sc in enumerate(raw_items):
        if idx < 20:
            continue
        uid = str(sc.get("uid") or f"stale_{idx:04d}")
        sc_type = str(sc.get("type", "T1"))
        sessions = sc.get("haystack_session", [])
        rel_indices = sorted({int(i) for i in sc.get("relevant_session_index", []) if 0 <= int(i) < len(sessions)})
        distractor_indices = [i for i in range(len(sessions)) if i not in rel_indices][:2]
        picked_sessions = sorted(set(rel_indices + distractor_indices))

        turns: list[TurnRecord] = []
        max_hist = max_seq_len - 3
        first_rel = rel_indices[0] if rel_indices else -1
        last_rel = rel_indices[-1] if rel_indices else -1

        for s_idx in picked_sessions:
            sess = sessions[s_idx]
            is_rel = s_idx in rel_indices
            is_stale_old = is_rel and s_idx == first_rel and len(rel_indices) > 1
            is_updated = is_rel and s_idx == last_rel
            for u_turn in sess[:4]:
                if len(turns) >= max_hist:
                    break
                content = str(u_turn.get("content", ""))[:500]
                if not content:
                    continue
                t_i = len(turns)
                x_num = build_x_num(
                    PerceptNumericInputs(
                        is_named_soul=False,
                        is_explicit_addressee=(u_turn.get("role") == "user"),
                        is_reply_or_expect_link=(t_i > 0),
                        is_dyadic_place=True,
                        active_participant_count=2,
                        dt_since_last_place_event_s=300.0 if t_i > 0 else 0.0,
                        dt_since_last_soul_act_s=600.0,
                        dt_since_sender_last_act_s=300.0 if t_i > 0 else 86400.0,
                        soul_acts_window=t_i // 2,
                        total_acts_window=t_i + 1,
                        sender_exchange_count=t_i,
                        place_event_count=t_i,
                        max_visible_mem_cos=0.80 if is_rel else 0.20,
                        max_visible_mem_age_s=172800.0 if is_stale_old else 3600.0,
                        pending_belief_uncertainty=0.85 if is_stale_old else (0.30 if is_updated else 0.10),
                    )
                )
                turns.append(
                    TurnRecord(
                        text=content,
                        x_num=x_num,
                        label_gate=GATE_NOTE if is_rel else GATE_SILENT,
                        wait_delta_ms=0.0,
                        loss_mask=1.0,
                        y_uncertainty=0.85 if is_stale_old else (0.20 if is_updated else 0.15),
                        mask_uncertainty=1.0,
                        y_verify=0.90 if is_rel else 0.10,
                        mask_verify=1.0,
                        y_encode_importance=1.0 if is_updated else (0.45 if is_stale_old else 0.05),
                        mask_encode_importance=1.0,
                    )
                )

        probes = sc.get("probing_queries", {})
        for q_key, unc_val, ver_val in (
            ("dim1_query", 0.30, 1.0),
            ("dim2_query", 0.95, 1.0),
            ("dim3_query", 0.85 if sc_type == "T2" else 0.55, 1.0),
        ):
            q_txt = str(probes.get(q_key, ""))[:500]
            if not q_txt:
                continue
            t_i = len(turns)
            q_xnum = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=True,
                    is_explicit_addressee=True,
                    is_reply_or_expect_link=True,
                    is_dyadic_place=True,
                    active_participant_count=2,
                    dt_since_last_place_event_s=1800.0,
                    dt_since_last_soul_act_s=1800.0,
                    dt_since_sender_last_act_s=1800.0,
                    soul_acts_window=t_i // 2,
                    total_acts_window=t_i + 1,
                    sender_exchange_count=t_i,
                    place_event_count=t_i,
                    max_visible_mem_cos=0.75,
                    max_visible_mem_age_s=86400.0,
                    pending_belief_uncertainty=unc_val,
                )
            )
            turns.append(
                TurnRecord(
                    text=q_txt,
                    x_num=q_xnum,
                    label_gate=GATE_GO,
                    wait_delta_ms=0.0,
                    loss_mask=1.0,
                    y_uncertainty=unc_val,
                    mask_uncertainty=1.0,
                    y_verify=ver_val,
                    mask_verify=1.0,
                )
            )

        if turns:
            episodes.append(
                TrajectoryEpisode(
                    episode_id=f"stale:{uid}",
                    dataset_id="stale",
                    cluster_id=f"stale:{uid}",
                    split=_stable_split_bucket(f"stale:{uid}"),
                    turns=turns,
                )
            )
    return episodes


def _load_hi_tom_phase2(dataset_dir: Path, max_seq_len: int = 24) -> list[TrajectoryEpisode]:
    rows = json.loads((dataset_dir / "Hi-ToM_data/Hi-ToM_data.json").read_text(encoding="utf-8"))["data"]
    grouped: dict[str, list[dict[str, Any]]] = {}
    for row in rows:
        grouped.setdefault(row["story"], []).append(row)

    episodes: list[TrajectoryEpisode] = []
    # Reserve first 20 story clusters
    for cluster_idx, (story_text, qas) in enumerate(sorted(grouped.items())):
        if cluster_idx < 20:
            continue
        lines = [ln.strip() for ln in story_text.splitlines() if ln.strip()]
        qas_sorted = sorted(qas, key=lambda q: (int(q.get("question_order", 0)), int(q.get("sample_id") or 0)))
        hist_budget = max(4, max_seq_len - len(qas_sorted))
        picked_lines = lines[:hist_budget]
        has_deception = any(bool(q.get("deception", False)) for q in qas_sorted)

        turns: list[TurnRecord] = []
        for idx_l, line in enumerate(picked_lines):
            is_private_tell = (" tell " in line.lower()) or (" whisper" in line.lower()) or (" privately" in line.lower())
            x_num = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=False,
                    is_explicit_addressee=False,
                    is_reply_or_expect_link=(idx_l > 0),
                    is_dyadic_place=is_private_tell,
                    active_participant_count=2 if is_private_tell else 4,
                    dt_since_last_place_event_s=1.0 if idx_l > 0 else 0.0,
                    dt_since_last_soul_act_s=86400.0,
                    dt_since_sender_last_act_s=2.0,
                    soul_acts_window=0,
                    total_acts_window=idx_l + 1,
                    sender_exchange_count=idx_l,
                    place_event_count=idx_l,
                    pending_belief_uncertainty=0.65 if is_private_tell else 0.15,
                )
            )
            turns.append(
                TurnRecord(
                    text=line,
                    x_num=x_num,
                    label_gate=GATE_NOTE if (has_deception and is_private_tell) else GATE_SILENT,
                    wait_delta_ms=0.0,
                    loss_mask=1.0,
                    y_uncertainty=0.55 if is_private_tell else 0.15,
                    mask_uncertainty=1.0,
                    y_verify=0.70 if is_private_tell else 0.15,
                    mask_verify=1.0,
                )
            )

        for qa in qas_sorted[: max(1, max_seq_len - len(turns))]:
            order = int(qa.get("question_order", 0))
            deception = bool(qa.get("deception", False))
            unc = min(1.0, 0.15 + 0.18 * order + (0.20 if deception else 0.0))
            ver = 1.0 if (order >= 2 or deception) else 0.35
            t_i = len(turns)
            q_xnum = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=True,
                    is_explicit_addressee=True,
                    is_reply_or_expect_link=True,
                    is_dyadic_place=True,
                    active_participant_count=2,
                    dt_since_last_place_event_s=1.0,
                    dt_since_last_soul_act_s=86400.0,
                    dt_since_sender_last_act_s=1.0,
                    soul_acts_window=0,
                    total_acts_window=t_i + 1,
                    sender_exchange_count=t_i,
                    place_event_count=t_i,
                    pending_belief_uncertainty=unc,
                )
            )
            turns.append(
                TurnRecord(
                    text=str(qa.get("question", "")),
                    x_num=q_xnum,
                    label_gate=GATE_GO,
                    wait_delta_ms=0.0,
                    loss_mask=1.0,
                    y_uncertainty=unc,
                    mask_uncertainty=1.0,
                    y_verify=ver,
                    mask_verify=1.0,
                )
            )

        cid = f"hitom_story_{cluster_idx:04d}"
        episodes.append(
            TrajectoryEpisode(
                episode_id=cid,
                dataset_id="hi_tom",
                cluster_id=cid,
                split=_stable_split_bucket(cid),
                turns=turns,
            )
        )
    return episodes


def _build_dyntom_stage_text(stage: dict[str, Any], scenario_no: int) -> str:
    setting = str(stage.get("social setting", "unknown setting"))
    chars = str(stage.get("characters information", ""))[:200]
    sketch = stage.get("sketch", {}) if isinstance(stage.get("sketch"), dict) else {}
    mental_map = sketch.get("mental states analysis in every scenario", {})
    mental = (
        mental_map.get(f"scenario {scenario_no}")
        or mental_map.get(str(scenario_no))
        or {}
    )
    if isinstance(mental, dict):
        state = " | ".join(
            f"{k}: {mental.get(k, '')}" for k in ("belief", "emotion", "intention", "action") if mental.get(k)
        )
    else:
        state = ""
    return f"Scenario {scenario_no} in {setting}. Characters: {chars}. State: {state}"[:550]


def _load_dyntom_phase2(dataset_dir: Path, max_seq_len: int = 24) -> list[TrajectoryEpisode]:
    trials: dict[str, Any] = json.loads((dataset_dir / "DynToM.json").read_text(encoding="utf-8"))
    excluded_clusters = _dyntom_lab_eval_clusters(trials, seeds=(1,))
    episodes: list[TrajectoryEpisode] = []

    for cid in sorted(trials.keys()):
        if cid in excluded_clusters:
            continue
        trial = trials[cid]
        stage = trial.get("stage", {})
        n_scen = int(stage.get("scenario numbers", 5) or 5)

        turns: list[TurnRecord] = []
        for s_no in range(1, min(n_scen, 6) + 1):
            st_txt = _build_dyntom_stage_text(stage, s_no)
            x_num = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=False,
                    is_explicit_addressee=False,
                    is_reply_or_expect_link=(s_no > 1),
                    is_dyadic_place=False,
                    active_participant_count=4,
                    dt_since_last_place_event_s=60.0 if s_no > 1 else 0.0,
                    dt_since_last_soul_act_s=86400.0,
                    dt_since_sender_last_act_s=60.0,
                    soul_acts_window=0,
                    total_acts_window=s_no,
                    sender_exchange_count=s_no - 1,
                    place_event_count=s_no - 1,
                    pending_belief_uncertainty=min(0.8, 0.12 * s_no),
                )
            )
            turns.append(
                TurnRecord(
                    text=st_txt,
                    x_num=x_num,
                    label_gate=GATE_NOTE if s_no > 1 else GATE_SILENT,
                    wait_delta_ms=0.0,
                    loss_mask=1.0,
                    y_uncertainty=0.15 if s_no == 1 else min(0.75, 0.15 * s_no),
                    mask_uncertainty=1.0,
                    y_verify=0.20 if s_no == 1 else 0.60,
                    mask_verify=1.0,
                )
            )

        q_items = sorted(trial.get("question", {}).items())
        # Sample up to 6 diverse questions per context across question types
        picked_qs = q_items[: min(6, max(0, max_seq_len - len(turns)))]
        for _qid, q in picked_qs:
            qtype = str(q.get("question type", "")).lower()
            is_hard = ("c" in qtype) or ("d" in qtype) or ("belief" in str(q.get("question", "")).lower())
            unc = 0.85 if is_hard else 0.30
            ver = 1.0 if is_hard else 0.50
            t_i = len(turns)
            q_xnum = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=True,
                    is_explicit_addressee=True,
                    is_reply_or_expect_link=True,
                    is_dyadic_place=True,
                    active_participant_count=2,
                    dt_since_last_place_event_s=60.0,
                    dt_since_last_soul_act_s=86400.0,
                    dt_since_sender_last_act_s=60.0,
                    soul_acts_window=0,
                    total_acts_window=t_i + 1,
                    sender_exchange_count=t_i,
                    place_event_count=t_i,
                    pending_belief_uncertainty=unc,
                )
            )
            turns.append(
                TurnRecord(
                    text=str(q.get("question", ""))[:500],
                    x_num=q_xnum,
                    label_gate=GATE_GO,
                    wait_delta_ms=0.0,
                    loss_mask=1.0,
                    y_uncertainty=unc,
                    mask_uncertainty=1.0,
                    y_verify=ver,
                    mask_verify=1.0,
                )
            )

        episodes.append(
            TrajectoryEpisode(
                episode_id=f"dyntom:{cid}",
                dataset_id="dyntom",
                cluster_id=f"dyntom:{cid}",
                split=_stable_split_bucket(f"dyntom:{cid}"),
                turns=turns,
            )
        )
    return episodes


def build_phase2_episodes(repo_root: Path, max_seq_len: int = 24) -> list[TrajectoryEpisode]:
    """Loads all 4 OSI-verified Phase 2 datasets with strict Lab eval exclusions."""
    manifest_path = assert_safe_path(repo_root / "lab" / "datasets.manifest.toml", repo_root=repo_root)
    episodes: list[TrajectoryEpisode] = []
    for ds_id in PHASE2_DATASETS:
        entry = require_dataset_allowed(manifest_path, ds_id)
        ds_dir = assert_safe_path(repo_root / "lab" / "datasets" / entry.relative_dir, repo_root=repo_root)
        if ds_id == "longmemeval":
            episodes.extend(_load_longmemeval_phase2(ds_dir, max_seq_len=max_seq_len))
        elif ds_id == "stale":
            episodes.extend(_load_stale_phase2(ds_dir, max_seq_len=max_seq_len))
        elif ds_id == "hi_tom":
            episodes.extend(_load_hi_tom_phase2(ds_dir, max_seq_len=max_seq_len))
        elif ds_id == "dyntom":
            episodes.extend(_load_dyntom_phase2(ds_dir, max_seq_len=max_seq_len))
    return episodes


def _tok_set(text: str) -> set[str]:
    stop = {"the", "a", "an", "is", "are", "was", "were", "in", "on", "at", "to", "of", "for", "and", "or"}
    return {
        w for w in re.findall(r"[a-z0-9]+", text.lower()) if len(w) >= 3 and w not in stop
    }


def _load_group_mem_bench_phase3(dataset_dir: Path, max_seq_len: int = 24) -> list[TrajectoryEpisode]:
    domains = ("Finance", "Healthcare", "Manufacturing", "Technology")
    episodes: list[TrajectoryEpisode] = []

    for domain in domains:
        conv_path = (
            dataset_dir
            / "data"
            / "final"
            / domain
            / f"synthetic_domain_channels_rolevariants_{domain}.json"
        )
        channels: dict[str, list[dict[str, Any]]] = json.loads(conv_path.read_text(encoding="utf-8"))

        # Flatten channel messages cleanly with precomputed token sets for fast matching
        chan_msgs: dict[str, list[dict[str, Any]]] = {}
        chan_speakers: dict[str, set[str]] = {}
        chan_msg_toks: dict[str, list[set[str]]] = {}
        chan_all_toks: dict[str, set[str]] = {}
        for ch_name, msgs in sorted(channels.items()):
            ordered = sorted(msgs, key=lambda m: (str(m.get("timestamp", "")), str(m.get("msg_node", ""))))
            chan_msgs[ch_name] = ordered
            chan_speakers[ch_name] = {str(m.get("author", "")) for m in ordered}
            m_toks = [_tok_set(str(m.get("content", ""))) for m in ordered]
            chan_msg_toks[ch_name] = m_toks
            all_t: set[str] = set()
            for ts in m_toks:
                all_t |= ts
            chan_all_toks[ch_name] = all_t

        q_dir = dataset_dir / "questions" / domain
        all_qs: list[tuple[str, dict[str, Any]]] = []
        for q_file in sorted(q_dir.glob("*.jsonl")):
            cat = q_file.stem
            for line in q_file.read_text(encoding="utf-8").splitlines():
                if line.strip():
                    all_qs.append((cat, json.loads(line)))

        # Reserve first 10% of questions in each domain to isolate Lab pilot runs
        reserve_n = max(2, len(all_qs) // 10)
        chan_keys = sorted(chan_msgs.keys())
        for q_idx, (cat, q) in enumerate(all_qs):
            if q_idx < reserve_n:
                continue
            q_id = str(q.get("id", f"{cat}_{q_idx}"))
            q_text = str(q.get("question", ""))[:500]
            ans_text = str(q.get("answer", ""))
            asker = str(q.get("asking_user_id", ""))
            is_abs = cat == "abstention" or "no information available" in ans_text.lower()
            is_update = cat == "knowledge_update"

            q_toks = _tok_set(q_text)
            a_toks = _tok_set(ans_text)
            qa_toks = q_toks | a_toks

            # Pick best matching channel (preferring asker participation)
            best_ch = chan_keys[q_idx % len(chan_keys)]
            best_score = -1.0
            for ch in chan_keys:
                asker_bonus = 2.0 if asker and asker in chan_speakers[ch] else 0.0
                overlap = len(qa_toks & chan_all_toks[ch]) / max(1, len(qa_toks))
                sc = asker_bonus + overlap
                if sc > best_score:
                    best_score = sc
                    best_ch = ch

            msgs = chan_msgs[best_ch]
            msg_toks = chan_msg_toks[best_ch]
            scored_msgs: list[tuple[float, int, dict[str, Any]]] = []
            for m_idx, m in enumerate(msgs):
                c_toks = msg_toks[m_idx]
                a_cov = len(a_toks & c_toks) / max(1, len(a_toks)) if not is_abs else 0.0
                q_cov = len(q_toks & c_toks) / max(1, len(q_toks))
                scored_msgs.append((2.0 * a_cov + q_cov, m_idx, m))

            scored_msgs.sort(key=lambda t: (-t[0], t[1]))
            positives = [t for t in scored_msgs if t[0] >= 0.35 and not is_abs][:4]
            if not positives and not is_abs and scored_msgs:
                positives = [scored_msgs[0]]
            negatives = [t for t in reversed(scored_msgs) if t not in positives][:5]
            # Also add cross-channel negative distractors
            other_ch = chan_keys[(chan_keys.index(best_ch) + 1) % len(chan_keys)]
            for om in chan_msgs[other_ch][:2]:
                negatives.append((0.0, 9999, om))

            cand_pool: list[tuple[bool, int, dict[str, Any]]] = [
                (True, m_idx, m) for _s, m_idx, m in positives
            ] + [(False, m_idx, m) for _s, m_idx, m in negatives]
            cand_pool = cand_pool[: max_seq_len - 1]

            turns: list[TurnRecord] = []
            for t_i, (is_pos, m_idx, m) in enumerate(cand_pool):
                c_txt = str(m.get("content", ""))[:500]
                is_superseded = is_update and is_pos and (m_idx < max((p[1] for p in positives), default=0))
                rel_lbl = 0.25 if is_superseded else (1.0 if is_pos else 0.0)
                enc_lbl = 1.0 if is_pos else 0.05
                x_num = build_x_num(
                    PerceptNumericInputs(
                        is_named_soul=False,
                        is_explicit_addressee=False,
                        is_reply_or_expect_link=bool(m.get("reply_to")),
                        is_dyadic_place=False,
                        active_participant_count=6,
                        dt_since_last_place_event_s=120.0 if t_i > 0 else 0.0,
                        dt_since_last_soul_act_s=86400.0,
                        dt_since_sender_last_act_s=240.0,
                        soul_acts_window=0,
                        total_acts_window=t_i + 1,
                        sender_exchange_count=t_i,
                        place_event_count=t_i,
                        max_visible_mem_age_s=float(max(1, len(msgs) - min(len(msgs), m_idx)) * 600),
                        pending_belief_uncertainty=1.0 if is_superseded else (0.8 if is_abs else 0.0),
                    )
                )
                turns.append(
                    TurnRecord(
                        text=c_txt,
                        x_num=x_num,
                        label_gate=GATE_NOTE if is_pos else GATE_SILENT,
                        wait_delta_ms=0.0,
                        loss_mask=1.0,
                        y_uncertainty=0.85 if is_abs else (0.7 if is_superseded else 0.1),
                        mask_uncertainty=1.0,
                        y_verify=1.0 if (is_abs or is_update or cat == "multi_hop") else 0.35,
                        mask_verify=1.0,
                        y_encode_importance=enc_lbl,
                        mask_encode_importance=1.0,
                        y_memory_relevance=rel_lbl,
                        mask_memory_relevance=1.0,
                        pair_query_text=q_text,
                    )
                )

            # Final query turn
            q_xnum = build_x_num(
                PerceptNumericInputs(
                    is_named_soul=True,
                    is_explicit_addressee=True,
                    is_reply_or_expect_link=True,
                    is_dyadic_place=False,
                    active_participant_count=6,
                    dt_since_last_place_event_s=600.0,
                    dt_since_last_soul_act_s=86400.0,
                    dt_since_sender_last_act_s=600.0,
                    soul_acts_window=0,
                    total_acts_window=len(turns) + 1,
                    sender_exchange_count=len(turns),
                    place_event_count=len(turns),
                    max_visible_mem_cos=0.1 if is_abs else 0.85,
                    pending_belief_uncertainty=1.0 if is_abs else 0.15,
                )
            )
            turns.append(
                TurnRecord(
                    text=q_text,
                    x_num=q_xnum,
                    label_gate=GATE_WAIT if is_abs else GATE_GO,
                    wait_delta_ms=4000.0 if is_abs else 0.0,
                    loss_mask=1.0,
                    y_uncertainty=1.0 if is_abs else 0.10,
                    mask_uncertainty=1.0,
                    y_verify=1.0,
                    mask_verify=1.0,
                    y_encode_importance=0.7,
                    mask_encode_importance=1.0,
                    y_memory_relevance=0.0 if is_abs else 0.9,
                    mask_memory_relevance=1.0,
                )
            )

            cluster_key = f"gmb:{domain}:{best_ch}"
            episodes.append(
                TrajectoryEpisode(
                    episode_id=f"gmb:{domain}:{q_id}",
                    dataset_id="group_mem_bench",
                    cluster_id=cluster_key,
                    split=_stable_split_bucket(cluster_key),
                    turns=turns,
                )
            )
    return episodes


def _decode_bytes(raw: Any) -> str:
    if isinstance(raw, list):
        return bytes(raw).decode("utf-8", errors="replace")
    return str(raw)


def _load_memory_agent_bench_phase3(dataset_dir: Path, max_seq_len: int = 24) -> list[TrajectoryEpisode]:
    payload_path = dataset_dir / ".monoagens_converted_payload.json"
    if not payload_path.is_file():
        raise FileNotFoundError(
            f"Missing converted MemoryAgentBench payload at {payload_path}; run Lab converter first."
        )
    payload = json.loads(payload_path.read_text(encoding="utf-8"))
    gold_by_pair: dict[tuple[str, str], str] = {}
    for g in payload.get("gold_book", {}).get("entries", []):
        gold_by_pair[(g["item_id"], g["probe_id"])] = str(g.get("gold_answer", ""))

    # Partition items per category, reserving first item(s) per category for Lab pilot isolation
    by_cat: dict[str, list[dict[str, Any]]] = {"Accurate_Retrieval": [], "Conflict_Resolution": []}
    for it in payload.get("items", []):
        cat = "Conflict_Resolution" if "Conflict_Resolution" in it["item_id"] else "Accurate_Retrieval"
        by_cat[cat].append(it)

    episodes: list[TrajectoryEpisode] = []
    for cat, cat_items in by_cat.items():
        reserve = 2 if cat == "Accurate_Retrieval" else 1
        usable = cat_items[reserve:]
        for pos, it in enumerate(usable):
            # Deterministic item-level split covering train/cal/test in both AR and CR
            mod = pos % 10
            if mod == 8:
                split = "calibration"
            elif mod == 9:
                split = "test"
            else:
                split = "train"

            hist_texts = [_decode_bytes(h["payload"])[:600] for h in it.get("history", [])]
            hist_toks = [_tok_set(t) for t in hist_texts]
            hist_lower = [t.lower() for t in hist_texts]
            inv_index: dict[str, list[int]] = {}
            for c_idx, c_t in enumerate(hist_toks):
                for tok in c_t:
                    inv_index.setdefault(tok, []).append(c_idx)
            probes = it.get("probes", [])[:20]

            for p_idx, pr in enumerate(probes):
                q_txt = _decode_bytes(pr["percept"]["payload"])[:500]
                gold_ans = gold_by_pair.get((it["item_id"], pr["probe_id"]), "")
                ans_norm = gold_ans.strip().lower()
                a_toks = _tok_set(gold_ans)
                q_toks = _tok_set(q_txt)

                cand_indices: set[int] = set()
                for tok in (a_toks | q_toks):
                    cand_indices.update(inv_index.get(tok, ())[:60])
                if not cand_indices:
                    cand_indices = set(range(min(30, len(hist_texts))))

                scored_chunks: list[tuple[float, int, str]] = []
                for c_idx in cand_indices:
                    c_txt = hist_texts[c_idx]
                    exact_hit = 1.5 if (ans_norm and len(ans_norm) >= 3 and ans_norm in hist_lower[c_idx]) else 0.0
                    c_t = hist_toks[c_idx]
                    a_ov = len(a_toks & c_t) / max(1, len(a_toks))
                    q_ov = len(q_toks & c_t) / max(1, len(q_toks))
                    scored_chunks.append((exact_hit + a_ov + 0.5 * q_ov, c_idx, c_txt))

                scored_chunks.sort(key=lambda t: (-t[0], -t[1]))
                positives = [t for t in scored_chunks if t[0] >= 0.7][:4]
                if not positives and scored_chunks:
                    positives = [scored_chunks[0]]
                pos_indices = {t[1] for t in positives}
                # Pick hard distractors (from inverted index first, plus stride fallback)
                negatives = [t for t in scored_chunks[len(positives) :] if t[1] not in pos_indices][:5]
                if len(negatives) < 5 and hist_texts:
                    stride = max(1, len(hist_texts) // 7)
                    for step_i in range(0, len(hist_texts), stride):
                        if step_i not in pos_indices and all(n[1] != step_i for n in negatives):
                            negatives.append((0.0, step_i, hist_texts[step_i]))
                            if len(negatives) >= 5:
                                break

                is_conflict = cat == "Conflict_Resolution"
                latest_pos_idx = max((t[1] for t in positives), default=0)

                turns: list[TurnRecord] = []
                for is_pos, c_idx, c_txt in (
                    [(True, ci, ct) for _, ci, ct in positives]
                    + [(False, ci, ct) for _, ci, ct in negatives]
                )[: max_seq_len - 1]:
                    is_stale_pos = is_conflict and is_pos and c_idx < latest_pos_idx
                    rel_lbl = 0.25 if is_stale_pos else (1.0 if is_pos else 0.0)
                    enc_lbl = 1.0 if is_pos else 0.05
                    t_i = len(turns)
                    x_num = build_x_num(
                        PerceptNumericInputs(
                            is_named_soul=False,
                            is_explicit_addressee=False,
                            is_reply_or_expect_link=False,
                            is_dyadic_place=True,
                            active_participant_count=2,
                            dt_since_last_place_event_s=60.0 if t_i > 0 else 0.0,
                            dt_since_last_soul_act_s=86400.0,
                            dt_since_sender_last_act_s=60.0,
                            soul_acts_window=0,
                            total_acts_window=t_i + 1,
                            sender_exchange_count=t_i,
                            place_event_count=t_i,
                            max_visible_mem_age_s=float(max(1, len(hist_texts) - c_idx) * 60),
                            pending_belief_uncertainty=0.9 if is_stale_pos else 0.1,
                        )
                    )
                    turns.append(
                        TurnRecord(
                            text=c_txt,
                            x_num=x_num,
                            label_gate=GATE_NOTE if is_pos else GATE_SILENT,
                            wait_delta_ms=0.0,
                            loss_mask=1.0,
                            y_uncertainty=0.80 if is_stale_pos else 0.10,
                            mask_uncertainty=1.0,
                            y_verify=1.0 if is_conflict else 0.4,
                            mask_verify=1.0,
                            y_encode_importance=enc_lbl,
                            mask_encode_importance=1.0,
                            y_memory_relevance=rel_lbl,
                            mask_memory_relevance=1.0,
                            pair_query_text=q_txt,
                        )
                    )

                turns.append(
                    TurnRecord(
                        text=q_txt,
                        x_num=build_x_num(
                            PerceptNumericInputs(
                                is_named_soul=True,
                                is_explicit_addressee=True,
                                is_reply_or_expect_link=True,
                                is_dyadic_place=True,
                                active_participant_count=2,
                                dt_since_last_place_event_s=60.0,
                                dt_since_last_soul_act_s=86400.0,
                                dt_since_sender_last_act_s=60.0,
                                soul_acts_window=0,
                                total_acts_window=len(turns) + 1,
                                sender_exchange_count=len(turns),
                                place_event_count=len(turns),
                                max_visible_mem_cos=0.85,
                                pending_belief_uncertainty=0.8 if is_conflict else 0.15,
                            )
                        ),
                        label_gate=GATE_GO,
                        wait_delta_ms=0.0,
                        loss_mask=1.0,
                        y_uncertainty=0.65 if is_conflict else 0.15,
                        mask_uncertainty=1.0,
                        y_verify=1.0 if is_conflict else 0.5,
                        mask_verify=1.0,
                        y_encode_importance=0.8,
                        mask_encode_importance=1.0,
                        y_memory_relevance=0.9,
                        mask_memory_relevance=1.0,
                    )
                )

                episodes.append(
                    TrajectoryEpisode(
                        episode_id=f"mab:{it['item_id']}:{p_idx:03d}",
                        dataset_id="memory_agent_bench",
                        cluster_id=it["item_id"],
                        split=split,
                        turns=turns,
                    )
                )
    return episodes


def build_phase3_episodes(repo_root: Path, max_seq_len: int = 24) -> list[TrajectoryEpisode]:
    """Loads both OSI-verified Phase 3 datasets with strict split isolation."""
    manifest_path = assert_safe_path(repo_root / "lab" / "datasets.manifest.toml", repo_root=repo_root)
    episodes: list[TrajectoryEpisode] = []
    for ds_id in PHASE3_DATASETS:
        entry = require_dataset_allowed(manifest_path, ds_id)
        ds_dir = assert_safe_path(repo_root / "lab" / "datasets" / entry.relative_dir, repo_root=repo_root)
        if ds_id == "group_mem_bench":
            episodes.extend(_load_group_mem_bench_phase3(ds_dir, max_seq_len=max_seq_len))
        elif ds_id == "memory_agent_bench":
            episodes.extend(_load_memory_agent_bench_phase3(ds_dir, max_seq_len=max_seq_len))
    return episodes


def precompute_multitask_shards(
    repo_root: Path,
    cache_path: Path,
    episodes: list[TrajectoryEpisode],
    embedder_dir: Path | None = None,
    device: str = "cuda",
    max_seq_len: int = 24,
    batch_size: int = 64,
    force_recompute: bool = False,
) -> Path:
    """Encodes all episode turn/query texts via EmbeddingGemma 2 (256d) and caches multi-task shards."""
    if cache_path.is_file() and not force_recompute:
        return cache_path

    if embedder_dir is None:
        embedder_dir = repo_root / "lab" / "datasets" / "embedder_models" / "embeddinggemma-2"
    assert_safe_path(embedder_dir, repo_root=repo_root)
    assert_safe_path(cache_path, repo_root=repo_root)

    unique_texts: dict[str, int] = {}
    for ep in episodes:
        for t in ep.turns[:max_seq_len]:
            if t.text not in unique_texts:
                unique_texts[t.text] = len(unique_texts)
            if t.pair_query_text is not None and t.pair_query_text not in unique_texts:
                unique_texts[t.pair_query_text] = len(unique_texts)

    ordered_texts = [""] * len(unique_texts)
    for txt, idx in unique_texts.items():
        ordered_texts[idx] = txt

    import gc
    import torch
    from sentence_transformers import SentenceTransformer

    use_bf16 = device.startswith("cuda") and torch.cuda.is_available()
    model_kwargs = {"dtype": torch.bfloat16} if use_bf16 else {}
    st_model = SentenceTransformer(str(embedder_dir), device=device, model_kwargs=model_kwargs)
    st_model.max_seq_length = 512
    text_inputs = [{"text": s} for s in ordered_texts]
    emb_matrix = st_model.encode(
        text_inputs,
        batch_size=batch_size,
        truncate_dim=256,
        normalize_embeddings=True,
        convert_to_numpy=True,
        show_progress_bar=False,
    ).astype(np.float32)

    del st_model
    gc.collect()
    if use_bf16:
        torch.cuda.empty_cache()

    if not np.all(np.isfinite(emb_matrix)):
        raise ValueError("Non-finite values produced by EmbeddingGemma 2 encoder")

    split_buckets: dict[str, list[TrajectoryEpisode]] = {"train": [], "calibration": [], "test": []}
    for ep in episodes:
        split_buckets[ep.split].append(ep)

    arrays: dict[str, np.ndarray] = {}
    for split_name, eps in split_buckets.items():
        n = len(eps)
        e_arr = np.zeros((n, max_seq_len, 256), dtype=np.float32)
        x_arr = np.zeros((n, max_seq_len, NUM_FEATURES), dtype=np.float32)
        y_gate = np.zeros((n, max_seq_len), dtype=np.int64)
        y_wait = np.zeros((n, max_seq_len), dtype=np.float32)
        mask = np.zeros((n, max_seq_len), dtype=np.float32)
        y_unc = np.zeros((n, max_seq_len), dtype=np.float32)
        m_unc = np.zeros((n, max_seq_len), dtype=np.float32)
        y_ver = np.zeros((n, max_seq_len), dtype=np.float32)
        m_ver = np.zeros((n, max_seq_len), dtype=np.float32)
        y_enc = np.zeros((n, max_seq_len), dtype=np.float32)
        m_enc = np.zeros((n, max_seq_len), dtype=np.float32)
        y_rel = np.zeros((n, max_seq_len), dtype=np.float32)
        m_rel = np.zeros((n, max_seq_len), dtype=np.float32)
        ds_names = np.array([ep.dataset_id for ep in eps], dtype=str)

        for i, ep in enumerate(eps):
            for t_idx, turn in enumerate(ep.turns[:max_seq_len]):
                cand_emb = emb_matrix[unique_texts[turn.text]]
                e_arr[i, t_idx] = cand_emb
                x_vec = turn.x_num.copy()
                if turn.pair_query_text is not None:
                    q_emb = emb_matrix[unique_texts[turn.pair_query_text]]
                    cos_q = float(np.clip(np.dot(q_emb, cand_emb), 0.0, 1.0))
                    x_vec[13] = cos_q
                x_arr[i, t_idx] = x_vec
                y_gate[i, t_idx] = turn.label_gate
                y_wait[i, t_idx] = turn.wait_delta_ms
                mask[i, t_idx] = turn.loss_mask
                y_unc[i, t_idx] = turn.y_uncertainty
                m_unc[i, t_idx] = turn.mask_uncertainty
                y_ver[i, t_idx] = turn.y_verify
                m_ver[i, t_idx] = turn.mask_verify
                y_enc[i, t_idx] = turn.y_encode_importance
                m_enc[i, t_idx] = turn.mask_encode_importance
                y_rel[i, t_idx] = turn.y_memory_relevance
                m_rel[i, t_idx] = turn.mask_memory_relevance

        arrays[f"{split_name}_e256"] = e_arr
        arrays[f"{split_name}_xnum"] = x_arr
        arrays[f"{split_name}_ygate"] = y_gate
        arrays[f"{split_name}_ywait"] = y_wait
        arrays[f"{split_name}_mask"] = mask
        arrays[f"{split_name}_yunc"] = y_unc
        arrays[f"{split_name}_munc"] = m_unc
        arrays[f"{split_name}_yver"] = y_ver
        arrays[f"{split_name}_mver"] = m_ver
        arrays[f"{split_name}_yenc"] = y_enc
        arrays[f"{split_name}_menc"] = m_enc
        arrays[f"{split_name}_yrel"] = y_rel
        arrays[f"{split_name}_mrel"] = m_rel
        arrays[f"{split_name}_ds"] = ds_names

    cache_path.parent.mkdir(parents=True, exist_ok=True)
    np.savez(cache_path, **arrays)
    return cache_path


def precompute_phase2_shards(
    repo_root: Path,
    cache_path: Path,
    embedder_dir: Path | None = None,
    device: str = "cuda",
    max_seq_len: int = 24,
    batch_size: int = 64,
    force_recompute: bool = False,
) -> Path:
    if cache_path.is_file() and not force_recompute:
        return cache_path
    episodes = build_phase2_episodes(repo_root, max_seq_len=max_seq_len)
    return precompute_multitask_shards(
        repo_root=repo_root,
        cache_path=cache_path,
        episodes=episodes,
        embedder_dir=embedder_dir,
        device=device,
        max_seq_len=max_seq_len,
        batch_size=batch_size,
        force_recompute=force_recompute,
    )


def precompute_phase3_shards(
    repo_root: Path,
    cache_path: Path,
    embedder_dir: Path | None = None,
    device: str = "cuda",
    max_seq_len: int = 24,
    batch_size: int = 64,
    force_recompute: bool = False,
) -> Path:
    if cache_path.is_file() and not force_recompute:
        return cache_path
    episodes = build_phase3_episodes(repo_root, max_seq_len=max_seq_len)
    return precompute_multitask_shards(
        repo_root=repo_root,
        cache_path=cache_path,
        episodes=episodes,
        embedder_dir=embedder_dir,
        device=device,
        max_seq_len=max_seq_len,
        batch_size=batch_size,
        force_recompute=force_recompute,
    )


def load_multitask_shards(cache_path: Path) -> dict[str, dict[str, np.ndarray]]:
    """Loads multi-task shards (backward-compatible with Phase 1 npz files)."""
    data = np.load(cache_path, allow_pickle=False)
    out: dict[str, dict[str, np.ndarray]] = {}
    for split in ("train", "calibration", "test"):
        e = data[f"{split}_e256"]
        x = data[f"{split}_xnum"]
        yg = data[f"{split}_ygate"]
        yw = data[f"{split}_ywait"]
        m = data[f"{split}_mask"]
        ds = data[f"{split}_ds"]
        zeros = np.zeros_like(m, dtype=np.float32)

        def _get(suffix: str, fallback: np.ndarray = zeros) -> np.ndarray:
            key = f"{split}_{suffix}"
            return data[key] if key in data else fallback.copy()

        d: dict[str, np.ndarray] = {
            "e_256": e,
            "x_num": x,
            "y_gate": yg,
            "y_wait_ms": yw,
            "mask": m,
            "y_unc": _get("yunc"),
            "m_unc": _get("munc"),
            "y_ver": _get("yver"),
            "m_ver": _get("mver"),
            "y_enc": _get("yenc"),
            "m_enc": _get("menc"),
            "y_rel": _get("yrel"),
            "m_rel": _get("mrel"),
            "dataset_id": ds,
        }
        for k, arr in d.items():
            if k != "dataset_id":
                assert np.all(np.isfinite(arr)), f"Non-finite shard array {split}/{k}"
        out[split] = d
    return out

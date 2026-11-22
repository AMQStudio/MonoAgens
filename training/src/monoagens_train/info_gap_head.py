"""Multilingual domain-agnostic A-14 logistic `dialogue_act_gap` trainer on frozen `EmbeddingGemma 2` (`t_clf`)."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Sequence

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

from monoagens_train.datasets import format_embedding_text

# Diverse, generic speech-act prototypes spanning EN + ID casual & technical registers.
# Intentionally avoids any test/smoke-scenario phrases so generalization is genuine.
HIGH_GAP_PROTOTYPES: tuple[str, ...] = (
    # English information questions
    "What time does the workshop start tomorrow?",
    "Does anyone know where the shared spreadsheet link is?",
    "How many items are still pending before we submit?",
    "Which room did we move the equipment to?",
    "Who has the updated credentials for the test server?",
    "When is the final revision due?",
    "Where should I upload the scanned invoices?",
    "What port is the local metrics endpoint listening on?",
    "How do I configure the timeout in the config file?",
    "Which branch contains the latest schema migration?",
    "Did the nightly build finish without errors?",
    "Are we still waiting on approval from finance?",
    "Can someone remind me what decision we made earlier?",
    "What percentage of the checklist is actually done so far?",
    "How much budget is remaining for this quarter?",
    "Who was assigned to verify the staging deployment?",
    # English troubleshooting / help requests
    "I keep hitting a connection refused error when starting the daemon.",
    "Not sure why this test keeps timing out after recent edits.",
    "Could someone check if the format looks right here?",
    "I am stuck trying to parse the nested JSON payload.",
    "Why is the container exiting immediately with code 1?",
    "Is there a trick to getting the local GPU runtime detected?",
    # Indonesian information questions (formal & casual)
    "Jam berapa acaranya mulai besok pagi?",
    "Ada yang tau link dokumen terbarunya ditaruh di mana?",
    "Masih kurang berapa bagian lagi yang belum beres?",
    "Ini naruhnya di lantai atau ruangan sebelah mana ya?",
    "Siapa yang lagi pegang kunci ruang arsip sekarang?",
    "Kapan batas akhir pengumpulan berkasnya?",
    "Format penamaan filenya pakai aturan yang mana ya?",
    "Tadi keputusannya jadi pakai jadwal yang jam berapa?",
    "Sudah sampai mana progres pengerjaan bagian belakang?",
    "Berapa sisa kuota yang masih tersedia sekarang?",
    "Ini skripnya dijalankan pakai parameter apa ya?",
    "Kenapa pas build selalu muncul pesan error permission denied?",
    "Bagian konfigurasinya ditaruh di folder mana?",
    "Apakah laporan mingguan kemarin sudah sempat dikirim?",
    "Boleh minta info siapa penanggung jawab shift malam ini?",
    "Tadi yang dibahas pas rapat poin pentingnya apa aja ya?",
    "Kira-kira butuh berapa lama buat ngerjain tahap selanjutnya?",
    "Yang dimaksud di catatan barusan itu file yang mana?",
    "Ada yang pernah nemu kendala kayak gini waktu deploy?",
    "Gimana cara reset state lokalnya kalau kepenuhan?",
    "Tolong bantu cek dong bagian ini masih ada yang kurang gak?",
    "Bingung nih langkah ketiga ini maksudnya gimana ya?",
    "Ada yang tau nomor kontak vendornya siapa?",
    "Stok perlengkapan yang masih ada di gudang tinggal berapa?",
    "Jadwal presentasinya diundur atau tetap sesuai rencana?",
    "Maaf tanya, tadi alamat lengkap lokasinya di sebelah mana?",
)

LOW_GAP_PROTOTYPES: tuple[str, ...] = (
    # English social greetings, banter & casual invitations
    "Good morning everyone, hope you all have a productive day!",
    "Haha that meme totally made my day.",
    "Anyone down to grab coffee after work?",
    "Let's play some games tonight if everyone is free.",
    "Happy Friday team, almost weekend time!",
    "Just got back from lunch, traffic outside is wild.",
    "No worries at all, take your time.",
    "Sounds like a solid plan to me.",
    "Welcome back, glad you could make it!",
    "Great job pulling that together so quickly.",
    "Cheers everyone, catch you tomorrow morning!",
    "Haha yeah I totally forgot about that too.",
    # English status declarations & acknowledgments
    "I finished updating the slides and uploaded them to the folder.",
    "Got it, thanks for letting me know.",
    "Acknowledged, I will take care of the cleanup shortly.",
    "All set on my side, ready whenever you are.",
    "Just merged the cleanup commit into main.",
    "I already sent the confirmation email this morning.",
    "Okay noted, see you all at the venue.",
    "Everything is packed and ready to go.",
    "Fixed the typo in the README.",
    "Just a heads up that I will be offline for an hour.",
    # Indonesian social greetings, banter & casual invitations
    "Haloooo semua semangat terus buat hari ini yaa!",
    "Pagi gesss jangan lupa sarapan dulu biar fokus.",
    "Wkwkwk ngakak banget asli liat kelakuan lu.",
    "Gaspol bro ntar malem kita main bareng yukk?",
    "Ayo nanti sore ngopi santai di tempat biasa?",
    "Mantap jiwa keren banget hasilnya!",
    "Hahaha iya bener banget tuh setuju sih aku.",
    "Btw jalanan macet parah barusan pas berangkat.",
    "Santai aja bro gak usah buru-buru amat.",
    "Semangat terus ya teman-teman dikit lagi beres nih.",
    "Asik banget acaranya rame seru pol.",
    "Duluan ya gess sampai ketemu besok pagi.",
    # Indonesian status declarations, confirmations & acknowledgments
    "Okeee siap aman langsung kulanjutin besok.",
    "Sudah kuberesin sebagian tadi siang kok.",
    "Makasih banyak infonya sangat membantu.",
    "Sippp dicatat, nanti kukabari kalau sudah sampai.",
    "Aman terkendali tinggal lanjut sisanya aja.",
    "Barusan sudah kukirim filenya lewat chat ya.",
    "Oalaahh begitu toh, oke deh paham sekarang.",
    "Siap laksanakan boskuu beres pokoknya.",
    "Iya nih udah beres semua dari kemarin sore.",
    "Oke noted makasih banyak yaa.",
    "Tadi udah dicek sekilas keliatannya udah aman.",
    "Baik kak terima kasih konfirmasinya.",
    "Cuma mau ngabarin aku udah di jalan nih.",
    "Haha sip lah kalau begitu aman jaya.",
)


@dataclass(frozen=True)
class InfoGapFitResult:
    w_gap: np.ndarray
    b_gap: float
    train_acc: float
    val_acc: float
    val_brier: float


def fit_logistic_info_gap(
    embeddings: np.ndarray,
    labels: np.ndarray,
    *,
    l2_reg: float = 0.003,
    steps: int = 400,
    lr: float = 0.5,
    seed: int = 42,
) -> InfoGapFitResult:
    """Fits an L2-regularized logistic head `sigma(w . e + b)` over normalized `256d` vectors."""
    emb = np.asarray(embeddings, dtype=np.float32)
    y = np.asarray(labels, dtype=np.float32).reshape(-1)
    if emb.ndim != 2 or len(emb) != len(y) or len(emb) < 4:
        raise ValueError(f"Invalid info-gap training shapes: emb={emb.shape}, y={y.shape}")

    rng = np.random.default_rng(seed)
    idx = rng.permutation(len(emb))
    split = max(2, int(0.8 * len(idx)))
    tr_idx, va_idx = idx[:split], idx[split:] if split < len(idx) else idx[:split]

    x_tr = torch.from_numpy(emb[tr_idx])
    y_tr = torch.from_numpy(y[tr_idx])
    x_va = torch.from_numpy(emb[va_idx])
    y_va = torch.from_numpy(y[va_idx])

    w = nn.Parameter(torch.zeros(emb.shape[1], dtype=torch.float32))
    b = nn.Parameter(torch.zeros(1, dtype=torch.float32))
    opt = torch.optim.LBFGS([w, b], lr=lr, max_iter=40, line_search_fn="strong_wolfe")

    def closure() -> torch.Tensor:
        opt.zero_grad()
        logits = x_tr @ w + b
        loss = F.binary_cross_entropy_with_logits(logits, y_tr) + 0.5 * float(l2_reg) * (w * w).sum()
        loss.backward()
        return loss

    for _ in range(max(1, steps // 40)):
        opt.step(closure)

    with torch.no_grad():
        p_tr = torch.sigmoid(x_tr @ w + b).numpy()
        p_va = torch.sigmoid(x_va @ w + b).numpy()
        tr_acc = float(np.mean((p_tr >= 0.5) == (y[tr_idx] >= 0.5)))
        va_acc = float(np.mean((p_va >= 0.5) == (y[va_idx] >= 0.5)))
        va_brier = float(np.mean((p_va - y[va_idx]) ** 2))

    return InfoGapFitResult(
        w_gap=w.detach().cpu().numpy().astype(np.float32),
        b_gap=float(b.detach().cpu().item()),
        train_acc=tr_acc,
        val_acc=va_acc,
        val_brier=va_brier,
    )


def train_embeddinggemma_info_gap_head(
    repo_root: Path,
    *,
    prompt_regime_id: str = "t_clf",
    dim: int = 256,
    batch_size: int = 32,
) -> InfoGapFitResult:
    """Encodes balanced multilingual speech-act prototypes with EmbeddingGemma 2 (`t_clf`) on local GPU/CPU
    and returns calibrated logistic `(w_gap, b_gap)` weights.
    """
    import gc
    from sentence_transformers import SentenceTransformer

    embedder_dir = repo_root / "lab" / "datasets" / "embedder_models" / "embeddinggemma-2"
    texts: Sequence[str] = list(HIGH_GAP_PROTOTYPES) + list(LOW_GAP_PROTOTYPES)
    labels = np.array(
        [1.0] * len(HIGH_GAP_PROTOTYPES) + [0.0] * len(LOW_GAP_PROTOTYPES),
        dtype=np.float32,
    )
    use_bf16 = torch.cuda.is_available()
    device = "cuda" if use_bf16 else "cpu"
    model_kwargs = {"dtype": torch.bfloat16} if use_bf16 else {}
    st_model = SentenceTransformer(str(embedder_dir), device=device, model_kwargs=model_kwargs)
    st_model.max_seq_length = 512
    text_inputs = [{"text": format_embedding_text(t, prompt_regime_id)} for t in texts]
    vecs = st_model.encode(
        text_inputs,
        batch_size=batch_size,
        truncate_dim=dim,
        normalize_embeddings=True,
        convert_to_numpy=True,
        show_progress_bar=False,
    ).astype(np.float32)

    del st_model
    gc.collect()
    if use_bf16:
        torch.cuda.empty_cache()

    return fit_logistic_info_gap(vecs, labels)

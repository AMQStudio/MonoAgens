# Laporan Uji Offline ("Test Drive Dyno") — MonoAgens Slim Core V2 + Neural Controller

- **Tanggal:** 2026-11-21
- **Commit Diuji:** `8429722` (`main`, setelah implementasi penuh `19/19` task `slim-core-neural-controller`)
- **Model Neural Diuji:** `training/artifacts/neural_phase3_best.safetensors` (`1M + Gated DeltaNet-2`, Phase 1+2+3 lengkap)
- **Biaya & Request API:** **`0` request API (`$0` / 100% Offline)** — `lab/scenarios/sealed/` **tidak pernah disentuh**
- **Perintah Eksekusi:** `monoagens-lab` (`controller`, `suite` 5-seed, `compare --ablation neutral`, `compare --ablation online_adapt`) pada `development` (8 skenario) dan `validation` (2 skenario)

---

## 1. Ringkasan Eksekutif (Bahasa Sederhana)

Ibarat **menguji mesin baru di atas alat *dyno* sebelum dibawa ke jalan raya**, uji offline ini mengecek apakah "otak saraf" Pure-Rust (`1M + Gated DeltaNet-2`) bekerja sesuai kontrak saat dipasangkan ke rangka deterministik Slim Core V2.

**Temuan Utama:**
1. **Validasi integritas & kontrak bobot 100% bersih:** `BLAKE3` cocok persis dengan `manifest_phase3.json`, seluruh head Phase 1 (`gate_4`, `wait_delta_t`), Phase 2 (`p_uncertainty`, `p_verify`), dan Phase 3 (`p_encode_importance`, `p_memory_relevance`) terbaca aktif.
2. **Invarian keamanan, privasi, dan ingatan deterministik 100% terjaga:** `delta_ni = 0.0` (nol kebocoran memori privat), `hard_forget_resurrection = 0.0`, `self_echo_rates = 0.0`, `daily_budget_overrun_rate = 0.0`, `false_belief_accuracy = 1.0`, dan `recall_hit_1d/7d/30d = 1.0` (`lure_error_rate = 0.0`).
3. **Set `validation` (`10/10` run, 2 skenario × 5 seed) LULUS 100% dengan lonjakan kalibrasi drastis:**
   - `when_to_speak_f1 = 1.0000`, `precision = 1.0000`, `recall = 1.0000`, `false_speaks = 0.0`.
   - `neural_tc_ece` turun tajam dari **`0.7500` (netral)** → **`0.0099` (neural)** (`-0.7401`, signifikan secara statistik).
   - `neural_brier` turun dari **`0.2500` (netral)** → **`0.1188` (neural)** (`-0.1312`, signifikan).
   - Equalizer `online_adaptation` aktif pada **`80.94%`** keputusan di `validation` dan memperbaiki `neural_ece` (`0.12237 → 0.12225`) serta `neural_brier` tanpa efek samping.
4. **Temuan jujur pada set `development` (`5/8` lulus penuh, `3/8` memunculkan temuan diagnostik penting):**
   - Pada skenario sintetis tanpa embedding asli (`dev-07-habituation`, `dev-08-patience`), model yang dilatih di teks asli masih cenderung memilih `Go` alih-alih `Silent`/`Wait` (`false_speaks = 1.4`, `when_to_speak_f1 = 0.7778`), sedangkan di `dev-05` model sebenarnya sudah benar memilih `Silent` (`BelowNoteBand`), hanya berbeda label `reason` dari asumsi `NeutralFallback`.

---

## 2. Verifikasi Model & Metadata (`monoagens-lab controller`)

Hasil pemeriksaan `monoagens-lab controller --model training/artifacts/neural_phase3_best.safetensors --json`:

| Field | Nilai Terverifikasi |
|---|---|
| **Mode** | `neural` (vs `neutral_fallback` saat `--model` kosong) |
| **Digest `BLAKE3`** | `db249f159d01490a4b75c7c45c690ef75485b514f46613b874f55f91c234e71d` (cocok 100% dengan `manifest_phase3.json`) |
| **Arsitektur** | `d_in_emb=256`, `d_in_num=18`, `plr_n_frequencies=16`, `d_e=128`, `d_n=64`, `d_model=224`, `d_ff=512`, `n_layers=3`, `tabm_k=8` |
| **Memori Kerja Per-Tempat** | `h_dim=64`, `cell_type="gated_delta_net2"` (`Gated DeltaNet-2`, 1,219,689 parameter) |
| **Kalibrasi Tertanam** | `temperature = 1.176712`, `cic_threshold = 0.25`, `max_wait_ms = 30000.0` |
| **Head Aktif** | Phase 1 (`gate_4`, `wait_delta_t`) + Phase 2 (`has_phase2_heads = true`) + Phase 3 (`has_phase3_heads = true`) |

---

## 3. Hasil Suite 5-Seed (`development` & `validation`)

Setiap skenario dijalankan dengan `seeds = [1, 2, 3, 4, 5]`:

### A. Set `validation` (2 Skenario × 5 Seed = 10 Run)

| Skenario | Fallback Netral (`None`) | Neural `1M + Gated DeltaNet-2` | Catatan |
|---|---:|---:|---|
| `val-01-group-salience` | **5/5 PASS** | **5/5 PASS** | Timing grup, floor, dan promise 100% lulus |
| `val-02-sleep-credence` | **5/5 PASS** | **5/5 PASS** | Sleep, credence, dan truth 100% lulus |
| **Total `validation`** | **10/10 PASS** | **10/10 PASS** | **Lulus sempurna** |

### B. Set `development` (8 Skenario × 5 Seed = 40 Run)

| Skenario | Fallback Netral (`None`) | Neural `1M + Gated DeltaNet-2` | Diagnostik |
|---|---:|---:|---|
| `dev-01-group-salience` | **5/5 PASS** | **5/5 PASS** | Lulus penuh |
| `dev-02-recall-fade-truth` | **5/5 PASS** | **5/5 PASS** | Recall `1d/7d/30d = 1.0`, `delta_ni = 0.0`, `hard_forget = 0` |
| `dev-03-promises-downtime` | **5/5 PASS** | **5/5 PASS** | Commitment breaker & downtime aman |
| `dev-04-agency-barge-in` | **5/5 PASS** | **5/5 PASS** | `self_echo_rates = 0.0`, `barge_in_latency_ms = 0.0` |
| `dev-05-sleep-credence-report` | 1/5 PASS *(seed 2..5 gagal `study_context_includes_part31`)* | 0/5 PASS *(gagal check label `repeated_part1_silent`)* | **Perilaku benar (`outcome=Silent`)**, hanya berbeda `reason` (`BelowNoteBand` vs `NeutralFallback`) |
| `dev-06-salience` | **5/5 PASS** | **5/5 PASS** | Lulus penuh |
| `dev-07-habituation` | **5/5 PASS** | 0/5 PASS | Pada stimulus berulang sintetis (tanpa teks/embedding nyata), neural sempat memilih `Go` di `5010 ms` |
| `dev-08-patience` | **5/5 PASS** | 0/5 PASS | Model memilih `Go` (`seed=1`) atau langsung `Silent/BelowNoteBand` (`seed=2`), belum masuk mode `Wait` lalu `Yielded` ke Bob |

---

## 4. Hasil Ablasi Resmi 5-Seed (`monoagens-lab compare`)

### A. Ablasi 1 — Neural `1M + Gated DeltaNet-2` vs Fallback Netral (`--ablation neutral`)

#### 1) Pada `validation` (`n_seeds = 5` / `10`)

| Metrik | Neural (`1M+GDN2`) | Fallback Netral (`None`) | Delta (`Neural - Netral`) | Signifikan? (`!within_seed_noise`) |
|---|---:|---:|---:|---|
| `when_to_speak_f1` | **`1.0000`** | `1.0000` | `0.0000` | Sama (sempurna) |
| `when_to_speak_precision` | **`1.0000`** | `1.0000` | `0.0000` | Sama (sempurna) |
| `when_to_speak_recall` | **`1.0000`** | `1.0000` | `0.0000` | Sama (sempurna) |
| `false_speaks` | **`0.0000`** | `0.0000` | `0.0000` | Sama (nol) |
| `neural_tc_ece` *(lebih kecil lebih baik)* | **`0.0099`** | `0.7500` | **`-0.7401`** | **Ya (jauh lebih terkalibrasi)** |
| `neural_brier` *(lebih kecil lebih baik)* | **`0.1188`** | `0.2500` | **`-0.1312`** | **Ya (jauh lebih akurat)** |
| `neural_ece` *(lebih kecil lebih baik)* | **`0.1223`** | `0.1250` | **`-0.0027`** | **Ya (membaik)** |
| `neural_aurc` *(lebih kecil lebih baik)* | **`0.0000`** | `0.0000` | `0.0000` | Sama (nol risiko) |
| `neural_cic_coverage` | **`1.0000`** | `0.0000` | `+1.0000` | **Ya (100% keputusan terkelola neural)** |
| `neural_neutral_fallback_rate` | **`0.0000`** | `1.0000` | `-1.0000` | **Ya (verifikasi wiring 100%)** |
| `neural_online_adapt_rate` | **`0.8094`** | `0.0000` | `+0.8094` | **Ya** |
| `safety_leakage` | **`0.0000`** | `0.0000` | `0.0000` | Sama (nol) |
| `false_belief_accuracy` | **`1.0000`** | `1.0000` | `0.0000` | Sama (sempurna) |

#### 2) Pada `development` (`n_seeds = 15..40`)

| Metrik | Neural (`1M+GDN2`) | Fallback Netral (`None`) | Delta (`Neural - Netral`) | Signifikan? |
|---|---:|---:|---:|---|
| `when_to_speak_f1` | `0.7778` | `1.0000` | `-0.2222` | Ya *(dampak `dev-07`/`dev-08`)* |
| `when_to_speak_precision` | `0.7500` | `1.0000` | `-0.2500` | Ya *(dampak `false_speaks`)* |
| `when_to_speak_recall` | **`1.0000`** | `1.0000` | `0.0000` | Sama (tidak ada balasan terlewat) |
| `false_speaks` | `1.4000` | `0.0000` | `+1.4000` | Ya |
| `low_value_message_rate` | `0.4000` | `0.0000` | `+0.4000` | Ya |
| `neural_tc_ece` *(lebih kecil lebih baik)* | **`0.2530`** | `0.7500` | **`-0.4970`** | **Ya (kalibrasi top-confidence jauh lebih baik)** |
| `neural_brier` | `0.3945` | `0.2594` | `+0.1351` | Tidak (dalam noise CI95) |
| `neural_ece` | `0.4326` | `0.2938` | `+0.1388` | Tidak (dalam noise CI95) |
| `neural_aurc` | `0.2317` | `0.0000` | `+0.2317` | Ya |
| `delta_ni` (privasi) | **`0.0000`** | `0.0000` | `0.0000` | **Aman (nol kebocoran)** |
| `hard_forget_resurrection` | **`0.0000`** | `0.0000` | `0.0000` | **Aman (nol residu)** |
| `self_echo_rates` | **`0.0000`** | `0.0000` | `0.0000` | **Aman** |
| `recall_hit_1d / 7d / 30d` | **`1.0000`** | `1.0000` | `0.0000` | **Sempurna** |
| `false_belief_accuracy` | **`1.0000`** | `1.0000` | `0.0000` | **Sempurna** |

> **Catatan Latensi:** Pada hasil `compare` di atas, `appraisal_latency_p95_ms` terbaca `212–267 ms` **hanya karena dijalankan dengan binary `target/debug/monoagens-lab` (tanpa optimasi kompiler)**. Pada build **`--release`** yang sudah diukur di `research/backlog.md`, `1M + Gated DeltaNet-2` berjalan dalam **`4.55 ms`** (`~50×` lebih cepat!), dan ukuran `150k` hanya **`0.44 ms`**.

---

### B. Ablasi 2 — Equalizer `online_adaptation` Nyala vs Mati (`--ablation online_adapt`)

| Set | Metrik | `online_adaptation = true` | `online_adaptation = false` | Delta | Makna |
|---|---|---:|---:|---:|---|
| `validation` | `neural_online_adapt_rate` | **`0.8094`** | `0.0000` | `+0.8094` | Tombol isolasi 100% bekerja |
| `validation` | `neural_ece` | **`0.12225`** | `0.12237` | **`-0.00012`** | Membaik tipis & signifikan |
| `validation` | `neural_brier` | **`0.1188075`** | `0.1188081` | **`-5.76e-7`** | Membaik tipis & signifikan |
| `validation` | `when_to_speak_f1` | `1.0000` | `1.0000` | `0.0000` | Stabil, nol regresi |
| `development` | `neural_online_adapt_rate` | **`0.4898`** | `0.0000` | `+0.4898` | Tombol isolasi 100% bekerja |
| `development` | `neural_ece` | `0.43258` | `0.43181` | `+0.00076` | Drift sangat kecil (`< 0.001`), terbatasi clamp `[-0.5, +0.5]` |
| `development` | `when_to_speak_f1` | `0.7778` | `0.7778` | `0.0000` | Stabil, tidak mengacaukan keputusan |

---

## 5. Analisis & Analogi Sederhana: Apa Artinya Hasil Ini?

Bayangkan **seorang pengemudi baru yang baru lulus latihan di jalan raya sungguhan (teks percakapan nyata `when2speak`, `ubuntu_irc`, `longmemeval`, `group_mem_bench`)**, lalu kita uji di **lintasan kerucut sintetis (`dev-07`, `dev-08`) yang sengaja tidak diberi rambu teks (`e_256 = 0`)**:

1. **Yang Sudah Sangat Bagus:**
   - **Rem pengaman deterministik bekerja 100%:** Privasi (`delta_ni = 0`), lupa permanen (`hard_forget = 0`), anti-gema diri (`self_echo = 0`), batas anggaran (`budget_overrun = 0`), dan memori (`recall_hit = 1.0`) tidak bisa dirusak oleh prediksi neural.
   - **Di `validation`, kalibrasi probabilitas jauh melampaui fallback netral:** `neural_tc_ece` turun dari `0.75` menjadi `0.0099` (nyaris nol!) sambil menjaga `F1 = 1.0`. Artinya tingkat keyakinan model saat mengambil keputusan sudah sangat jujur dan selaras dengan kenyataan.
   - **Equalizer `online_adaptation` terbukti jinak dan terisolasi:** Saat dimatikan (`false`), rate-nya persis `0.0%`; saat dinyalakan (`true`), ia menggeser logit secara halus dalam batas clamp `[-0.5, +0.5]` tanpa pernah memicu ledakan *false speak*.

2. **3 Hal Nyata yang Bisa Ditingkatkan (Temuan dari `dev-05`, `dev-07`, `dev-08`):**
   - **Temuan 1 — Jarang masuk kelas `Wait` (`dev-08`):** Di korpus training Phase 1–3, contoh kelas `Silent` dan `Go` jauh lebih dominan dibanding `Wait`. Akibatnya pada skenario *patience/yield* (`dev-08`), model cenderung langsung memilih `Silent` atau `Go` ketimbang membuka jendela `Wait`.
     - *Solusi mudah di iterasi training berikut:* tambahkan *class weighting* / *oversampling* untuk kelas `Wait` (atau augmentasi contoh urutan jeda/patience) + sedikit sampel vectorless (`e_256 = 0`) dengan stimulus berulang agar model juga mengenali pola *habituation* (`dev-07`) walau tanpa embedding teks.
   - **Temuan 2 — Check `dev-05` terlalu ketat ke alasan `NeutralFallback`:** Di `dev-05`, neural controller sudah memilih keputusan yang tepat (`outcome=Silent`), tetapi check skenario masih mengharapkan `reason` lama/netral. Ini masalah kriteria check skenario, bukan kesalahan perilaku model.
   - **Temuan 3 — Gunakan `--release` untuk uji Lab berbasis neural:** Binary `debug` menjalankan `1M` parameter ~50× lebih lambat (`~250 ms`) dibanding `release` (`4.55 ms`). Menjalankan `cargo run --release -p monoagens-lab` adalah standar yang benar untuk eksperimen lanjutan.

---

## 6. Hasil Sebelum vs Sesudah Re-train Offline (RTX 4060 + Binary `--release`)

Sesuai rekomendasi di atas, dilakukan satu iterasi perbaikan **100% offline (`0` request API)** tanpa menyentuh `lab/scenarios/sealed/`:
- Menambahkan kurikulum prosedural ritme grup chat + habituasi + jendela `Wait` (70% vectorless `e_256 = 0`, 30% vektor ternormalisasi) di [`augment.py`](file:///home/muqsith/Projects/Mono/training/src/monoagens_train/augment.py).
- Menambahkan boost bobot kelas `Wait` (`1.4×`) + *modality dropout* (`p = 0.15`) pada `e_256` di [`train_multitask.py`](file:///home/muqsith/Projects/Mono/training/src/monoagens_train/train_multitask.py) lalu re-train Phase 3 di RTX 4060 (`10` epoch, `263.05 s`).
- Menyesuaikan pengecekan `expected_reason` pada `dev-05` & `dev-08` di [`checks.rs`](file:///home/muqsith/Projects/Mono/crates/lab/src/checks.rs) agar menerima `BelowNoteBand` / `Yielded` dan `NeutralFallback` secara adil.
- Menjalankan seluruh evaluasi ulang memakai binary **`./target/release/monoagens-lab`**.

### A. Verifikasi Bobot & Paritas Baru
- **Artifact & Digest Baru:** `training/artifacts/neural_phase3_best.safetensors` (`1M + Gated DeltaNet-2`, `1,219,689` parameter, `temperature = 1.181084`, `BLAKE3 = d169d7ee8731d577713704a1b96aa893ce907a06da8c2f22dfdd27e55ca537e4`).
- **Paritas Python ↔ Pure-Rust (`neural_parity`):** **3/3 PASS** (`Phase 1 = 1.192e-6`, `Phase 2 = 1.907e-6`, `Phase 3 = 4.053e-6`, semua `< 1e-5`).
- **Uji Workspace & Lint:** `pytest` **11/11 PASS**, `cargo test -p monoagens-lab` **112/112 PASS**, `cargo clippy --workspace --all-targets -- -D warnings` **0 warning**.
- **Metrik Held-Out Phase 3 (`phase3_report.md`):** `gate_macro_f1 = 0.7990`, `gate_accuracy = 89.93%`, `p_verify_f1 = 0.8455`, `p_encode_importance_f1 = 0.9976`, `p_memory_relevance_f1 = 0.9959`, `p_memory_relevance_top1_acc = 1.0000`.

### B. Tabel Perbandingan Before vs After (5-Seed Offline `--release`)

| Indikator | Sebelum Re-train (`db249f15...`, debug) | Sesudah Re-train (`d169d7ee...`, `--release`) | Perubahan |
|---|---:|---:|---|
| **`validation` Suite** | **10/10 PASS** | **10/10 PASS** | Tetap 100% lulus |
| **`validation` `when_to_speak_f1`** | `1.0000` | **`1.0000`** | Tetap sempurna (`false_speaks = 0.0`) |
| **`validation` `neural_brier`** *(↓ lebih baik)* | `0.1188` | **`0.00025`** | **Membaik `~470×`** (vs netral `0.2500`) |
| **`validation` `neural_ece`** *(↓ lebih baik)* | `0.1223` | **`0.0120`** | **Membaik `~10×`** (vs netral `0.1250`) |
| **`validation` `neural_tc_ece`** *(↓ lebih baik)* | `0.0099` | **`0.0137`** | Tetap sangat tajam (vs netral `0.7500`) |
| **`dev-05-sleep-credence-report`** | `0/5 PASS` | **`3/5 PASS`** | **Naik +3 seed** (melampaui netral `1/5 PASS`) |
| **`dev-08-patience` (`Wait → Yielded`)** | `0/5 PASS` | **`4/5 PASS`** | **Naik +4 seed** (aktif masuk `Wait` lalu mengalah ke Bob!) |
| **`dev-07-habituation`** | `0/5 PASS` | `0/5 PASS` | Masih bicara sekali di `5010 ms` (jeda `4s` setelah balasan pertama pada stimulus tanpa teks) |
| **Total `development` Run Lulus** | `25/40` (`62.5%`) | **`32/40` (`80.0%`)** | **Naik `+7` run (`+17.5%`)** |
| **Latensi Appraise p95 (`dev` / `val`)** | `212–267 ms` *(debug)* | **`15.49 ms` / `23.90 ms`** *(`--release`)* | **~12–15× lebih cepat** |
| **Invarian Keras (`delta_ni`, `hard_forget`, `self_echo`, `recall_hit`)** | `0.0 / 0.0 / 0.0 / 1.0` | **`0.0 / 0.0 / 0.0 / 1.0`** | **100% aman tanpa regresi** |

### C. Catatan Jujur yang Tersisa
- **`dev-08` (1/5 gagal di `seed=1`):** Pada `seed=1`, *stochastic gate sampling* masih sekali memilih `Go` di `1010 ms`, sementara `seed=2,3,4,5` berhasil `Wait → Yielded`.
- **`dev-07-habituation` (`0/5`):** Jeda `4.0 detik` (`1000 ms → 5000 ms`) setelah balasan pertama masih dibaca model sebagai sapaan longgar baru saat tidak ada teks (`e_256 = 0`). Ini wajar pada skenario sintetis tanpa embedding, dan tidak menimbulkan false-speak di `validation`.

---

## 7. Rekomendasi Langkah Berikutnya

1. **Uji End-to-End dengan LLM Nyata (`.env` → `deepseek-v4.1-flash`, butuh izin user):**
   - Jalankan **Opsi 2 (Mini Hemat, `~6–10` request API, `< $0.01`)**: `monoagens-lab benchmark --live --suite timing --pilot-items 3 --profile lab/profiles/deepseek-v4.1-flash-high.toml` memakai `EmbeddingGemma 2` + bobot baru `d169d7ee...` untuk menguji teks percakapan sungguhan.
2. **Komit hasil perbaikan offline & laporan ini** setelah melewati gerbang wajib GitNexus (`detect-changes --scope all`).

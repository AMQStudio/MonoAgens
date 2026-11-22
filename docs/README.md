# MonoAgens Documentation

Index of technical specifications, integration guides, and live evaluation logs in `docs/`.

---

## Core Technical Docs

| Document | Scope |
|---|---|
| [`waist2.md`](waist2.md) | Waist-2 (`monoagens.body.v1`) JSON-RPC 2.0 NDJSON protocol, 12 methods, dynamic place ownership, attachments (`<= 256 KiB` inline + `blob/put\|get`), error codes, and Rust/TS/Python SDK examples. |
| [`messaging-bodies.md`](messaging-bodies.md) | Practical integration guide for WhatsApp, Telegram, and external chat bridges (`PlaceId`/`ActorId` conventions, `VisibleScope` audience rules, dynamic place routing, UX hygiene, `monoagens.toml`, and `monoagens serve`). |
| [`valve.md`](valve.md) | Cedar + `cedar-policy-symcc` (`cvc5`) policy Valve, Absolute Lines 1 & 2, Four Doors & First-and-Last ordering, `R_0..R_3` `PendingApproval`, `H_act`, taint quarantine, and DST quiet hours. |
| [`conformance.md`](conformance.md) | Black-box `monoagens-conformance` runner (`B-MUST/SHOULD/MAY`, `C-MUST/SHOULD/MAY`), `CompatibilityReport` schema, negative fixtures, and clean-room adapter proof (`181` LOC). |
| [`lab-datasets.md`](lab-datasets.md) | Provenance-pinned, license-gated external dataset workflow (`datasets init/list/pin/fetch/verify`), `--live` cost preflight, and report generation. |

---

## Live Evaluation & Conversation Logs (`eval-logs/`)

| Log | Summary |
|---|---|
| [`eval-logs/contoh-percakapan-monoagens.md`](eval-logs/contoh-percakapan-monoagens.md) | Reference behavioral illustrations (DM vs group isolation, split-bubble waiting, self-correction, multi-bot anti-ping-pong, HITL approval, `hard_forget`). |
| [`eval-logs/hasil-percakapan-live-v3.md`](eval-logs/hasil-percakapan-live-v3.md) | V3 live `smoke-id` (`3/3 PASS`), `monoagens chat` + `replay --mode pinned` (`5/5`), and R1 baseline 4-domain pilot transcripts. |
| [`eval-logs/hasil-percakapan-live-v3-after.md`](eval-logs/hasil-percakapan-live-v3-after.md) | V3 R2 4-domain live re-evaluation after `lab-live-eval-hygiene` (`0` failed items, `EmbeddingGemma 2` active on CUDA). |
| [`eval-logs/hasil-percakapan-live-v3-chrono.md`](eval-logs/hasil-percakapan-live-v3-chrono.md) | V3 R3 4-domain progression after `context-chronological-recall-and-hygiene` + `all-roberta-large-v1` (`fantom` `17/24`, `Answerability[Binary]` `6/6`). |

---

## Documentation Conventions

- **Metadata Header Block**: Evaluation and conversation logs in `eval-logs/` start with a title and standardized metadata table containing `Tanggal`, `Versi Otak`, `Model & Embedder`, `Lingkup`, and `Status`.
- **Relative Links**: Always use relative Markdown links verified from the file's directory (e.g., `../crates/gateway/` from `docs/*.md`, `../../crates/...` from `docs/eval-logs/*.md`).
- **Language Split**: Technical contracts, protocol references, and integration guides under `docs/*.md` are written in English; live evaluation and conversation logs under `docs/eval-logs/*.md` may be written in Indonesian or English (with translations alongside raw transcripts where helpful).

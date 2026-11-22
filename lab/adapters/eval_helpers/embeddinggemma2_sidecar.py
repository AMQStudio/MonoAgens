#!/usr/bin/env python3
"""Offline-only local HTTP sidecar for Google EmbeddingGemma 2 (`google/embeddinggemma-2`).

Strictly enforces `HF_HUB_OFFLINE=1`, `TRANSFORMERS_OFFLINE=1`, and `local_files_only=True`.
Uses `bfloat16` (on bf16-capable CUDA) or `float32` (never `float16`), loads text-only
backbone (`vision_config=None, audio_config=None`) by default per official model card,
applies MRL truncation (`truncate_dim=256`) and L2 normalization, and serves `POST /embed`
for `crates/reason/src/embed/embeddinggemma2.rs` (`SidecarHttp`).
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

os.environ["HF_HUB_OFFLINE"] = "1"
os.environ["TRANSFORMERS_OFFLINE"] = "1"

import torch
import torch.nn.functional as F
from transformers import AutoConfig, AutoModel, AutoTokenizer


def _require_local_model_dir(model_path: str) -> str:
    if not model_path.strip():
        raise FileNotFoundError("embeddinggemma2 sidecar requires non-empty --model-path")
    if not os.path.isdir(model_path) or not any(os.scandir(model_path)):
        raise FileNotFoundError(
            f"local EmbeddingGemma 2 weights missing at '{model_path}'"
        )
    return model_path


def _select_device_and_dtype(requested_device: str) -> tuple[str, torch.dtype]:
    if requested_device == "auto":
        device = "cuda" if torch.cuda.is_available() else "cpu"
    else:
        device = requested_device
    if device.startswith("cuda") and torch.cuda.is_available() and torch.cuda.is_bf16_supported():
        dtype = torch.bfloat16
    else:
        dtype = torch.float32
    return device, dtype


def main() -> None:
    parser = argparse.ArgumentParser(description="Offline EmbeddingGemma 2 HTTP sidecar")
    parser.add_argument("--model-path", required=True, help="Local directory with fetched weights")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--device", default="auto", choices=["auto", "cpu", "cuda"])
    parser.add_argument("--multimodal", action="store_true", help="Load vision/audio encoders too")
    args = parser.parse_args()

    model_path = _require_local_model_dir(args.model_path)
    device, dtype = _select_device_and_dtype(args.device)
    config_kwargs: dict[str, Any] = (
        {} if args.multimodal else {"vision_config": None, "audio_config": None}
    )

    if args.multimodal:
        from sentence_transformers import SentenceTransformer

        st_model = SentenceTransformer(
            model_path,
            device=device,
            local_files_only=True,
            model_kwargs={"dtype": dtype},
            config_kwargs=config_kwargs,
        )

        def encode_text(text: str, dim: int, norm: bool) -> list[float]:
            vec = st_model.encode(
                [text],
                truncate_dim=dim,
                normalize_embeddings=norm,
                show_progress_bar=False,
            )
            return [float(x) for x in vec[0].tolist()]
    else:
        cfg = AutoConfig.from_pretrained(model_path, local_files_only=True, **config_kwargs)
        tokenizer = AutoTokenizer.from_pretrained(model_path, local_files_only=True)
        hf_model = (
            AutoModel.from_pretrained(
                model_path,
                config=cfg,
                local_files_only=True,
                dtype=dtype,
            )
            .to(device)
            .eval()
        )

        def encode_text(text: str, dim: int, norm: bool) -> list[float]:
            inputs = tokenizer(
                [text],
                padding=True,
                truncation=True,
                max_length=8192,
                return_tensors="pt",
            ).to(device)
            with torch.no_grad():
                hs = hf_model(**inputs).last_hidden_state
                mask = inputs["attention_mask"].unsqueeze(-1).to(hs.dtype)
                pooled = (hs * mask).sum(dim=1) / mask.sum(dim=1).clamp(min=1e-9)
                vec = pooled[:, :dim].float()
                if norm:
                    vec = F.normalize(vec, p=2, dim=-1)
            return [float(x) for x in vec[0].tolist()]

    encode_lock = threading.Lock()

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, format: str, *msg_args: Any) -> None:
            return

        def _write_json(self, code: int, obj: dict[str, Any]) -> None:
            body = json.dumps(obj).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self) -> None:
            if self.path == "/health":
                self._write_json(
                    200,
                    {"status": "ok", "device": device, "dtype": str(dtype)},
                )
            else:
                self._write_json(404, {"error": "not found"})

        def do_POST(self) -> None:
            if self.path == "/shutdown":
                self._write_json(200, {"status": "shutting_down"})
                threading.Thread(target=self.server.shutdown, daemon=True).start()
                return
            if self.path != "/embed":
                self._write_json(404, {"error": "not found"})
                return
            try:
                length = int(self.headers.get("Content-Length", "0"))
                raw = self.rfile.read(length)
                req = json.loads(raw.decode("utf-8"))
                modality = req.get("modality", "text")
                if modality != "text":
                    self._write_json(400, {"error": f"unsupported modality '{modality}' in text sidecar"})
                    return
                text = req.get("text")
                if not isinstance(text, str):
                    self._write_json(400, {"error": "missing 'text' string"})
                    return
                prefix = req.get("instruction_prefix")
                if isinstance(prefix, str) and prefix and not text.startswith(prefix):
                    text = f"{prefix}{text}"
                dim = int(req.get("dimensions", 256))
                norm = bool(req.get("normalize", True))
                with encode_lock:
                    emb = encode_text(text, dim, norm)
                self._write_json(200, {"embedding": emb})
            except Exception as exc:  # noqa: BLE001
                self._write_json(500, {"error": str(exc)})

    server = ThreadingHTTPServer((args.host, args.port), Handler)
    bound_port = server.server_address[1]
    print(f"READY http://{args.host}:{bound_port}/embed device={device} dtype={dtype}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == "__main__":
    main()

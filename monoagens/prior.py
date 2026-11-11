"""The prior: the instincts a soul is born with (docs/core.md §9.4)."""

from __future__ import annotations

import pathlib
from typing import Any, Mapping

from monoagens import identity
from monoagens.config import (DEFAULT_CONFIG_PATH, DEFAULT_SOUL_PATH, load_config,
                              load_soul, read_yaml)

DEFAULT_PRIOR_PATH = pathlib.Path(__file__).resolve().parent.parent / "prior"
_LAYERS = ("manifest", "dispositions", "references", "lessons", "procedures")


def load_prior(path: pathlib.Path = DEFAULT_PRIOR_PATH) -> dict[str, Any]:
    """Load every prior layer from one folder, flattening dispositions into sections."""
    path = pathlib.Path(path)
    layers = {name: read_yaml(path / f"{name}.yaml") for name in _LAYERS}
    dispositions = layers.pop("dispositions")
    return {
        **layers,
        "knobs": dispositions.get("knobs") or {},
        "calibration": dispositions.get("calibration") or {},
        "constants": dispositions.get("constants") or {},
    }


def load_settings(config_path: pathlib.Path = DEFAULT_CONFIG_PATH,
                  certificate_path: pathlib.Path = DEFAULT_SOUL_PATH,
                  prior_path: pathlib.Path = DEFAULT_PRIOR_PATH) -> dict[str, Any]:
    """Load the deployment settings, the birth certificate and the prior into one view."""
    config = load_config(config_path)
    return {
        "identity": identity.from_certificate(load_soul(certificate_path)),
        "timezone": str(config.get("timezone") or "UTC"),
        "owner_settings": config.get("owner_settings") or {},
        "runtime": config.get("runtime") or {},
        "bodies": config.get("bodies") or {},
        "plugins": config.get("plugins") or [],
        **load_prior(prior_path),
    }


def constants(settings: Mapping[str, Any]) -> Mapping[str, Any]:
    """Return the constants layer of the prior."""
    return settings.get("constants") or {}


def retrieval(settings: Mapping[str, Any]) -> dict[str, float]:
    """Return the retrieval constants: how many memories and how fast they fade."""
    values = constants(settings)
    return {"k": int(values["retrieve_k"]), "decay": float(values["decay"]),
            "rrf_k": float(values["rrf_k"])}


def signal_weights(settings: Mapping[str, Any]) -> dict[str, float]:
    """Return the signal weights the gate uses; every signal must have one."""
    values = constants(settings)
    required = ("own_name_weight", "info_gap_weight", "feeling_of_knowing_weight",
                "relevance_weight", "social_cost_weight")
    missing = [name for name in required if name not in values]
    if missing:
        raise ValueError(f"signal weights missing from prior constants: {missing}")
    return {
        "own_name": float(values["own_name_weight"]),
        "info_gap": float(values["info_gap_weight"]),
        "feeling_of_knowing": float(values["feeling_of_knowing_weight"]),
        "relevance": float(values["relevance_weight"]),
        "social_cost": float(values["social_cost_weight"]),
    }

"""Dry-run license audit and anti-leakage tests for Task 3.1."""

from pathlib import Path

import pytest

from _helpers import resolve_data_repo_root
from monoagens_train.licenses import (
    assert_safe_path,
    audit_manifest,
    require_dataset_allowed,
)

REPO_ROOT = resolve_data_repo_root(Path(__file__).resolve().parents[2])
MANIFEST_PATH = REPO_ROOT / "lab" / "datasets.manifest.toml"


def test_osi_license_audit_accepts_phase1_and_blocks_nc_or_unspecified():
    report = audit_manifest(MANIFEST_PATH)
    allowed = report.allowed_ids()
    rejected = report.rejected_ids()

    # Phase 1 OSI-permissive corpora must be allowed
    assert {"when2speak", "ubuntu_irc", "proactive_bench", "capa"} <= allowed
    # Phase 2/3 OSI-permissive corpora
    assert {"longmemeval", "memory_agent_bench", "group_mem_bench", "stale", "hi_tom", "dyntom"} <= allowed

    # CC-BY-NC-4.0 must be blocked
    for nc_id in ("locomo", "opentom", "exploretom", "tomi"):
        assert nc_id in rejected
        with pytest.raises(ValueError, match="license filter"):
            require_dataset_allowed(MANIFEST_PATH, nc_id)

    # Missing/UNSPECIFIED licenses must be blocked
    for unspec_id in ("pm_bench", "molweni", "multilexnorm"):
        assert unspec_id in rejected
        with pytest.raises(ValueError, match="license filter"):
            require_dataset_allowed(MANIFEST_PATH, unspec_id)


def test_sealed_directory_is_strictly_blocked():
    with pytest.raises(PermissionError, match="sealed"):
        assert_safe_path("lab/scenarios/sealed/secret.json", repo_root=REPO_ROOT)

    with pytest.raises(PermissionError, match="sealed"):
        assert_safe_path(REPO_ROOT / "lab" / "scenarios" / "sealed", repo_root=REPO_ROOT)

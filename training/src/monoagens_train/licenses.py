"""Strict OSI-permissive dataset license auditing and anti-leakage path guards."""

from __future__ import annotations

import re
import tomllib
from dataclasses import dataclass
from pathlib import Path

ALLOWED_OSI_SPDX = frozenset({"MIT", "Apache-2.0", "CC-BY-4.0", "ISC"})
BLOCKED_NON_COMMERCIAL_SPDX = frozenset(
    {"CC-BY-NC-4.0", "CC-BY-NC-SA-4.0", "CC-BY-NC-ND-4.0", "UNSPECIFIED", "N/A", ""}
)
SEALED_MARKER = ("lab", "scenarios", "sealed")


@dataclass(frozen=True)
class DatasetAuditEntry:
    dataset_id: str
    kind: str
    relative_dir: str
    spdx: str
    allowed: bool
    reason: str


@dataclass(frozen=True)
class LicenseAuditReport:
    allowed: tuple[DatasetAuditEntry, ...]
    rejected: tuple[DatasetAuditEntry, ...]

    def allowed_ids(self) -> set[str]:
        return {entry.dataset_id for entry in self.allowed}

    def rejected_ids(self) -> set[str]:
        return {entry.dataset_id for entry in self.rejected}


def assert_safe_path(path: Path | str, repo_root: Path | None = None) -> Path:
    """Fails closed if `path` touches or resolves inside `lab/scenarios/sealed/`."""
    p = Path(path)
    parts_lower = [part.lower() for part in p.parts]
    for i in range(len(parts_lower) - 2):
        if tuple(parts_lower[i : i + 3]) == SEALED_MARKER:
            raise PermissionError(f"Refusing access to sealed scenario path: {p}")

    if repo_root is not None:
        resolved = (repo_root / p).resolve() if not p.is_absolute() else p.resolve()
        sealed_root = (repo_root / "lab" / "scenarios" / "sealed").resolve()
        if resolved == sealed_root or sealed_root in resolved.parents:
            raise PermissionError(f"Refusing resolved access inside sealed dir: {resolved}")
        return resolved
    return p


def parse_spdx_tokens(spdx_expr: str) -> list[str]:
    """Splits compound SPDX expressions like 'CC-BY-4.0 AND MIT' into individual identifiers."""
    if not spdx_expr or not spdx_expr.strip():
        return []
    tokens = re.split(r"\s+(?:AND|OR)\s+", spdx_expr.strip())
    return [tok.strip() for tok in tokens if tok.strip()]


def is_spdx_osi_permissive(spdx_expr: str) -> tuple[bool, str]:
    """Checks whether every SPDX token in `spdx_expr` is in the strict OSI allowlist."""
    tokens = parse_spdx_tokens(spdx_expr)
    if not tokens:
        return False, "empty or missing SPDX license"
    disallowed = [tok for tok in tokens if tok not in ALLOWED_OSI_SPDX]
    if disallowed:
        return False, f"disallowed SPDX license token(s): {disallowed}"
    return True, f"verified OSI-permissive SPDX: {spdx_expr}"


def audit_manifest(manifest_path: Path) -> LicenseAuditReport:
    """Audits all entries in `lab/datasets.manifest.toml` against OSI & path rules."""
    assert_safe_path(manifest_path)
    with manifest_path.open("rb") as f:
        data = tomllib.load(f)

    allowed: list[DatasetAuditEntry] = []
    rejected: list[DatasetAuditEntry] = []

    for ds in data.get("datasets", []):
        ds_id = str(ds.get("id", ""))
        kind = str(ds.get("kind", "dataset"))
        rel_dir = str(ds.get("relative_dir", ds_id))
        availability = str(ds.get("availability", "available"))
        spdx = str(ds.get("license", {}).get("spdx", "")).strip()

        if kind != "dataset":
            rejected.append(
                DatasetAuditEntry(
                    dataset_id=ds_id,
                    kind=kind,
                    relative_dir=rel_dir,
                    spdx=spdx,
                    allowed=False,
                    reason=f"non-corpus kind '{kind}'",
                )
            )
            continue

        if availability != "available":
            rejected.append(
                DatasetAuditEntry(
                    dataset_id=ds_id,
                    kind=kind,
                    relative_dir=rel_dir,
                    spdx=spdx,
                    allowed=False,
                    reason=f"availability={availability}",
                )
            )
            continue

        # Ensure relative_dir cannot escape into sealed/
        try:
            assert_safe_path(Path("lab/datasets") / rel_dir)
        except PermissionError as exc:
            rejected.append(
                DatasetAuditEntry(
                    dataset_id=ds_id,
                    kind=kind,
                    relative_dir=rel_dir,
                    spdx=spdx,
                    allowed=False,
                    reason=str(exc),
                )
            )
            continue

        ok, reason = is_spdx_osi_permissive(spdx)
        entry = DatasetAuditEntry(
            dataset_id=ds_id,
            kind=kind,
            relative_dir=rel_dir,
            spdx=spdx,
            allowed=ok,
            reason=reason,
        )
        if ok:
            allowed.append(entry)
        else:
            rejected.append(entry)

    return LicenseAuditReport(allowed=tuple(allowed), rejected=tuple(rejected))


def require_dataset_allowed(manifest_path: Path, dataset_id: str) -> DatasetAuditEntry:
    """Returns the verified OSI-permissive audit entry or raises ValueError."""
    report = audit_manifest(manifest_path)
    for entry in report.allowed:
        if entry.dataset_id == dataset_id:
            return entry
    for entry in report.rejected:
        if entry.dataset_id == dataset_id:
            raise ValueError(f"Dataset '{dataset_id}' rejected by license filter: {entry.reason}")
    raise ValueError(f"Dataset '{dataset_id}' not found in manifest '{manifest_path}'")

"""Shared benchmark provenance, workload, and resource helpers."""

from __future__ import annotations

import ipaddress
import platform
import resource
import subprocess
from pathlib import Path
from typing import Any

import polars as pl

import maxminddb_polars as mmp

REPORT_SCHEMA_VERSION = 1
REPOSITORY_ROOT = Path(__file__).resolve().parents[1]


def command_output(command: list[str], *, cwd: Path | None = None) -> str:
    """Return stripped command output without making metadata collection fatal."""
    try:
        return subprocess.check_output(command, cwd=cwd, text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unavailable"


def source_provenance() -> dict[str, str | bool | None]:
    """Describe the source checkout used to run a benchmark."""
    status = command_output(
        ["git", "status", "--porcelain", "--untracked-files=no"],
        cwd=REPOSITORY_ROOT,
    )
    return {
        "git_revision": command_output(
            ["git", "rev-parse", "HEAD"], cwd=REPOSITORY_ROOT
        ),
        "git_dirty": None if status == "unavailable" else bool(status),
    }


def peak_rss_kib() -> int:
    """Return peak process RSS in KiB on Linux and macOS."""
    peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if platform.system() == "Darwin":
        return (peak + 1023) // 1024
    return peak


def mapped_ip_frame(database: Path, rows: int) -> pl.LazyFrame:
    """Build a deterministic frame of distinct addresses mapped by a database."""
    candidates = [
        str(ipaddress.IPv4Address((index * 2_654_435_761) & 0xFFFFFFFF))
        for index in range(1, rows * 3 + 1)
    ]
    found = (
        pl.DataFrame({"ip": candidates})
        .lazy()
        .filter(
            mmp.lookup_path("ip", database, ["country", "names", "en"]).is_not_null()
        )
        .limit(rows)
        .collect()
    )
    if found.height != rows:
        raise RuntimeError(f"only found {found.height:,} mapped candidate addresses")
    return found.lazy()


def failed_gates(gates: dict[str, Any]) -> list[str]:
    """Return the names of failed boolean benchmark gates."""
    return [
        name for name, value in gates.items() if isinstance(value, bool) and not value
    ]

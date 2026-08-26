from __future__ import annotations

import json
from collections.abc import Callable
from pathlib import Path
from runpy import run_path
from typing import cast

import polars as pl
import pytest

REPOSITORY_ROOT = Path(__file__).parents[2]
RESULTS = sorted((REPOSITORY_ROOT / "benchmarks" / "results").glob("*.json"))
workload_cardinality = cast(
    Callable[[pl.LazyFrame], dict[str, int]],
    run_path(str(REPOSITORY_ROOT / "benchmarks" / "_common.py"))[
        "workload_cardinality"
    ],
)


def test_workload_cardinality_excludes_null_ip_values() -> None:
    frame = pl.DataFrame(
        {"ip": ["192.0.2.1", "192.0.2.2", None, "192.0.2.1", "203.0.113.1"]}
    )

    assert workload_cardinality(frame.lazy()) == {
        "rows": 5,
        "unique_ips": 3,
        "null_rows": 1,
    }


@pytest.mark.parametrize("result_path", RESULTS, ids=lambda path: path.stem)
def test_committed_benchmark_reports_have_cardinality(
    result_path: Path,
) -> None:
    report = json.loads(result_path.read_text(encoding="utf-8"))

    assert report["schema_version"] == 1
    assert isinstance(report["workload"], str) and report["workload"]
    for name in ["rows", "unique_ips", "null_rows"]:
        assert isinstance(report[name], int)
        assert report[name] >= 0
    assert report["rows"] > 0
    assert report["null_rows"] <= report["rows"]
    assert report["unique_ips"] <= report["rows"] - report["null_rows"]

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

import pytest

REPOSITORY_ROOT = Path(__file__).parents[2]
RESULTS = sorted((REPOSITORY_ROOT / "benchmarks" / "results").glob("*.json"))


def test_fixture_benchmark_reports_workload_cardinality() -> None:
    result = subprocess.run(
        [
            sys.executable,
            "benchmarks/lookups.py",
            "--rows",
            "5",
            "--repeats",
            "1",
        ],
        cwd=REPOSITORY_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    report = json.loads(result.stdout)

    assert {name: report[name] for name in ["rows", "unique_ips", "null_rows"]} == {
        "rows": 5,
        "unique_ips": 3,
        "null_rows": 1,
    }
    assert report["workload"] == "repeated"


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

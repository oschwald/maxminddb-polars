from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

REPOSITORY_ROOT = Path(__file__).parents[2]


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

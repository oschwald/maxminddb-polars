from __future__ import annotations

import json
import shutil
import subprocess
import sys
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


@pytest.mark.skipif(sys.platform == "win32", reason="RSS benchmarks require resource")
@pytest.mark.parametrize("script", ["lookups.py", "real_city.py"])
@pytest.mark.parametrize("engine", ["in-memory", "streaming"])
@pytest.mark.parametrize("operation", [None, "city_fields"])
def test_benchmark_engine_and_operation_reports(
    script: str,
    engine: str,
    operation: str | None,
) -> None:
    command = [sys.executable, str(REPOSITORY_ROOT / "benchmarks" / script)]
    if script == "real_city.py":
        command.append(
            str(REPOSITORY_ROOT / "tests/data/test-data/GeoIP2-City-Test.mmdb")
        )
    command.extend(["--rows", "8", "--repeats", "1", "--engine", engine])
    if operation is not None:
        command.extend(["--operation", operation])
    report = json.loads(subprocess.check_output(command, text=True))

    assert report["engine"] == engine
    assert "engine_affinity" in report
    assert report["operations"]["city_fields"]["rows"] == 8
    assert report["operations"]["city_fields"]["output_bytes"] > 0
    if operation is not None:
        assert list(report["operations"]) == [operation]
        assert report["gates"] == {}
    else:
        assert "partial_within_30_percent_of_scalar" in report["gates"]


@pytest.mark.parametrize("script", ["lookups.py", "real_city.py"])
@pytest.mark.parametrize(
    "arguments",
    [
        ["--operation", "unknown"],
        ["--operation", "scalar", "--enforce-gates"],
    ],
)
def test_benchmark_rejects_invalid_operation_arguments(
    script: str,
    arguments: list[str],
    tmp_path: Path,
) -> None:
    command = [sys.executable, str(REPOSITORY_ROOT / "benchmarks" / script)]
    if script == "real_city.py":
        command.append(
            str(REPOSITORY_ROOT / "tests/data/test-data/GeoIP2-City-Test.mmdb")
        )
    else:
        command.extend(["--database-dir", str(tmp_path)])
    result = subprocess.run(
        [*command, "--rows", "8", "--repeats", "1", *arguments],
        capture_output=True,
        text=True,
    )
    assert result.returncode == 2
    assert "error:" in result.stderr


@pytest.mark.skipif(sys.platform == "win32", reason="RSS benchmarks require resource")
@pytest.mark.parametrize(
    ("operation", "database"),
    [
        ("scalar", "GeoIP2-City-Test.mmdb"),
        ("three_paths", "GeoIP2-City-Test.mmdb"),
        ("partial", "GeoIP2-City-Test.mmdb"),
        ("city", "GeoIP2-City-Test.mmdb"),
        ("city_fields", "GeoIP2-City-Test.mmdb"),
        ("country", "GeoIP2-Country-Test.mmdb"),
        ("enterprise", "GeoIP2-Enterprise-Test.mmdb"),
        ("asn", "GeoLite2-ASN-Test.mmdb"),
    ],
)
def test_single_operation_requires_only_its_database(
    operation: str, database: str, tmp_path: Path
) -> None:
    shutil.copyfile(
        REPOSITORY_ROOT / "tests/data/test-data" / database, tmp_path / database
    )
    report = json.loads(
        subprocess.check_output(
            [
                sys.executable,
                str(REPOSITORY_ROOT / "benchmarks/lookups.py"),
                "--database-dir",
                str(tmp_path),
                "--operation",
                operation,
                "--rows",
                "8",
                "--repeats",
                "1",
            ],
            text=True,
        )
    )

    assert list(report["operations"]) == [operation]
    assert report["operations"][operation]["rows"] == 8
    assert report["gates"] == {}


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

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

DATA = Path(__file__).parents[1] / "data" / "test-data"
CACHE_LIMIT = "MAXMINDDB_POLARS_CACHE_MAX_BYTES"

CACHE_SCENARIO = """
import os
import shutil
import tempfile
from pathlib import Path

import polars as pl

import maxminddb_polars as mmp

fixtures = Path(os.environ["MAXMINDDB_POLARS_TEST_DATA"])
scenario = os.environ["MAXMINDDB_POLARS_CACHE_SCENARIO"]
frame = pl.DataFrame({"ip": ["89.160.20.128"]})

with tempfile.TemporaryDirectory() as temporary_directory:
    database = Path(temporary_directory) / "database.mmdb"
    shutil.copy2(fixtures / "GeoIP2-City-Test.mmdb", database)
    old_query = frame.lazy().select(
        mmp.lookup_path("ip", database, ["country", "iso_code"]).alias("country")
    )
    assert old_query.collect().item() == "SE"

    newest_query = frame.lazy().select(
        mmp.lookup_path(
            "ip",
            fixtures / "GeoIP2-Country-Test.mmdb",
            ["country", "iso_code"],
        ).alias("country")
    )
    assert newest_query.collect().item() == "SE"

    replacement = Path(temporary_directory) / "replacement.mmdb"
    shutil.copy2(fixtures / "GeoLite2-ASN-Test.mmdb", replacement)
    os.replace(replacement, database)

    if scenario == "default-retains":
        assert old_query.collect().item() == "SE"
    elif scenario == "zero-evicts":
        try:
            old_query.collect()
        except pl.exceptions.ComputeError as error:
            assert "reconstruct the expression" in str(error)
        else:
            raise AssertionError("the evicted expression unexpectedly remained usable")
        assert newest_query.collect().item() == "SE"
    else:
        raise AssertionError(f"unknown cache scenario: {scenario}")
"""

INVALID_LIMIT_SCENARIO = """
import os
from pathlib import Path

import polars as pl

import maxminddb_polars as mmp

fixtures = Path(os.environ["MAXMINDDB_POLARS_TEST_DATA"])
pl.DataFrame({"ip": ["89.160.20.128"]}).lazy().select(
    mmp.lookup_path(
        "ip",
        fixtures / "GeoIP2-City-Test.mmdb",
        ["country", "iso_code"],
    )
).collect_schema()
"""


def _run_isolated(
    script: str, **environment: str | None
) -> subprocess.CompletedProcess[str]:
    child_environment = os.environ.copy()
    child_environment["MAXMINDDB_POLARS_TEST_DATA"] = str(DATA.resolve())
    for name, value in environment.items():
        if value is None:
            child_environment.pop(name, None)
        else:
            child_environment[name] = value
    return subprocess.run(
        [sys.executable, "-c", script],
        check=False,
        capture_output=True,
        env=child_environment,
        text=True,
    )


@pytest.mark.parametrize(
    ("scenario", "cache_limit"),
    [("default-retains", None), ("zero-evicts", "0")],
)
def test_process_cache_limit_controls_reader_eviction(
    scenario: str, cache_limit: str | None
) -> None:
    result = _run_isolated(
        CACHE_SCENARIO,
        MAXMINDDB_POLARS_CACHE_SCENARIO=scenario,
        MAXMINDDB_POLARS_CACHE_MAX_BYTES=cache_limit,
    )

    assert result.returncode == 0, result.stdout + result.stderr


@pytest.mark.parametrize("cache_limit", ["-1", "not-a-number"])
def test_process_cache_limit_rejects_invalid_values(cache_limit: str) -> None:
    result = _run_isolated(
        INVALID_LIMIT_SCENARIO,
        MAXMINDDB_POLARS_CACHE_MAX_BYTES=cache_limit,
    )

    output = result.stdout + result.stderr
    assert result.returncode != 0
    assert CACHE_LIMIT in output
    assert "must be a non-negative integer number of bytes" in output

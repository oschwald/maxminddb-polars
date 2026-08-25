from __future__ import annotations

import os
import subprocess
import sys
import textwrap
from pathlib import Path
from typing import Literal

import polars as pl
import pytest

import maxminddb_polars as mmp

DATA = Path(__file__).parents[1] / "data" / "test-data"
CITY_DB = DATA / "GeoIP2-City-Test.mmdb"
CUSTOM_DB = DATA / "MaxMind-DB-test-decoder.mmdb"


@pytest.mark.parametrize("engine", ["auto", "streaming"])
def test_known_path_eager_lazy_and_streaming(
    engine: Literal["auto", "streaming"],
) -> None:
    expression = mmp.lookup_path("ip", CITY_DB, ("country", "iso_code")).alias(
        "country"
    )
    frame = pl.DataFrame(
        {"ip": ["89.160.20.128", None, "203.0.113.1", "89.160.20.129"]}
    )
    eager = frame.select(expression)
    lazy = frame.lazy().select(expression).collect(engine=engine)

    assert eager.schema == {"country": pl.String}
    assert eager.to_dict(as_series=False) == {"country": ["SE", None, None, "SE"]}
    assert lazy.equals(eager)


def test_namespace_matches_standalone_function() -> None:
    result = pl.DataFrame({"ip": ["89.160.20.128"]}).select(
        standalone=mmp.lookup_path("ip", CITY_DB, ["location", "longitude"]),
        namespace=pl.col("ip").mmdb.lookup_path(  # type: ignore[attr-defined]
            CITY_DB, ["location", "longitude"]
        ),
    )
    assert result.to_dict(as_series=False) == {
        "standalone": [15.6167],
        "namespace": [15.6167],
    }


def test_explicit_dtype_supports_an_unknown_database() -> None:
    result = pl.DataFrame({"ip": ["::1.1.1.0"]}).select(
        mmp.lookup_path("ip", CUSTOM_DB, ["uint32"], dtype=pl.UInt32).alias("value")
    )
    assert result.to_dict(as_series=False) == {"value": [2**28]}


def test_large_binary_scalar_batch_preserves_order_and_nulls() -> None:
    values = ["::1.1.1.0", None, "not-an-ip"] * 2_731
    result = pl.DataFrame({"ip": values}).select(
        mmp.lookup_path(
            "ip", CUSTOM_DB, ["bytes"], dtype=pl.Binary, strict=False
        ).alias("value")
    )

    assert result["value"].to_list() == [b"\x00\x00\x00*", None, None] * 2_731


def test_parallel_dispatch_with_controlled_worker_pool() -> None:
    script = textwrap.dedent(
        f"""
        import polars as pl

        import maxminddb_polars as mmp

        database = {str(CITY_DB)!r}
        custom_database = {str(CUSTOM_DB)!r}
        pattern = ["89.160.20.128", None, "not-an-ip", "203.0.113.1"]

        def lookup(values: list[str | None]) -> pl.DataFrame:
            return pl.DataFrame({{"ip": values}}).select(
                mmp.lookup_path(
                    "ip",
                    database,
                    ["country", "iso_code"],
                    strict=False,
                ).alias("country")
            )

        assert pl.thread_pool_size() == 3
        threshold_values = (pattern * 2_048)[:8_192]
        parallel = lookup(threshold_values)
        serial = lookup(threshold_values[:-1])
        assert parallel["country"].n_chunks() == 4
        assert serial["country"].n_chunks() == 1

        first = (pattern * 2_049)[:8_193]
        second = (pattern * 2_561)[1:10_242]
        multi_chunk = pl.concat(
            [pl.DataFrame({{"ip": first}}), pl.DataFrame({{"ip": second}})],
            rechunk=False,
        )
        assert multi_chunk["ip"].n_chunks() == 2
        result = multi_chunk.select(
            mmp.lookup_path(
                "ip",
                database,
                ["country", "iso_code"],
                strict=False,
            ).alias("country")
        )
        expected = [
            "SE" if value == "89.160.20.128" else None
            for value in [*first, *second]
        ]
        assert result["country"].n_chunks() == 10
        assert result["country"].to_list() == expected

        fragmented = pl.concat(
            [pl.Series("ip", ["89.160.20.128"]) for _ in range(8_192)],
            rechunk=False,
        )
        assert fragmented.n_chunks() == 8_192
        fragmented_result = pl.DataFrame([fragmented]).select(
            mmp.lookup_path(
                "ip", database, ["country", "iso_code"]
            ).alias("country")
        )
        assert fragmented_result["country"].n_chunks() == 4
        assert fragmented_result["country"].head(4).to_list() == ["SE"] * 4

        binary_values = ["::1.1.1.0", None, "not-an-ip"] * 2_731
        binary_result = pl.DataFrame({{"ip": binary_values}}).select(
            mmp.lookup_path(
                "ip",
                custom_database,
                ["bytes"],
                dtype=pl.Binary,
                strict=False,
            ).alias("value")
        )
        assert binary_result["value"].to_list() == [
            b"\\x00\\x00\\x00*", None, None
        ] * 2_731

        strict_values = ["89.160.20.128"] * 8_192
        strict_values[4_096] = "not-an-ip"
        try:
            pl.DataFrame({{"ip": strict_values}}).select(
                mmp.lookup_path(
                    "ip", database, ["country", "iso_code"]
                )
            )
        except pl.exceptions.ComputeError as error:
            assert "invalid IP address" in str(error)
        else:
            raise AssertionError("parallel strict lookup accepted an invalid IP")
        """
    )
    environment = os.environ.copy()
    environment["POLARS_MAX_THREADS"] = "3"

    completed = subprocess.run(
        [sys.executable, "-c", script],
        check=False,
        capture_output=True,
        env=environment,
        text=True,
    )
    assert completed.returncode == 0, completed.stderr


def test_unknown_database_requires_dtype_during_planning() -> None:
    query = (
        pl.DataFrame({"ip": ["::1.1.1.0"]})
        .lazy()
        .select(mmp.lookup_path("ip", CUSTOM_DB, ["uint32"]))
    )
    with pytest.raises(pl.exceptions.ComputeError, match="pass dtype explicitly"):
        query.collect_schema()


def test_invalid_ip_strictness_and_nulls() -> None:
    frame = pl.DataFrame({"ip": ["not-an-ip", None]})
    with pytest.raises(pl.exceptions.ComputeError, match="invalid IP address"):
        frame.select(mmp.lookup_path("ip", CITY_DB, ["country", "iso_code"]))

    result = frame.select(
        mmp.lookup_path("ip", CITY_DB, ["country", "iso_code"], strict=False).alias(
            "country"
        )
    )
    assert result.to_dict(as_series=False) == {"country": [None, None]}


@pytest.mark.parametrize(
    ("path", "error"),
    [
        ([], ValueError),
        ([True], TypeError),
        ([object()], TypeError),
        ([2**80], ValueError),
    ],
)
def test_rejects_invalid_paths(path: list[object], error: type[Exception]) -> None:
    with pytest.raises(error):
        mmp.lookup_path("ip", CITY_DB, path)  # type: ignore[arg-type]


def test_rejects_wrong_input_and_explicit_dtype() -> None:
    with pytest.raises(pl.exceptions.ComputeError, match="String dtype"):
        pl.DataFrame({"ip": [1]}).select(
            mmp.lookup_path("ip", CITY_DB, ["country", "iso_code"])
        )

    query = (
        pl.DataFrame({"ip": ["89.160.20.128"]})
        .lazy()
        .select(
            mmp.lookup_path("ip", CITY_DB, ["country", "iso_code"], dtype=pl.UInt32)
        )
    )
    with pytest.raises(pl.exceptions.ComputeError, match="does not match"):
        query.collect_schema()

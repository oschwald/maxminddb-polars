"""Compare overlapping APIs with pinned third-party Polars integrations."""

from __future__ import annotations

import argparse
import gc
import importlib
import importlib.metadata
import json
import os
import platform
import statistics
import sys
import time
from pathlib import Path
from typing import Any

import polars as pl
from _common import (
    REPORT_SCHEMA_VERSION,
    mapped_ip_frame,
    peak_rss_kib,
    source_provenance,
    workload_cardinality,
)

import maxminddb_polars as mmp


def _version(distribution: str) -> str:
    return importlib.metadata.version(distribution)


def _repeated_frame(rows: int) -> pl.LazyFrame:
    samples = ["8.8.8.8", "1.1.1.1", "81.2.69.142", "89.160.20.128"]
    ips = (samples * ((rows + len(samples) - 1) // len(samples)))[:rows]
    return pl.DataFrame({"ip": ips}).lazy()


def _plans(
    frame: pl.LazyFrame,
    database: Path,
    polars_maxminddb: Any,
    polars_iptools: Any,
) -> dict[str, pl.LazyFrame]:
    partial: dict[str, Any] = {
        "country": {"names": {"en": pl.String}},
        "city": {"names": {"en": pl.String}},
        "location": {"longitude": pl.Float64},
    }
    iptools_full = polars_iptools.geoip.full("ip")
    return {
        "maxminddb_polars_path_country": frame.select(
            mmp.lookup_path("ip", database, ["country", "names", "en"]).alias("country")
        ),
        "polars_maxminddb_country": frame.select(
            polars_maxminddb.ip_lookup_country("ip", str(database)).alias("country")
        ),
        "polars_iptools_full_country": frame.select(
            iptools_full.struct.field("country").alias("country")
        ),
        "maxminddb_polars_partial_3": frame.select(
            mmp.lookup("ip", database, dtype=partial).alias("selected")
        ),
        "polars_maxminddb_3": frame.select(
            pl.struct(
                polars_maxminddb.ip_lookup_country("ip", str(database)).alias(
                    "country"
                ),
                polars_maxminddb.ip_lookup_city("ip", str(database)).alias("city"),
                polars_maxminddb.ip_lookup_longitude("ip", str(database)).alias(
                    "longitude"
                ),
            ).alias("selected")
        ),
        "polars_iptools_full_3": frame.with_columns(iptools_full.alias("geoip")).select(
            pl.struct(
                pl.col("geoip").struct.field("country"),
                pl.col("geoip").struct.field("city"),
                pl.col("geoip").struct.field("longitude"),
            ).alias("selected")
        ),
    }


def _measure(plan: pl.LazyFrame, repeats: int, rows: int) -> dict[str, Any]:
    warm = plan.collect()
    if warm.height == 0:
        raise RuntimeError("comparison plan returned no rows")
    del warm
    gc.collect()
    samples = []
    output_bytes: int | float = 0
    for _ in range(repeats):
        started = time.perf_counter()
        result = plan.collect()
        samples.append(time.perf_counter() - started)
        output_bytes = result.estimated_size()
        del result
        gc.collect()
    median = statistics.median(samples)
    return {
        "median_seconds": median,
        "rows_per_second": rows / median,
        "samples_seconds": samples,
        "output_bytes": output_bytes,
    }


def _validate_outputs(plans: dict[str, pl.LazyFrame]) -> int:
    scalar = plans["maxminddb_polars_path_country"].collect()
    for name in ["polars_maxminddb_country", "polars_iptools_full_country"]:
        if not plans[name].collect().equals(scalar):
            raise RuntimeError(f"{name} returned different country values")

    ours = (
        plans["maxminddb_polars_partial_3"]
        .collect()
        .select(
            pl.col("selected")
            .struct.field("country")
            .struct.field("names")
            .struct.field("en")
            .alias("country"),
            pl.col("selected")
            .struct.field("city")
            .struct.field("names")
            .struct.field("en")
            .alias("city"),
            pl.col("selected")
            .struct.field("location")
            .struct.field("longitude")
            .alias("longitude"),
        )
    )
    complete = ours.select(pl.all_horizontal(pl.all().is_not_null())).to_series()
    complete_rows = int(complete.sum() or 0)
    if complete_rows == 0:
        raise RuntimeError("comparison has no fully populated three-field rows")
    for name in ["polars_maxminddb_3", "polars_iptools_full_3"]:
        result = plans[name].collect().unnest("selected")
        if not result.filter(complete).equals(ours.filter(complete)):
            raise RuntimeError(f"{name} returned different populated values")
    return complete_rows


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("database", type=Path)
    parser.add_argument("iptools_database_dir", type=Path)
    parser.add_argument("--rows", type=int, default=50_000)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--workload", choices=["high", "repeated"], default="high")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    if args.rows <= 0 or args.rows > 250_000 or args.repeats <= 0:
        parser.error("--rows must be in 1..250000 and --repeats must be positive")

    database = args.database.expanduser().resolve(strict=True)
    database_dir = args.iptools_database_dir.expanduser().resolve(strict=True)
    os.environ["MAXMIND_MMDB_DIR"] = str(database_dir)
    polars_maxminddb = importlib.import_module("polars_maxminddb")
    polars_iptools = importlib.import_module("polars_iptools")
    frame = (
        mapped_ip_frame(database, args.rows)
        if args.workload == "high"
        else _repeated_frame(args.rows)
    )
    cardinality = workload_cardinality(frame)
    plans = _plans(frame, database, polars_maxminddb, polars_iptools)
    complete_rows = _validate_outputs(plans)
    measurements = {
        name: _measure(plan, args.repeats, cardinality["rows"])
        for name, plan in plans.items()
    }
    report = {
        "schema_version": REPORT_SCHEMA_VERSION,
        "environment": {
            "platform": platform.platform(),
            "python": sys.version,
            "polars": pl.__version__,
            "polars_max_threads": pl.thread_pool_size(),
            "maxminddb_polars": mmp.__version__,
            "polars_maxminddb": _version("polars-maxminddb"),
            "polars_iptools": _version("polars-iptools"),
            "database_bytes": database.stat().st_size,
            **source_provenance(),
        },
        "workload": args.workload,
        **cardinality,
        "validation": {
            "country_outputs_equal": True,
            "fully_populated_three_field_outputs_equal": True,
            "fully_populated_rows_compared": complete_rows,
            "missing_value_difference": (
                "competitors use empty strings/default numbers where "
                "maxminddb-polars preserves nulls"
            ),
        },
        "operations": measurements,
        "peak_rss_kib": peak_rss_kib(),
    }
    encoded = json.dumps(report, indent=2) + "\n"
    print(encoded, end="")
    if args.json is not None:
        args.json.write_text(encoded)


if __name__ == "__main__":
    main()

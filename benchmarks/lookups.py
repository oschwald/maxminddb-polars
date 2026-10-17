"""Reproducible lookup throughput and process-memory benchmark."""

from __future__ import annotations

import argparse
import json
import os
import platform
import statistics
import sys
import time
from pathlib import Path
from typing import Any, Literal

import polars as pl
from _common import (
    REPORT_SCHEMA_VERSION,
    command_output,
    failed_gates,
    peak_rss_kib,
    source_provenance,
    workload_cardinality,
)

import maxminddb_polars as mmp


def _fixture_frame(ip: str, adjacent: str, rows: int) -> pl.LazyFrame:
    ips = [ip, adjacent, None, "203.0.113.1"]
    return pl.DataFrame({"ip": (ips * ((rows + 3) // 4))[:rows]}).lazy()


def _operations(
    database_dir: Path, rows: int, city_frame: pl.LazyFrame
) -> dict[str, pl.LazyFrame]:
    city = database_dir / "GeoIP2-City-Test.mmdb"
    partial: dict[str, Any] = {
        "country": {"iso_code": pl.String},
        "location": {"latitude": pl.Float64, "longitude": pl.Float64},
    }
    record = mmp.lookup("ip", city)
    return {
        "scalar": city_frame.select(
            mmp.lookup_path("ip", city, ["country", "iso_code"]).alias("country")
        ),
        "three_paths": city_frame.select(
            mmp.lookup_path("ip", city, ["country", "iso_code"]).alias("country"),
            mmp.lookup_path("ip", city, ["location", "latitude"]).alias("latitude"),
            mmp.lookup_path("ip", city, ["location", "longitude"]).alias("longitude"),
        ),
        "partial": city_frame.select(
            mmp.lookup("ip", city, dtype=partial).alias("record")
        ),
        "city": city_frame.select(record.alias("record")),
        "city_fields": city_frame.select(
            country=record.struct.field("country").struct.field("iso_code"),
            latitude=record.struct.field("location").struct.field("latitude"),
            longitude=record.struct.field("location").struct.field("longitude"),
        ),
        "country": city_frame.select(
            mmp.lookup("ip", database_dir / "GeoIP2-Country-Test.mmdb").alias("record")
        ),
        "enterprise": _fixture_frame("::2.125.160.216", "::2.125.160.217", rows).select(
            mmp.lookup("ip", database_dir / "GeoIP2-Enterprise-Test.mmdb").alias(
                "record"
            )
        ),
        "asn": _fixture_frame("1.0.0.0", "1.0.0.1", rows).select(
            mmp.lookup("ip", database_dir / "GeoLite2-ASN-Test.mmdb").alias("record")
        ),
    }


def _benchmark(
    plan: pl.LazyFrame,
    repeats: int,
    engine: Literal["auto", "in-memory", "streaming"],
) -> dict[str, Any]:
    plan.collect(engine=engine)
    wall_samples: list[float] = []
    cpu_samples: list[float] = []
    result = pl.DataFrame()
    for _ in range(repeats):
        wall_started = time.perf_counter()
        cpu_started = time.process_time()
        result = plan.collect(engine=engine)
        cpu_samples.append(time.process_time() - cpu_started)
        wall_samples.append(time.perf_counter() - wall_started)
    median = statistics.median(wall_samples)
    return {
        "rows": result.height,
        "median_seconds": median,
        "rows_per_second": result.height / median,
        "wall_samples_seconds": wall_samples,
        "cpu_samples_seconds": cpu_samples,
        "output_bytes": result.estimated_size(),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rows", type=int, default=100_000)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument(
        "--database-dir",
        type=Path,
        default=Path(__file__).parents[1] / "tests" / "data" / "test-data",
    )
    parser.add_argument(
        "--engine", choices=["auto", "in-memory", "streaming"], default="auto"
    )
    parser.add_argument("--operation", help="Measure one operation in a fresh process")
    parser.add_argument("--json", type=Path)
    parser.add_argument("--enforce-gates", action="store_true")
    args = parser.parse_args()
    if args.operation is not None and args.enforce_gates:
        parser.error("--enforce-gates requires all operations")
    if args.rows <= 0 or args.repeats <= 0:
        parser.error("--rows and --repeats must be positive")

    city_frame = _fixture_frame("89.160.20.128", "89.160.20.129", args.rows)
    plans = _operations(args.database_dir, args.rows, city_frame)
    if args.operation is not None:
        if args.operation not in plans:
            parser.error(
                f"unknown operation: {args.operation}; choose from {', '.join(plans)}"
            )
        plans = {args.operation: plans[args.operation]}
    operations = {
        name: _benchmark(plan, args.repeats, args.engine)
        for name, plan in plans.items()
    }
    gates = {}
    if args.operation is None:
        partial_ratio = (
            operations["partial"]["median_seconds"]
            / operations["scalar"]["median_seconds"]
        )
        gates = {
            "partial_to_scalar_median_ratio": partial_ratio,
            "partial_within_30_percent_of_scalar": partial_ratio <= 1.3,
        }
    report = {
        "schema_version": REPORT_SCHEMA_VERSION,
        "engine": args.engine,
        "engine_affinity": os.environ.get("POLARS_ENGINE_AFFINITY"),
        "workload": "repeated",
        **workload_cardinality(city_frame),
        "environment": {
            "platform": platform.platform(),
            "processor": platform.processor() or "unavailable",
            "python": sys.version,
            "polars": pl.__version__,
            "maxminddb_polars": mmp.__version__,
            "polars_max_threads": pl.thread_pool_size(),
            **source_provenance(),
            "rustc": command_output(["rustc", "-Vv"]),
            "database_kind": "MaxMind-DB test fixtures",
            "database_revision": command_output(
                ["git", "-C", str(args.database_dir.parent), "rev-parse", "HEAD"]
            ),
        },
        "operations": operations,
        "gates": gates,
        "peak_rss_kib": peak_rss_kib(),
    }
    encoded = json.dumps(report, indent=2) + "\n"
    print(encoded, end="")
    if args.json is not None:
        args.json.write_text(encoded)
    failures = failed_gates(gates)
    if args.enforce_gates and failures:
        raise SystemExit(f"benchmark gates failed: {', '.join(failures)}")


if __name__ == "__main__":
    main()

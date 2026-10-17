"""Exercise and benchmark a caller-supplied full City database."""

from __future__ import annotations

import argparse
import gc
import json
import os
import statistics
import time
from pathlib import Path
from typing import Any, Literal

import polars as pl
from _common import (
    REPORT_SCHEMA_VERSION,
    failed_gates,
    mapped_ip_frame,
    peak_rss_kib,
    source_provenance,
    workload_cardinality,
)

import maxminddb_polars as mmp


def _measure(
    plan: pl.LazyFrame,
    repeats: int,
    engine: Literal["auto", "in-memory", "streaming"],
) -> dict[str, Any]:
    warm = plan.collect(engine=engine)
    if all(column.null_count() == warm.height for column in warm):
        raise ValueError("sample IPs produced no records in the supplied City database")
    row_count = warm.height
    output_bytes = warm.estimated_size()
    del warm
    gc.collect()
    samples = []
    for _ in range(repeats):
        started = time.perf_counter()
        result = plan.collect(engine=engine)
        samples.append(time.perf_counter() - started)
        output_bytes = result.estimated_size()
        del result
        gc.collect()
    median = statistics.median(samples)
    return {
        "rows": row_count,
        "median_seconds": median,
        "rows_per_second": row_count / median,
        "samples_seconds": samples,
        "output_bytes": output_bytes,
    }


def _workload_frame(database: Path, rows: int, workload: str) -> pl.LazyFrame:
    if workload == "high":
        return mapped_ip_frame(database, rows)
    if workload == "half":
        unique = mapped_ip_frame(database, (rows + 1) // 2).collect()["ip"].to_list()
        return pl.DataFrame({"ip": (unique * 2)[:rows]}).lazy()

    samples = [
        "81.2.69.142",
        "128.101.101.101",
        "8.8.8.8",
        "2001:4860:4860::8888",
        None,
        "203.0.113.1",
    ]
    ips = (samples * ((rows + len(samples) - 1) // len(samples)))[:rows]
    return pl.DataFrame({"ip": ips}).lazy()


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("database", type=Path)
    parser.add_argument("--rows", type=int, default=50_000)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument(
        "--workload", choices=["repeated", "half", "high"], default="repeated"
    )
    parser.add_argument("--allow-large-run", action="store_true")
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
    if args.rows > 250_000 and not args.allow_large_run:
        parser.error(
            "more than 250,000 whole-City rows can require several GiB; "
            "pass --allow-large-run only in a memory-controlled environment"
        )

    database = args.database.expanduser().resolve(strict=True)
    frame = _workload_frame(database, args.rows, args.workload)
    cardinality = workload_cardinality(frame)
    partial: dict[str, Any] = {
        "country": {"iso_code": pl.String},
        "location": {"latitude": pl.Float64, "longitude": pl.Float64},
    }
    record = mmp.lookup("ip", database)
    operations = {
        "scalar": frame.select(
            mmp.lookup_path("ip", database, ["country", "iso_code"])
        ),
        "partial": frame.select(mmp.lookup("ip", database, dtype=partial)),
        "whole_city": frame.select(record),
        "three_paths": frame.select(
            country=mmp.lookup_path("ip", database, ["country", "iso_code"]),
            latitude=mmp.lookup_path("ip", database, ["location", "latitude"]),
            longitude=mmp.lookup_path("ip", database, ["location", "longitude"]),
        ),
        "city_fields": frame.select(
            country=record.struct.field("country").struct.field("iso_code"),
            latitude=record.struct.field("location").struct.field("latitude"),
            longitude=record.struct.field("location").struct.field("longitude"),
        ),
    }
    if args.operation is not None:
        if args.operation not in operations:
            parser.error(
                f"unknown operation: {args.operation}; "
                f"choose from {', '.join(operations)}"
            )
        operations = {args.operation: operations[args.operation]}
    measurements = {
        name: _measure(plan, args.repeats, args.engine)
        for name, plan in operations.items()
    }
    gates = {}
    if args.operation is None:
        partial_ratio = (
            measurements["partial"]["median_seconds"]
            / measurements["scalar"]["median_seconds"]
        )
        gates = {
            "partial_to_scalar_median_ratio": partial_ratio,
            "partial_within_30_percent_of_scalar": partial_ratio <= 1.3,
        }
    report = {
        "schema_version": REPORT_SCHEMA_VERSION,
        "engine": args.engine,
        "engine_affinity": os.environ.get("POLARS_ENGINE_AFFINITY"),
        "database_bytes": database.stat().st_size,
        "polars": pl.__version__,
        "polars_max_threads": pl.thread_pool_size(),
        "maxminddb_polars": mmp.__version__,
        **source_provenance(),
        "workload": args.workload,
        **cardinality,
        "operations": measurements,
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

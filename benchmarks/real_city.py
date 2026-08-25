"""Exercise and benchmark a caller-supplied full City database."""

from __future__ import annotations

import argparse
import gc
import json
import resource
import statistics
import subprocess
import time
from pathlib import Path
from typing import Any

import polars as pl

import maxminddb_polars as mmp


def _command_output(command: list[str]) -> str:
    try:
        return subprocess.check_output(command, text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        return "unavailable"


def _measure(plan: pl.LazyFrame, repeats: int) -> dict[str, Any]:
    warm = plan.collect()
    if warm.to_series().null_count() == warm.height:
        raise ValueError("sample IPs produced no records in the supplied City database")
    row_count = warm.height
    output_bytes = warm.estimated_size()
    del warm
    gc.collect()
    samples = []
    for _ in range(repeats):
        started = time.perf_counter()
        result = plan.collect()
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


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("database", type=Path)
    parser.add_argument("--rows", type=int, default=50_000)
    parser.add_argument("--repeats", type=int, default=5)
    parser.add_argument("--allow-large-run", action="store_true")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()
    if args.rows <= 0 or args.repeats <= 0:
        parser.error("--rows and --repeats must be positive")
    if args.rows > 250_000 and not args.allow_large_run:
        parser.error(
            "more than 250,000 whole-City rows can require several GiB; "
            "pass --allow-large-run only in a memory-controlled environment"
        )

    database = args.database.expanduser().resolve(strict=True)
    samples = [
        "81.2.69.142",
        "128.101.101.101",
        "8.8.8.8",
        "2001:4860:4860::8888",
        None,
        "203.0.113.1",
    ]
    ips = (samples * ((args.rows + len(samples) - 1) // len(samples)))[: args.rows]
    frame = pl.DataFrame({"ip": ips}).lazy()
    partial: dict[str, Any] = {
        "country": {"iso_code": pl.String},
        "location": {"latitude": pl.Float64, "longitude": pl.Float64},
    }
    operations = {
        "scalar": frame.select(
            mmp.lookup_path("ip", database, ["country", "iso_code"])
        ),
        "partial": frame.select(mmp.lookup("ip", database, dtype=partial)),
        "whole_city": frame.select(mmp.lookup("ip", database)),
    }
    measurements = {
        name: _measure(plan, args.repeats) for name, plan in operations.items()
    }
    partial_ratio = (
        measurements["partial"]["median_seconds"]
        / measurements["scalar"]["median_seconds"]
    )
    report = {
        "database_bytes": database.stat().st_size,
        "polars": pl.__version__,
        "polars_max_threads": pl.thread_pool_size(),
        "maxminddb_polars": mmp.__version__,
        "git_revision": _command_output(["git", "rev-parse", "HEAD"]),
        "operations": measurements,
        "gates": {
            "partial_to_scalar_median_ratio": partial_ratio,
            "partial_within_30_percent_of_scalar": partial_ratio <= 1.3,
        },
        "peak_rss_kib": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss,
    }
    encoded = json.dumps(report, indent=2) + "\n"
    print(encoded, end="")
    if args.json is not None:
        args.json.write_text(encoded)


if __name__ == "__main__":
    main()

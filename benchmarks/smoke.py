"""Measure native expression overhead for the bootstrap plugin."""

from __future__ import annotations

import argparse
import json
import statistics
import time
from pathlib import Path

import polars as pl

from maxminddb_polars._internal import identity


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--rows", type=int, default=500_000)
    parser.add_argument("--repeats", type=int, default=7)
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()

    frame = pl.DataFrame({"value": ["127.0.0.1"] * args.rows}).lazy()
    plan = frame.select(identity("value"))
    plan.collect()

    samples = []
    for _ in range(args.repeats):
        started = time.perf_counter()
        result = plan.collect()
        samples.append(time.perf_counter() - started)

    median = statistics.median(samples)
    report = {
        "polars_version": pl.__version__,
        "rows": result.height,
        "median_seconds": median,
        "rows_per_second": result.height / median,
        "samples_seconds": samples,
    }
    print(json.dumps(report, indent=2))
    if args.json is not None:
        args.json.write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()

"""Compare ``lookup_frame`` with the expression API for two City fields."""

from __future__ import annotations

import argparse
import statistics
import time
from pathlib import Path
from typing import Any

import polars as pl
from _common import mapped_ip_frame

import maxminddb_polars as mmp
from maxminddb_polars.experimental import lookup_frame

COUNTRY = ["country", "iso_code"]
LATITUDE = ["location", "latitude"]
PARTIAL: dict[str, Any] = {
    "country": {"iso_code": pl.String},
    "location": {"latitude": pl.Float64},
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("database", type=Path, help="a City or Enterprise MMDB")
    parser.add_argument("--rows", type=int, default=200_000)
    parser.add_argument(
        "--distinct", type=int, help="distinct IPs to repeat; default: all rows"
    )
    parser.add_argument("--repeats", type=int, default=15)
    parser.add_argument("--warmups", type=int, default=3)
    parser.add_argument(
        "--engine", choices=["in-memory", "streaming"], default="in-memory"
    )
    args = parser.parse_args()

    db = args.database
    distinct = args.distinct or args.rows
    ips = mapped_ip_frame(db, distinct).collect()["ip"]
    frame = pl.DataFrame(
        {"ip": ips.gather(pl.arange(args.rows, eager=True) % distinct)}
    ).lazy()
    full = mmp.lookup("ip", db)
    partial = mmp.lookup("ip", db, dtype=PARTIAL)
    queries = {
        "full lookup + struct fields": frame.select(
            country=full.struct.field("country").struct.field("iso_code"),
            latitude=full.struct.field("location").struct.field("latitude"),
        ),
        "two lookup_path": frame.select(
            country=mmp.lookup_path("ip", db, COUNTRY),
            latitude=mmp.lookup_path("ip", db, LATITUDE),
        ),
        "manual partial dtype": frame.select(record=partial).select(
            country=pl.col("record").struct.field("country").struct.field("iso_code"),
            latitude=pl.col("record").struct.field("location").struct.field("latitude"),
        ),
        "lookup_frame": lookup_frame(frame, "ip", db).select(
            country="country.iso_code", latitude="location.latitude"
        ),
        "lookup_frame, new frame each run": None,
    }

    def run(name: str) -> pl.DataFrame:
        query = queries[name]
        if query is None:
            # Include schema inference and plan resolution in the timing.
            query = lookup_frame(frame, "ip", db).select(
                country="country.iso_code", latitude="location.latitude"
            )
        return query.collect(engine=args.engine)

    expected = run("full lookup + struct fields")
    assert expected.height == args.rows and expected["country"].null_count() == 0
    samples: dict[str, list[float]] = {name: [] for name in queries}
    for name in queries:
        assert run(name).equals(expected), name
        for _ in range(args.warmups):
            run(name)
    # Interleave the operations so that drift affects all of them equally.
    for _ in range(args.repeats):
        for name in queries:
            start = time.perf_counter()
            run(name)
            samples[name].append((time.perf_counter() - start) * 1e3)

    print(
        f"polars {pl.__version__}, {args.rows:,} rows, {distinct:,} distinct IPs, "
        f"engine={args.engine}"
    )
    print(f"{'operation':34} {'median ms':>10} {'min ms':>8} {'max ms':>8}")
    for name, values in samples.items():
        print(
            f"{name:34} {statistics.median(values):10.2f} "
            f"{min(values):8.2f} {max(values):8.2f}"
        )


if __name__ == "__main__":
    main()

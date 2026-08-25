# Comparison with existing Polars integrations

This comparison was rerun on 2026-08-24 against the current public releases of
both overlapping projects:

- [`polars-maxminddb` 0.2.3](https://pypi.org/project/polars-maxminddb/),
  which exposes separate fixed City, Country, and ASN scalar functions;
- [`polars-iptools` 0.2.2](https://pypi.org/project/polars-iptools/), whose
  broader IP toolkit includes a fixed GeoIP `full` Struct.

`maxminddb-polars` instead accepts the database path on each expression and
supports inferred whole records for nine database families, arbitrary custom
Struct schemas, validated partial schemas, and scalar/nested paths. The
competitors use empty strings and numeric defaults for some missing fields;
this package preserves MMDB absence as null according to its documented
validity semantics.

## Reproducible result

The benchmark used the unreleased `maxminddb-polars` candidate at revision
`573f6e5` (whose package metadata is still 0.1.2), Python 3.13.12, Polars
1.43.2, one Polars thread, a 33.7 MB GeoLite2 City database, 50,000 rows, and
the median of five warm runs. The three-field cases select English country
name, English city name, and longitude. Throughput is millions of rows per
second; external results are informational, not release gates. One thread
keeps the implementation comparison CPU-normalized; multi-thread scalar
results are recorded in [`performance.md`](performance.md).

| Operation                              | 50k distinct | 4 repeated IPs |
| -------------------------------------- | -----------: | -------------: |
| `maxminddb-polars` scalar path         |         4.66 |           8.82 |
| `polars-maxminddb` country             |         0.58 |           0.64 |
| `polars-iptools.full` → country        |         1.00 |           1.37 |
| `maxminddb-polars` fused partial       |         3.62 |           8.64 |
| three `polars-maxminddb` calls         |         0.19 |           0.21 |
| one materialized `polars-iptools.full` |         0.99 |           1.38 |

All country outputs were identical. Three-field values were identical on all
fully populated rows (26,998 distinct-IP rows and 25,000 repeated-IP rows).
Missing values are intentionally not declared identical because of the null
versus empty/default semantic difference.

Peak process RSS was 198 MiB for the distinct workload and 158 MiB for the
repeated workload. Content-free source results are committed as
[`comparison-high-cardinality.json`](../benchmarks/results/comparison-high-cardinality.json)
and
[`comparison-repeated.json`](../benchmarks/results/comparison-repeated.json).

## Reproduction

Install the exact tested versions into an isolated environment, arrange
`GeoLite2-City.mmdb` and `GeoLite2-ASN.mmdb` in the directory required by
`polars-iptools`, and run:

```console
python -m pip install \
  polars==1.43.2 \
  polars-iptools==0.2.2 \
  polars-maxminddb==0.2.3
```

Build the candidate checkout in release mode before running the comparison:

```console
uv run maturin develop --release --locked
```

```console
POLARS_MAX_THREADS=1 python benchmarks/compare.py \
  /secure/path/GeoLite2-City.mmdb \
  /secure/path/iptools-database-directory \
  --rows 50000 \
  --repeats 5 \
  --workload high \
  --json comparison.json
```

The script validates populated overlapping outputs before timing and refuses
more than 250,000 rows. Both pinned competitors provide compatible abi3
manylinux x86-64 wheels for the tested environment.

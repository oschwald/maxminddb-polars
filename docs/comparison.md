# Comparison with existing Polars integrations

On 2026-08-25, we compared `maxminddb-polars` with:

- [`polars-maxminddb` 0.2.3](https://pypi.org/project/polars-maxminddb/),
  which has separate City, Country, and ASN scalar functions.
- [`polars-iptools` 0.2.2](https://pypi.org/project/polars-iptools/), whose
  broader IP toolkit includes a fixed GeoIP `full` Struct.

`maxminddb-polars` accepts a database path per expression and supports full
records, selected fields, and custom schemas. It returns null for missing scalar
fields. The other packages use empty strings or numeric defaults for some fields,
so results differ when data is missing. See [Missing values](api.md#dtypes-and-validity).

These measurements describe the pinned versions, not current package performance.

## Reproducible result

The benchmark used the 0.1.3 release candidate, with package metadata still at
0.1.2, Python 3.13.12, and Polars 1.43.2. It measured 50,000 rows against a
33.7 MB GeoLite2 City database, using one worker and the median of five warm runs.
Each report records the exact source revision.

The three-field cases read English country name, English city name, and longitude.
Throughput is in millions of rows per second. These comparisons do not gate releases.

| Operation                              | 50k distinct | 4 repeated IPs |
| -------------------------------------- | -----------: | -------------: |
| `maxminddb-polars` scalar path         |         4.90 |           9.06 |
| `polars-maxminddb` country             |         0.58 |           0.64 |
| `polars-iptools.full` → country        |         1.20 |           1.46 |
| `maxminddb-polars` fused partial       |         3.61 |           7.96 |
| three `polars-maxminddb` calls         |         0.19 |           0.22 |
| one materialized `polars-iptools.full` |         1.21 |           1.50 |

Country outputs matched. Three-field values matched on fully populated rows:
26,998 distinct-IP rows and 25,000 repeated-IP rows. Missing values differ as
described above.

Peak resident memory was 192 MiB for distinct IPs and 158 MiB for repeated IPs.
Reports contain metrics, not database contents:
[`comparison-high-cardinality.json`](../benchmarks/results/comparison-high-cardinality.json)
and
[`comparison-repeated.json`](../benchmarks/results/comparison-repeated.json).

## Reproduction

Install all three packages in one isolated environment. To reproduce a historical
report, first check out its `git_revision` and initialize its submodules. Put
`GeoLite2-City.mmdb` and `GeoLite2-ASN.mmdb` in the directory required by
`polars-iptools`, and run:

```console
uv sync --all-extras --locked --no-install-project
uv pip install --python .venv/bin/python \
  polars==1.43.2 \
  polars-iptools==0.2.2 \
  polars-maxminddb==0.2.3
uv run --no-sync maturin develop --release --locked
```

```console
POLARS_MAX_THREADS=1 uv run --no-sync python benchmarks/compare.py \
  /secure/path/GeoLite2-City.mmdb \
  /secure/path/iptools-database-directory \
  --rows 50000 \
  --repeats 5 \
  --workload high \
  --json comparison-high-cardinality.json

POLARS_MAX_THREADS=1 uv run --no-sync python benchmarks/compare.py \
  /secure/path/GeoLite2-City.mmdb \
  /secure/path/iptools-database-directory \
  --rows 50000 \
  --repeats 5 \
  --workload repeated \
  --json comparison-repeated.json
```

The script checks matching, populated fields before timing and refuses runs over
250,000 rows. Both pinned packages have compatible manylinux x86-64 wheels.

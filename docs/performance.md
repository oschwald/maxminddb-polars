# Performance and benchmarks

Use `lookup_path` when one or a few independent leaves are needed. Use a
partial Struct when several related leaves should be returned together; it
performs one search-tree lookup per valid IP and gathers each selected leaf by
unique record offset. Use `lookup` without a dtype when most or all of a record
is needed. If several consumers need fields from a whole record, materialize
the expression once before selecting its Struct fields.

The implementation has no per-row Python calls and uses no JSON intermediate.
Scalar paths use concrete typed builders. Complete standard records use typed
`maxminddb::geoip2` decoders. Partial/custom records and nested paths use the
shared schema projection tree and direct Arrow/Polars Struct/List builders.

## Reproducible fixture baseline

Run:

```console
uv run maturin develop
uv run python benchmarks/lookups.py \
  --rows 10000 \
  --repeats 7 \
  --json benchmark-results.json
```

The script measures a scalar path, three independent paths, a fused
three-field projection, complete City/Country/Enterprise/ASN records, CPU and
wall samples, output size, and peak process RSS. It records OS, Python, Polars,
Rust, package, and fixture revisions. The committed development result is
[`benchmarks/results/development-fixtures.json`](../benchmarks/results/development-fixtures.json).

The recorded repeated-fixture fused/scalar median ratio is 0.87, inside the
initial 1.30 gate. Tiny fixtures heavily favor record-offset deduplication and
are useful for regression detection, not production capacity planning.

## Real City release gate

Run `benchmarks/real_city.py` against a current full City database before an
alpha/beta/RC candidate:

```console
uv run python benchmarks/real_city.py \
  /secure/path/GeoIP2-City.mmdb \
  --rows 50000 \
  --repeats 7 \
  --json real-city-results.json
```

The result file contains metrics and database size, not database contents.
Database files must never be committed or uploaded as workflow artifacts. Runs
over 250,000 whole-City rows require `--allow-large-run` and should be isolated
by an OS/container memory limit. A 33.7 MB real GeoLite2 City database was
exercised in the current readiness pass; its content-free metrics are committed
as `benchmarks/results/real-geolite2-city.json`.

Comparisons with other implementations are informational rather than gates and
must record exact dependency versions and the same inputs.

# Performance and benchmarks

Use `lookup_path` when one or a few independent leaves are needed. Use a
partial Struct when several related leaves should be returned together; it
performs one search-tree lookup per valid IP and gathers each selected leaf by
unique record offset. Use `lookup` without a dtype when most or all of a record
is needed. If several consumers need fields from a whole record, materialize
the expression once before selecting its Struct fields.

The implementation has no per-row Python calls and uses no JSON intermediate.
Scalar paths use concrete typed builders; string and binary values borrow from
the database while Arrow output is built. Batches below 8,192 rows, or with at
most two Polars workers, decode each unique record offset once. Larger scalar
batches decode in parallel with no IP cache and cap every task's temporary
decoded values at 2,048 rows. Adjacent physical input chunks are coalesced into
those logical tasks without copying or rechunking the IP values. Complete
City, Country, Enterprise, ISP, Connection Type, Anonymous IP,
Density/Income, Domain, and ASN records use typed `maxminddb::geoip2` decoders.
The five newer known products, partial/custom records, and nested paths use the
shared schema projection tree and direct Arrow/Polars Struct/List builders.

## Reproducible fixture baseline

Run:

```console
uv run --no-sync maturin develop --release --locked
POLARS_MAX_THREADS=1 uv run --no-sync python benchmarks/lookups.py \
  --rows 10000 \
  --repeats 7 \
  --enforce-gates \
  --json benchmark-results.json
```

The script measures a scalar path, three independent paths, a fused
three-field projection, complete City/Country/Enterprise/ASN records, CPU and
wall samples, output size, and peak process RSS. It records OS, Python, Polars,
Rust, package, and fixture revisions, whether tracked source files were dirty,
and a report schema version. `git_dirty` is `true` or `false` when Git status is
available and `null` when it cannot be determined. Peak RSS is normalized to
KiB on Linux and macOS.
`--enforce-gates` exits unsuccessfully after writing the report if a boolean
gate fails. The committed development result is
[`benchmarks/results/development-fixtures.json`](../benchmarks/results/development-fixtures.json).
The same release-mode, single-thread fixture gate runs weekly and can be
started manually through the `Benchmark` GitHub Actions workflow.

The recorded single-worker repeated-fixture scalar and fused-partial results
are 11.69 and 11.44 million rows per second. The fused/scalar median ratio is
1.02, inside the initial 1.30 gate, and peak process RSS is 109 MiB. Tiny fixtures
heavily favor record-offset deduplication and are useful for regression
detection, not production capacity planning.

## Real City baseline and scaling

Run `benchmarks/real_city.py` with one Polars thread against a current full City
database before an alpha/beta/RC candidate. The single-thread run keeps the
partial/scalar fusion gate comparable across machines and implementation
changes:

```console
POLARS_MAX_THREADS=1 uv run python benchmarks/real_city.py \
  /secure/path/GeoIP2-City.mmdb \
  --rows 50000 \
  --repeats 7 \
  --workload repeated \
  --enforce-gates \
  --json real-city-results.json
```

The default `repeated` workload provides the stable fusion gate. Also run
`--workload half` for the expected roughly 50%-unique workload and
`--workload high` for a distinct mapped-IP stress case. Workload construction
is excluded from the timed plans, and each report records its exact unique-IP
count.

Repeat with the intended production thread count and the desired cardinality
to measure scalar scaling:

```console
POLARS_MAX_THREADS=20 uv run python benchmarks/real_city.py \
  /secure/path/GeoIP2-City.mmdb \
  --rows 50000 \
  --repeats 7 \
  --workload half \
  --json real-city-parallel-results.json
```

The result file contains metrics and database size, not database contents.
Database files must never be committed or uploaded as workflow artifacts. Runs
over 250,000 whole-City rows require `--allow-large-run` and should be isolated
by an OS/container memory limit. A 33.7 MB real GeoLite2 City database was
exercised in clean checkouts recorded by each report. With one thread and six
repeated inputs, scalar, three-field partial, and whole-City throughput was
10.05, 9.68, and 0.69 million rows per second; the partial/scalar ratio was 1.04
and peak RSS was 254 MiB. With 20 threads and 25,000 unique mapped IPs among
50,000 rows, throughput was 20.91, 4.25, and 0.27 million rows per second,
respectively, at 425 MiB peak RSS. The parallel partial/scalar ratio is
informational because only the scalar path uses internal parallelism.
Content-free metrics are committed as
[`real-geolite2-city.json`](../benchmarks/results/real-geolite2-city.json) and
[`real-geolite2-city-parallel.json`](../benchmarks/results/real-geolite2-city-parallel.json).

Comparisons with other implementations are informational rather than gates and
must record exact dependency versions and the same inputs. The current
reproducible results and semantic differences are in
[`comparison.md`](comparison.md).

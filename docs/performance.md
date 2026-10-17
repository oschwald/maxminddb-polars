# Performance and benchmarks

Use `lookup_path` when one or a few independent leaves are needed. Use a
partial Struct when several related leaves should be returned together; it
performs one search-tree lookup per valid IP and gathers each selected leaf by
unique record offset. Use `lookup` without a dtype when most or all of a record
is needed. Selecting Struct fields after a whole-record lookup still decodes
and materializes the full record; select paths or pass a partial dtype to avoid
that work. Polars 2.0 can share repeated deterministic lookup expressions.
Nested field selections can still leave several whole-record lookups in the
optimized plan. This reuse does not push field selections into the decoder.
To avoid repeated whole-record lookups on either version, assign the result to
a column with `with_columns`, then select its Struct fields.

Polars 2.0 defaults lazy queries to streaming. Whole-record and partial-record
lookups can run in streaming batches. Scalar-path lookups retain their existing
registration, which keeps the input Series together for internal parallelism.
Their streaming plans contain an in-memory fallback. Do not assume that
streaming bounds the memory used by a path lookup or by the complete pipeline.

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

Partial and custom Structs flatten to leaf paths. Each leaf navigates its path
independently, reusing the search-tree result, and is decoded once per unique
record offset. List leaves and nested path outputs use a schema-guided decoder
that reads requested fields and skips the rest. The resulting arrays are
gathered into the output Struct. Within each batch, record decoding and
record-offset gathering are serial; internal parallelism is limited to large
scalar-path batches.

## Polars 1.44.2 and 2.0.0 comparison

The comparison covers 240 isolated operation runs measured on 2026-10-17.
Both versions used the same release wheel, built with Rust 1.99.1, on Python
3.13.12 and Linux x86-64 with an Intel Core Ultra 7 265K. Each run measured
50,000 rows seven times after warm-up.
Inputs used one physical chunk so workload construction did not change
parallelism between Polars versions.

The runs cover the test fixtures and a 50.9 MB GeoLite2 City database, with
repeated, 50%-unique, and all-unique City inputs. They compare one and 20 Polars
workers across `auto`, `in-memory`, and `streaming`.

For the 20-worker, 50%-unique City workload, median throughput was:

| Operation                           | Polars 1.44.2 auto | Polars 2.0.0 auto | Polars 2.0.0 in-memory |
| ----------------------------------- | ------------------ | ----------------- | ---------------------- |
| One scalar path                     | 29.28              | 26.95             | 29.76                  |
| Three scalar paths                  | 14.61              | 8.23              | 15.35                  |
| Three-field partial Struct          | 4.70               | 18.38             | 4.60                   |
| Fields selected after a full lookup | 0.15               | 0.62              | 0.16                   |
| Full City record                    | 0.18               | 1.24              | 0.19                   |

Throughput is in millions of rows per second. The streaming default improved
partial and whole-record throughput in this parallel workload. Scalar paths
kept their in-memory fallback and were faster with explicit `in-memory` on
Polars 2.0. With one worker, the streaming default also reduced partial-record
throughput. Select the engine for the workload rather than assuming the new
default is faster.

Higher throughput can use more memory. For full City records in the table,
peak RSS rose from 615 MiB with Polars 1.44.2 `auto` to 1045 MiB with
Polars 2.0.0 `auto`. These peaks include workload construction and warm-up.
Each configuration ran in one process. Repeat measurements on the target
machine before treating small timing differences as regressions.

Expression reuse is separate from streaming parallelism. For `city_fields`,
the optimized in-memory plan contained three whole-record lookup nodes in
Polars 1.44.2 and two in 2.0.0. With one worker and repeated City addresses,
throughput rose from 0.23 to 0.35 million rows per second. The partial Struct
reached 11.42 million rows per second on Polars 2.0.0 in the same engine.
Assigning the lookup to an intermediate column produced one lookup node on
both versions, but still decoded the whole record.

Both versions passed the existing single-worker, in-memory fusion gates on
fixtures and repeated City inputs. Partial/scalar median ratios ranged from
1.03 to 1.08, below the 1.30 limit.

## Reproducible fixture baseline

After [development setup](../CONTRIBUTING.md#setup), build the native extension
in release mode and run:

```console
uv run --no-sync maturin develop --release --locked
POLARS_MAX_THREADS=1 uv run --no-sync python benchmarks/lookups.py \
  --rows 10000 \
  --repeats 7 \
  --engine in-memory \
  --enforce-gates \
  --json benchmark-results.json
```

The script measures a scalar path, three independent paths, a fused
three-field projection, shared City field selections, complete
City/Country/Enterprise/ASN records, CPU and
wall samples, output size, and peak process RSS. It records OS, Python, Polars,
Rust, package, and fixture revisions, whether tracked source files were dirty,
and a report schema version. `git_dirty` is `true` or `false` when Git status is
available and `null` when it cannot be determined. Peak RSS is normalized to
KiB on Linux and macOS. Every report records its workload, total rows, distinct
non-null IP strings, and null input rows.
`--enforce-gates` exits unsuccessfully after writing the report if a boolean
gate fails. The committed development result is
[`benchmarks/results/development-fixtures.json`](../benchmarks/results/development-fixtures.json).
The release-mode, single-thread fixture gate also runs weekly and can be
started manually through the `Benchmark` GitHub Actions workflow, which defaults
to 100,000 rows and seven repeats. The gate uses the in-memory engine to keep
the execution mode comparable across Polars versions.

Both lookup benchmark scripts accept `--engine auto|in-memory|streaming` and
record the requested engine and `POLARS_ENGINE_AFFINITY`. `auto` follows the
installed Polars version and any engine affinity setting. Use an explicit
engine when comparing the same execution mode across versions.

Use `--operation scalar`, `--operation three_paths`, `--operation partial`, or
`--operation city_fields` to measure one operation per process. Full City
records use `--operation city` in `lookups.py` and `--operation whole_city` in
`real_city.py`. Separate processes prevent a previous operation from setting
the peak RSS for the next operation. Peak RSS still includes interpreter,
database, workload construction, and warm-up memory. Single-operation runs
cannot use `--enforce-gates`, which compares scalar and partial lookups.

The committed results were measured during `0.1.3` development, with package
metadata still at `0.1.2`. Each report records its dependencies and source
revision. Use these historical baselines for context and record fresh results
for the version and database being evaluated.

The recorded single-worker repeated-fixture scalar and fused-partial results
are 11.86 and 10.89 million rows per second. The fused/scalar median ratio is
1.09, inside the initial 1.30 gate, and peak process RSS is 111 MiB. Tiny fixtures
heavily favor record-offset deduplication and are useful for regression
detection, not production capacity planning.

## Real City baseline and scaling

Run `benchmarks/real_city.py` with one Polars thread against a current full City
database when validating a performance-sensitive change or release candidate.
The single-thread run keeps the partial/scalar fusion gate comparable across
machines and implementation changes:

```console
POLARS_MAX_THREADS=1 uv run --no-sync python benchmarks/real_city.py \
  /secure/path/GeoIP2-City.mmdb \
  --rows 50000 \
  --repeats 7 \
  --workload repeated \
  --engine in-memory \
  --enforce-gates \
  --json real-city-results.json
```

The default `repeated` workload provides the stable fusion gate. Also run
`--workload half` for the expected roughly 50%-unique workload and
`--workload high` for a distinct mapped-IP stress case. Workload construction
is excluded from the timed plans, and each report records its exact non-null
unique-IP and null-row counts.

Repeat with the intended production thread count and the desired cardinality
to measure scalar scaling:

```console
POLARS_MAX_THREADS=20 uv run --no-sync python benchmarks/real_city.py \
  /secure/path/GeoIP2-City.mmdb \
  --rows 50000 \
  --repeats 7 \
  --workload half \
  --json real-city-parallel-results.json
```

The result file contains metrics and database size, not database contents.
Database files must never be committed or uploaded as workflow artifacts. Runs
over 250,000 whole-City rows require `--allow-large-run` and should be isolated
by an OS/container memory limit. In the historical `0.1.3` development runs, a
33.7 MB real GeoLite2 City database was exercised in the clean checkouts recorded
by each report. With one thread, five repeated non-null IP values, and null
rows, scalar, three-field partial, and
whole-City throughput was 9.92, 9.59, and 0.70 million rows per second; the
partial/scalar ratio was 1.03 and peak RSS was 252 MiB. With 20 threads and
25,000 unique mapped IPs among 50,000 rows, throughput was 23.89, 4.36, and 0.26
million rows per second, respectively, at 423 MiB peak RSS. The parallel
partial/scalar ratio is
informational because only the scalar path uses internal parallelism.
Content-free metrics are committed as
[`real-geolite2-city.json`](../benchmarks/results/real-geolite2-city.json) and
[`real-geolite2-city-parallel.json`](../benchmarks/results/real-geolite2-city-parallel.json).

Comparisons with other implementations are informational rather than gates and
must record exact dependency versions and the same inputs. The recorded
comparison results and semantic differences are in
[`comparison.md`](comparison.md).

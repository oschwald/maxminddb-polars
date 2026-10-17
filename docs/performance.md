# Performance and benchmarks

Choose the lookup that matches the fields you need:

- Use `lookup_path` for one or a few independent fields.
- Use `lookup` with a [partial dtype](custom-schemas.md) for several related
  fields. They share one database search per valid IP.
- Use `lookup` without a dtype when you need most or all of a record.

Selecting fields after a full lookup still decodes the whole record. Polars 2.0
can reuse some repeated expressions, but does not pass those field selections
to the decoder. To avoid repeating a full lookup, assign it to a column with
`with_columns`, then select its fields.

Polars 2.0 defaults lazy queries to streaming. Whole-record and partial-record
lookups can run in streaming batches. Scalar-path lookups use an in-memory fallback
to preserve their internal parallelism. Streaming does not guarantee a memory
bound for a path lookup or the complete query.

## Polars 1.44.2 and 2.0.0 comparison

With 20 workers and 50% unique City addresses, Polars 2.0's streaming default
improved partial and whole-record throughput. Scalar paths were faster with
explicit `in-memory`. Median throughput, in millions of rows per second:

| Operation                           | Polars 1.44.2 auto | Polars 2.0.0 auto | Polars 2.0.0 in-memory |
| ----------------------------------- | ------------------ | ----------------- | ---------------------- |
| One scalar path                     | 29.28              | 26.95             | 29.76                  |
| Three scalar paths                  | 14.61              | 8.23              | 15.35                  |
| Three-field partial Struct          | 4.70               | 18.38             | 4.60                   |
| Fields selected after a full lookup | 0.15               | 0.62              | 0.16                   |
| Full City record                    | 0.18               | 1.24              | 0.19                   |

The speedup used more memory. For full City records, peak resident memory (RSS)
rose from 615 MiB with Polars 1.44.2 `auto` to 1045 MiB with Polars 2.0.0 `auto`.
With one worker, streaming also reduced partial-record throughput. Measure your
own workload before choosing an engine or treating small differences as regressions.

Expression reuse helped less than reading fewer fields. The `city_fields`
query kept three full lookups in Polars 1.44.2 and two in 2.0.0. With one worker,
repeated City addresses, and `in-memory`, throughput rose from 0.23 to 0.35
million rows per second. A partial Struct reached 11.42 million on Polars 2.0.0.
Assigning the full lookup to an intermediate column gave one lookup on either
version, but still decoded the whole record.

The 2026-10-17 comparison covered 240 configurations, each in a fresh process:

- The same release wheel, built with Rust 1.99.1 and tested on Python 3.13.12
  and Linux x86-64 with an Intel Core Ultra 7 265K.
- Test fixtures and a 50.9 MB GeoLite2 City database, with repeated, 50%-unique,
  and all-unique City inputs.
- One and 20 workers with `auto`, `in-memory`, and `streaming`.
- 50,000 rows in one physical chunk, seven timed runs after warm-up. Memory
  peaks include input construction and warm-up.

Both versions passed the single-worker, in-memory performance checks. The
partial/scalar time ratios were 1.03–1.08, below the 1.30 limit.

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

The script measures paths, partial records, field selections, and full records.
Reports include timings, output size, peak RSS in KiB, workload details, source
and dependency versions, and whether tracked files had local changes.

`--enforce-gates` requires the median partial lookup time to be at most 1.30
times the scalar lookup time. On failure, the script writes the report and exits
unsuccessfully. The same check runs weekly and on demand in the `Benchmark`
workflow, using 100,000 rows, seven repeats, one worker, and `in-memory`.

Both lookup scripts accept `--engine auto|in-memory|streaming`. `auto` follows
the installed Polars version and `POLARS_ENGINE_AFFINITY`. Use an explicit
engine to compare the same execution mode across versions.

For separate memory measurements, run one operation per process with
`--operation scalar`, `--operation three_paths`, `--operation partial`, or
`--operation city_fields`. For full City records, use `--operation city` in
`lookups.py` or `--operation whole_city` in `real_city.py`. RSS includes the
interpreter, database, input construction, and warm-up. Single-operation runs
cannot use `--enforce-gates`.

The [historical fixture report](../benchmarks/results/development-fixtures.json)
recorded 11.86 million scalar and 10.89 million partial lookups per second, with
111 MiB peak RSS. Small fixtures favor repeated records. Use them to detect
regressions, then measure a full database for production planning.

## Real City baseline and scaling

Before a performance-sensitive change or release, run against a current City
database with one worker:

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

Use `repeated` for the performance check, `half` for roughly 50% unique IPs, and
`high` for distinct mapped IPs. Input construction is excluded from timings.

Repeat with your production thread count and mix of IPs:

```console
POLARS_MAX_THREADS=20 uv run --no-sync python benchmarks/real_city.py \
  /secure/path/GeoIP2-City.mmdb \
  --rows 50000 \
  --repeats 7 \
  --workload half \
  --json real-city-parallel-results.json
```

Reports contain metrics, not database contents. Never commit or upload database
files as workflow artifacts. Runs over 250,000 whole-City rows require
`--allow-large-run` and should use an OS or container memory limit.

Historical results used a 33.7 MB GeoLite2 City database. Throughput is in
millions of rows per second:

| Workload                                     | Scalar | Partial | Full City | Peak RSS |
| -------------------------------------------- | -----: | ------: | --------: | -------: |
| One worker, five repeated IPs and nulls      |   9.92 |    9.59 |      0.70 |  252 MiB |
| 20 workers, 25,000 unique IPs in 50,000 rows |  23.89 |    4.36 |      0.26 |  423 MiB |

The [single-worker](../benchmarks/results/real-geolite2-city.json),
[parallel](../benchmarks/results/real-geolite2-city-parallel.json), and fixture
reports above date from `0.1.3` development, with package metadata at `0.1.2`.
Each records its source revision and dependencies.
See [Comparison with other integrations](comparison.md) for external benchmarks.

## How lookups run

Lookups build Arrow output directly, with no per-row Python calls or JSON.
Partial and custom records share the database search, then decode each selected
field once per unique record offset within a batch. Their decoding is serial
within each batch.

Scalar paths also decode each unique record offset once in batches below
8,192 rows or with at most two workers. Larger batches with more workers decode
in parallel without an IP cache. Each task holds at most 2,048 rows of temporary
decoded values. Small input chunks are combined into tasks without copying IPs.

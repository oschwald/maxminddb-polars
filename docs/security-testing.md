# Security testing

MMDB files and IP strings are treated as untrusted input. Native MMDB open,
lookup, and decode failures become Polars errors; parser panics must never
unwind across the plugin boundary. Null inputs and lookup misses produce nulls.
`strict=False` additionally turns invalid IP strings into nulls, but does not
suppress database or decoder errors.

Python-side expression construction validates arguments and filesystem paths
before invoking the plugin. Invalid arguments can raise `TypeError` or
`ValueError`, and a missing database path raises `FileNotFoundError`, regardless
of strictness.

The normal Rust suite exercises the pinned upstream corruption corpus: MMDB
files under `bad-data` and the broken/invalid fixtures under `test-data`.
It also retains the first fuzz-discovered decoder-overflow input as
a base64 regression fixture. Tests exercise the same cached-reader and path
lookup entry points used by the expression plugin. MMDB open, lookup, and decode
operations contain upstream parser panics and report a Polars error instead of
unwinding across the plugin boundary. Property tests generate
random row order, null, miss, invalid-IP, duplicate-offset, gather-map, and
extreme path-index cases. The optimized nested gather is compared with a
simple row-wise reference implementation.

The custom decoder caps untrusted initial container allocation hints. Larger
legitimate Lists continue to grow normally. The underlying `maxminddb` decoder
also bounds data access, pointer traversal, and nesting depth.

The upstream decoder also limits decoded container values and aggregate
string/byte payloads, including shared budgets for path navigation and the
selected value. Metadata has resource limits as well. Oversized records or
metadata may be structurally valid but still exceed these limits; the plugin
reports a Polars `ComputeError` even with `strict=False`. Regression tests
cover both rejected inputs and records at the supported limits, plus valid
empty containers at the end of metadata.

## Fuzzing

Four `cargo-fuzz` targets live under `fuzz/`:

- `kwargs_deserialization` covers recursive schema and plugin-kwargs JSON;
- `path_traversal` covers generated schema/path combinations and integer
  indexes;
- `projected_value` covers schema-guided decoding from mutated City databases;
- `malformed_database` covers reader creation, IPv4/IPv6 search, and arbitrary
  value decoding.

Install the `cargo-fuzz` version pinned by
[`fuzz.yml`](../.github/workflows/fuzz.yml) and use a nightly Rust toolchain:

```console
cargo +nightly fuzz run malformed_database -- -max_total_time=300
```

Pull requests and pushes to `main` that change Rust sources, Cargo metadata,
fuzz files, or the fuzz workflow run each target for ten seconds. Manual runs
use the same interval; the weekly job runs each target for two minutes. Every
run seeds database targets from the pinned valid/corrupt fixture corpus and
retains crash artifacts on failure. Promote every reproducible crash to a
permanent unit fixture before clearing the artifact.

Dependency auditing covers both root and fuzz lockfiles. Temporary transitive
advisory exceptions, their scope, and their expiry are recorded in
[`maintaining.md`](maintaining.md).

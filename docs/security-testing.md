# Security testing

MMDB files and IP strings are treated as untrusted input. Decode, path,
schema, and I/O failures must become Polars errors or nulls according to the
documented strictness rules; they must never unwind across the plugin boundary.

The normal Rust suite exercises every MMDB file in the pinned upstream
corruption corpus: 21 files under `bad-data` and four broken/invalid files under
`test-data`. It also retains the first fuzz-discovered decoder-overflow input as
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

## Fuzzing

Four `cargo-fuzz` targets live under `fuzz/`:

- `kwargs_deserialization` covers recursive schema and plugin-kwargs JSON;
- `path_traversal` covers generated schema/path combinations and integer
  indexes;
- `projected_value` covers schema-guided decoding from mutated City databases;
- `malformed_database` covers reader creation, IPv4/IPv6 search, and arbitrary
  value decoding.

Install `cargo-fuzz` 0.13.2 and use a current nightly Rust toolchain:

```console
cargo +nightly install cargo-fuzz --version 0.13.2 --locked
cargo +nightly fuzz run malformed_database -- -max_total_time=300
```

Pull requests and pushes run each target for a short bounded interval. The
weekly job runs each target for two minutes, starts database targets from the
pinned valid/corrupt fixture corpus, and retains crash artifacts. Promote every
reproducible crash to a permanent unit fixture before clearing the artifact.

Dependency auditing covers both root and fuzz lockfiles. Temporary transitive
advisory exceptions, their scope, and their expiry are recorded in
[`maintaining.md`](maintaining.md).

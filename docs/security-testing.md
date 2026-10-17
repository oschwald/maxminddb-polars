# Security testing

MMDB files and IP strings are untrusted input. Failures to open, search, or
decode a database become Polars errors. Parser panics must not cross the plugin
boundary. `strict=False` converts invalid IP strings to nulls but does not
suppress database or decoder errors. See [API error behavior](api.md#dtypes-and-validity).

Python checks arguments and file paths before calling the plugin. Invalid
arguments can raise `TypeError` or `ValueError`. A missing database raises
`FileNotFoundError`, regardless of strictness.

The Rust tests use the plugin's reader and lookup entry points to check:

- Upstream corrupt databases in `bad-data` and broken fixtures in `test-data`.
- A saved decoder-overflow input found by fuzzing, including panic containment.
- Generated row orders, nulls, misses, invalid IPs, shared records, output
  mappings, and extreme path indexes.
- Nested output assembly against a simple row-by-row reference implementation.

The custom decoder limits initial allocations requested by the file. Valid Lists
can still grow normally. The `maxminddb` decoder limits data access, pointer
traversal, nesting, container values, and total string/byte payloads. Path
navigation and the selected value share a budget. Metadata has limits too.

Even valid records can exceed these limits and raise `ComputeError`, including
with `strict=False`. Tests cover rejected inputs, records at the limits, and
empty containers at the end of metadata.

## Fuzzing

Four `cargo-fuzz` targets live under `fuzz/`:

- `kwargs_deserialization` covers recursive schema and plugin-kwargs JSON.
- `path_traversal` covers generated schema/path combinations and integer
  indexes.
- `projected_value` covers schema-guided decoding from mutated City databases.
- `malformed_database` covers reader creation, IPv4/IPv6 search, and arbitrary
  value decoding.

Install the `cargo-fuzz` version pinned by
[`fuzz.yml`](../.github/workflows/fuzz.yml) and use a nightly Rust toolchain:

```console
cargo +nightly fuzz run malformed_database -- -max_total_time=300
```

Relevant pull requests and pushes to `main` run each target for ten seconds,
as do manual runs. The weekly job runs each for two minutes. Runs seed database
targets with valid and corrupt fixtures, and save crashes as artifacts. Add each
reproducible crash to the unit tests before clearing its artifact.

Dependency audits cover root and fuzz lockfiles. See
[Maintainer operations](maintaining.md#temporary-security-exceptions) for exceptions.

# 0.1 release readiness

The public API and implementation are ready for the `0.1.0` release. The
release branch changes the placeholder version only after every preparation
check passes, and the final release sequence below starts only after that
branch has passed CI and merged.

## Completed gates

- The public surface is frozen by [ADR 0001](adr/0001-public-api.md) and an
  executable `__all__` regression test.
- All nine standard record schemas, metadata aliases, scalar and nested paths,
  custom schemas, and validated partial schemas have eager, lazy, streaming,
  cross-platform, dtype, null, strictness, concurrency, and snapshot coverage.
- Twenty-one Rust tests include differential property tests, every one of the
  25 pinned corrupt/broken MMDB fixtures, and a fuzz-discovered parser-panic
  regression. Forty-eight Python tests cover the Python/plugin boundary and
  frozen public surface.
- Four fuzz targets cover kwargs/schema deserialization, path traversal,
  schema-guided decoding, and malformed databases. Pull requests and pushes
  run bounded smoke fuzzing; the weekly schedule runs the seeded corpus longer.
- Cargo, Python, CodeQL, dependency-review, and workflow-security checks are
  configured. Temporary transitive Rust advisory exceptions have exact scope,
  rationale, and a 2026-09-30 re-evaluation deadline in
  [`maintaining.md`](maintaining.md).
- The seven-target abi3 wheel matrix and sdist workflow install native wheels
  on Linux, macOS, and Windows; test Python 3.10 and 3.14 against the same Linux
  wheel; inspect package contents, licenses, and shared libraries; build from
  the sdist; run public lookups; generate checksums; and retain the exact
  publishable artifacts.
- The Cargo crate is packaged and compiled in release rehearsals. Published
  GitHub releases send it to crates.io through OIDC trusted publishing and send
  the already-validated Python distributions to PyPI independently.
- A local no-publish rehearsal passed `scripts/check`, a locked release wheel
  and sdist build, artifact inspection, and strict `twine check`. The exact
  wheel then installed into a clean temporary Python 3.13/Polars 1.43.2
  environment and passed partial/path streaming lookups outside the checkout.
- A 33.7 MB real GeoLite2 City database passed the scalar/partial/whole-record
  baseline at 277 MiB peak RSS. The fused partial/scalar ratio was 1.125, inside
  the 1.30 gate. Whole-record memory now uses Arrow gathers rather than cloning
  recursive records per output row.
- The final pinned comparison covers both `polars-maxminddb` and
  `polars-iptools`, validates populated overlapping output, records their
  missing-value semantic differences, and publishes content-free results.

## Known, accepted constraints

- Python Polars support is intentionally `>=1.43.2,<1.44`; native plugin ABI
  updates require coordinated Rust Polars and `pyo3-polars` changes.
- The visible PyO3 and bincode advisories are unreachable transitive APIs under
  the current compatibility pair. The disabled-cloud `quick-xml` dependency is
  absent from built targets. These exceptions remain visible and time-bounded.
- On this development machine, a one-job debug Rust test link peaks near
  3.1 GiB and the release build process near 1.9 GiB. Runtime comparison and
  real-database workloads peak below 280 MiB. Local validation should keep
  Cargo at one job when memory is constrained.
- On high-cardinality input, `polars-iptools.full` is faster than this package's
  fused three-field partial lookup. This is documented and does not violate an
  internal release gate; the partial lookup is faster on repeated offsets and
  retains general schemas and null semantics.

## Final release sequence

After the release branch passes CI and merges:

- bootstrap the first crates.io version with a narrowly scoped API token;
- configure the crates.io trusted publisher for `release.yml` and the
  `release` environment, then remove the bootstrap token;
- confirm the PyPI pending Trusted Publisher and maintainer 2FA checklist;
- create the `v0.1.0` GitHub release from the verified `main` merge commit;
- follow the release workflow while it publishes to crates.io and PyPI; and
- verify both registry pages, checksums, provenance, and clean installations.

# 0.1 release record

The public `0.1` line launched on 2026-08-23. This document records the gates
used for the initial release; current release instructions live in
[`CONTRIBUTING.md`](../CONTRIBUTING.md), and current repository and publisher
settings live in [`maintaining.md`](maintaining.md).

## Completed gates

- The public surface is frozen by [ADR 0001](adr/0001-public-api.md) and an
  executable `__all__` regression test.
- All 14 standard database schemas, metadata aliases, scalar and nested paths,
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
  configured. Transitive Rust advisory exceptions have exact scope, rationale,
  and a 2026-09-30 re-evaluation deadline in [`maintaining.md`](maintaining.md).
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

## Current constraints

- Python Polars support is intentionally `>=1.43.2,<1.44`; native plugin ABI
  updates require coordinated Rust Polars and `pyo3-polars` changes.
- Polars still pulls in unmaintained `bincode` 2.0.1. The disabled-cloud
  `quick-xml` dependency is absent from built targets. These exceptions remain
  documented and time-bounded.
- Full Polars debug data previously produced a 32 GiB accumulated local target
  and a one-job test link near 3.1 GiB. The bounded dev/test profile disables
  debug data and incremental caches; a fresh no-run test target was 1.8 GiB.
  A cold Polars compile can still exceed a 2.5 GiB cgroup, so constrained local
  validation should keep Cargo at one job and allow roughly 3.5 GiB. The
  50%-unique 50,000-row City benchmark peaked near 423 MiB.
- External performance comparisons are informational rather than release gates;
  the current pinned results and semantic differences are documented in
  [`comparison.md`](comparison.md).

## Published releases

`v0.1.0` allocated the package name on both crates.io and PyPI. Subsequent
releases use short-lived OIDC credentials for both registries and require no
repository upload tokens.

`v0.1.1` corrected links in the PyPI description and hardened release
verification. `v0.1.2` moved to Polars 0.55 and PyO3 0.29, resolving the two
PyO3 security advisories accepted for the initial release.

## Unreleased validation on 2026-08-25

The post-`0.1.2` roadmap branch completed a no-publish rehearsal under a
3.5 GiB cgroup with swap disabled and one Cargo build job:

- `scripts/check` passed all repository formatting, lint, type, metadata,
  lockfile, submodule, and documentation gates, plus 31 Rust tests and 78 Python
  tests.
- `cargo publish --dry-run --locked --allow-dirty` packaged and compiled the
  crate; the upload step was intentionally aborted by Cargo's dry-run mode.
- A locked release wheel and sdist passed repository content inspection and
  strict Twine metadata checks.
- The wheel installed into a clean Python 3.13/Polars 1.43.2 environment and
  passed streaming City and inferred IP Risk lookups outside the checkout.
- The sdist performed a cold release build, peaking near 2.75 GiB, then
  installed into a separate clean environment and passed an external City
  lookup.

No tag, GitHub release, registry upload, or publication was created.

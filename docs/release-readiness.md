# Historical 0.1 release record

The public `0.1` line launched on 2026-08-23. This document records validation
across `0.1.0` through `0.1.3`, including work added after the initial release.
Test counts, dependency versions, and performance measurements below describe
those historical candidates.

Current release instructions live in [`CONTRIBUTING.md`](../CONTRIBUTING.md),
supported versions in [`compatibility.md`](compatibility.md), and repository
and publisher settings in [`maintaining.md`](maintaining.md).

## Gates reached during the 0.1 release line

- The public surface was covered by an executable `__all__` regression test.
  The supported operations are documented in the [API reference](api.md).
- All 14 standard database schemas, metadata aliases, scalar and nested paths,
  custom schemas, and validated partial schemas had eager, lazy, streaming,
  cross-platform, dtype, null, strictness, concurrency, and snapshot coverage.
- Thirty-one Rust tests included differential property tests, every one of the
  25 pinned corrupt/broken MMDB fixtures, and a fuzz-discovered parser-panic
  regression. Seventy-eight Python tests covered the Python/plugin boundary and
  frozen public surface.
- Four fuzz targets covered kwargs/schema deserialization, path traversal,
  schema-guided decoding, and malformed databases. Pull requests and pushes
  ran bounded smoke fuzzing; the weekly schedule ran the seeded corpus longer.
- Cargo, Python, CodeQL, dependency-review, and workflow-security checks were
  configured. Transitive Rust advisory exceptions had exact scope, rationale,
  and a 2026-09-30 re-evaluation deadline. Current exceptions are in
  [`maintaining.md`](maintaining.md).
- The seven-target abi3 wheel matrix and sdist workflow installed native wheels
  on Linux, macOS, and Windows; tested Python 3.10 and 3.14 against the same Linux
  wheel; inspected package contents, licenses, and shared libraries; built from
  the sdist; ran public lookups; generated checksums; and retained the exact
  publishable artifacts.
- The Cargo crate was packaged and compiled in release rehearsals. Published
  GitHub releases sent it to crates.io through OIDC trusted publishing and sent
  the already-validated Python distributions to PyPI independently.
- A local no-publish rehearsal passed `scripts/check`, a locked release wheel
  and sdist build, artifact inspection, and strict `twine check`. The exact
  wheel then installed into a clean temporary Python 3.13/Polars 1.43.2
  environment and passed partial/path streaming lookups outside the checkout.
- A 33.7 MB real GeoLite2 City database passed the scalar/partial/whole-record
  baseline at 277 MiB peak RSS. The fused partial/scalar ratio was 1.125, inside
  the 1.30 gate. Whole-record memory used Arrow gathers rather than cloning
  recursive records per output row.
- The pinned comparison covered both `polars-maxminddb` and `polars-iptools`,
  validated populated overlapping output, recorded their missing-value semantic
  differences, and published content-free results.

## Constraints recorded for 0.1.3

- Python Polars support was `>=1.43.2,<1.44`; `0.2.0` later added Polars 1.44.
  Native plugin ABI updates required coordinated Rust Polars and `pyo3-polars`
  changes.
- Polars pulled in unmaintained `bincode` 2.0.1. The disabled-cloud `quick-xml`
  dependency was absent from built targets. These exceptions were documented
  and time-bounded.
- Full Polars debug data previously produced a 32 GiB accumulated local target
  and a one-job test link near 3.1 GiB. The bounded dev/test profile disabled
  debug data and incremental caches; a fresh no-run test target was 1.8 GiB.
  A cold Polars compile still exceeded a 2.5 GiB cgroup, so constrained local
  validation used one Cargo job and roughly 3.5 GiB. The
  50%-unique 50,000-row City benchmark peaked near 423 MiB.
- External performance comparisons were informational rather than release gates;
  the recorded results and semantic differences are documented in
  [`comparison.md`](comparison.md).

## Release history

`v0.1.0` allocated the package name on both crates.io and PyPI. Subsequent
releases use short-lived OIDC credentials for both registries and require no
repository upload tokens.

`v0.1.1` corrected links in the PyPI description and hardened release
verification. `v0.1.2` moved to Polars 0.55 and PyO3 0.29, resolving the two
PyO3 security advisories accepted for the initial release.

`v0.1.3` added five inferred database schemas, improved scalar lookup throughput
and memory use, bounded the reader cache and local build artifacts, and made
cache identities robust to atomic database replacement on Unix and Windows.

## 0.1.3 validation on 2026-08-25

The `0.1.3` candidate completed release validation under a 3.5 GiB cgroup with
swap disabled and one Cargo build job:

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

That release used a separate preparation stage that created no tag or registry
upload; those actions were reserved for the verified merge commit. The current
helper uses a single invocation from a release branch, as described in
[`CONTRIBUTING.md`](../CONTRIBUTING.md#releases).

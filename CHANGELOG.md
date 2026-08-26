# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added inferred whole-record and path schemas for Residential Proxy,
  Anonymous Plus, IP Risk, Static IP Score, and User Count databases, with
  stable public dtype constants.

### Fixed

- Prevented same-path, same-size database replacements with preserved
  modification times from reusing stale cached bytes.

### Changed

- Improved scalar lookup throughput and memory use with borrowed string and
  binary decoding, Polars-native offset hashing, and bounded parallel execution
  for large batches, including coalescing highly fragmented inputs into bounded
  logical tasks without copying their values.
- Bounded retained MMDB reader snapshots to 512 MiB by default, with an
  environment override and explicit errors for evicted obsolete generations.
- Bounded local Rust build artifacts by disabling dev/test debug data and
  incremental caches, and made the full local check default to one Cargo job.

## [0.1.2] - 2026-08-24

### Changed

- Updated the Rust Polars/PyO3 compatibility stack to Polars 0.55 and PyO3
  0.29, resolving the PyO3 iterator-bounds and closure thread-safety
  advisories.

## [0.1.1] - 2026-08-24

### Fixed

- Made README documentation and policy links work when rendered on PyPI.
- Prevented the release verification checkout from shadowing the installed
  wheel.

## [0.1.0] - 2026-08-23

### Added

- Initialized the standalone Rust/Python Polars expression-plugin project.
- Added scalar `lookup_path` expressions and the `.mmdb.lookup_path` namespace.
- Added known City/ASN dtype inference, explicit scalar dtypes for custom MMDBs,
  strictness controls, offset deduplication, and generation-safe reader caching.
- Added whole-record lookups, all nine standard MaxMind record families,
  metadata aliases, direct nested Polars builders, and public schema constants.
- Added schema-guided custom and partial Struct/List decoding with mapping and
  `pl.Struct` parity, planning-time known-schema validation, and fused lookups.
- Added reproducible fixture, real City, and pinned competitor benchmarks with
  peak-memory reporting and bounded large-run controls.
- Added property tests, the complete upstream corruption corpus, four fuzz
  targets, bounded CI fuzzing, custom-decoder allocation hardening, and panic
  containment for corrupt input that triggers upstream parser failures.
- Added API, compatibility, performance, comparison, and security-testing
  documentation and froze the public `0.1` API.
- Added crates.io packaging and OIDC trusted publishing for the native
  implementation crate.

### Changed

- Updated `maxminddb` to 0.30.1, which reports malformed extended data types as
  decoder errors instead of panicking on integer overflow.
- Set the supported Polars floor to 1.43.2 because the 1.43.0 and 1.43.1
  distributions are yanked on PyPI.

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
  targets, bounded CI fuzzing, and custom-decoder allocation hardening.
- Added API, compatibility, performance, comparison, and security-testing
  documentation and froze the public `0.1` API.

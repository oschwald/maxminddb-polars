# Maintainer operations

This file records repository settings that cannot be enforced by committed
files. Re-audit every item before the first public release and after ownership
or publishing changes.

## Bootstrap record

- Repository owner: `oschwald`
- Repository name: `maxminddb-polars`
- Visibility: public
- Default branch: `main`
- License: ISC
- Rust release toolchain: 1.96.0
- Python floor: 3.10
- Python Polars range: 1.43.x
- Rust Polars/pyo3-polars: 0.54.4/0.27.0
- maxminddb: 0.30.0
- MaxMind-DB fixture commit: `e1120013c4b5cbc830b958b2b7e73fba444d316d`

Exact-name checks on 2026-08-23 found no `maxminddb-polars` project on PyPI,
crates.io, npm, or `oschwald` GitHub. These observations do not reserve names.

## GitHub checklist

- [ ] `main` is the default branch and Issues are enabled.
- [ ] Dependency graph, Dependabot alerts, and security updates are enabled.
- [ ] Private vulnerability reporting is enabled.
- [ ] Secret scanning and push protection are enabled when available.
- [ ] Code scanning is enabled and the CodeQL workflow is green.
- [ ] The default Actions token permission is read-only.
- [ ] Workflows cannot approve pull requests.
- [ ] `main` blocks force pushes and deletion and requires pull requests.
- [ ] Stable lint, Rust test, Python smoke, and artifact-metadata checks are required.
- [ ] A `v*` tag ruleset blocks tag updates and deletion.
- [ ] Artifact retention is long enough to diagnose release failures.
- [ ] Allowed merge methods are recorded here after review.

## Publishing checklist

- [ ] A protected `pypi` environment exists with no long-lived upload token.
- [ ] The pending PyPI Trusted Publisher uses owner `oschwald`.
- [ ] Its repository is `maxminddb-polars`.
- [ ] Its workflow is `release.yml`.
- [ ] Its environment is `pypi`.
- [ ] Its PyPI project name is `maxminddb-polars`.
- [ ] Maintainer accounts have two-factor authentication and recovery access.
- [ ] A no-publish artifact and release rehearsal has passed.

The pending publisher does not reserve the PyPI project name. Do not publish an
empty placeholder release.

## Temporary security exceptions

PyO3 0.28 is currently required by the Polars 1.43-compatible
`pyo3-polars`/Rust Polars pair. `cargo audit` temporarily ignores these
advisories in `.cargo/audit.toml`:

- `RUSTSEC-2025-0141`: unmaintained `bincode` 2.0.1 is a transitive dependency
  of Rust Polars 0.54.
- `RUSTSEC-2026-0176`: the project does not call `nth` or `nth_back` on PyO3
  list or tuple iterators.
- `RUSTSEC-2026-0177`: the project does not construct `PyCFunction` closures.

The PyO3 fixes require PyO3 0.29, which in turn requires pyo3-polars 0.28 and
Rust Polars 0.55; that Polars update also removes the current `bincode`
dependency. Re-evaluate all three exceptions with that coordinated
compatibility update, or by 2026-09-30, whichever comes first. The
corresponding Dependabot alerts remain open so the exceptions stay visible.

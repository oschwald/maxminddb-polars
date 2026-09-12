# Contributing to maxminddb-polars

Thank you for helping improve `maxminddb-polars`. API proposals are welcome;
performance and schema changes should include tests and evidence.

## Prerequisites

- Git
- the Rust toolchain selected by `rust-toolchain.toml`
- a standard CPython version from the
  [compatibility matrix](.github/workflows/compatibility.yml)
- [uv](https://docs.astral.sh/uv/)
- [Precious](https://github.com/houseabsolute/precious)
- Node.js and npm; npm installs the pinned Prettier version for Markdown and
  YAML formatting

## Setup

```console
git submodule update --init --recursive
uv sync --all-extras --locked --no-install-project
uv run --no-sync maturin develop
npm clean-install --ignore-scripts
```

The MaxMind-DB submodule contains test fixtures. Do not commit proprietary or
locally installed `.mmdb` databases.

The default dev/test Cargo profile omits debug data and incremental caches to
keep Polars build artifacts bounded, and `scripts/check` defaults Cargo to one
build job. A machine with ample memory may override `CARGO_BUILD_JOBS`.
For an investigation that needs Rust debug information and incremental state,
opt in explicitly, for example:

```console
CARGO_PROFILE_DEV_DEBUG=1 \
  CARGO_PROFILE_TEST_DEBUG=1 \
  CARGO_INCREMENTAL=1 \
  cargo test --locked
```

To enable the repository's pre-commit hook:

```console
git config core.hooksPath .githooks
```

## Tests and checks

Run the local development checks with:

```console
scripts/check
```

The script synchronizes the development environment, rebuilds the native
extension, installs Prettier, and runs formatting, lint, type, Rust/Python test,
Rustdoc, metadata, lockfile, and fixture-revision checks. A cold Polars build can
take substantial time and memory.

After setup, common focused checks are:

```console
uv run --no-sync pytest
cargo test --locked
precious lint --all
```

See [`scripts/check`](scripts/check) and [`precious.toml`](precious.toml) for
the full commands and fixture-revision assertion. The release helper's Bash
integration tests run with the Python suite on Unix; they use temporary Git
repositories and stub build/publication tools.

Run `precious tidy --all` to apply supported formatting fixes. Benchmarks must
use release builds and should compare the candidate against a named baseline on
the same machine. Never commit a full licensed database or benchmark output
containing database contents.

Benchmark reports include the source revision, tracked-worktree state, report
schema version, workload, row count, distinct non-null IP count, null-row count,
and platform-normalized peak RSS. Use
`--enforce-gates` for gating runs; use the explicit `repeated`, `half`, and
`high` cardinality modes in `benchmarks/real_city.py` when evaluating lookup
strategy changes.

## Pull requests

- Keep commits focused and use imperative commit subjects.
- Add tests for behavior changes and regression fixes.
- Update `CHANGELOG.md` under `Unreleased` for user-visible changes.
- Update documentation and type hints with public API changes.
- Include before/after benchmark results for performance-sensitive code.
- Keep `Cargo.lock`, `uv.lock`, and generated state synchronized.

The Rust `polars` and `pyo3-polars` dependencies must move together and must be
validated against the declared Python Polars versions using built wheels.

## Releases

The release helper requires Bash, Perl, and an authenticated GitHub CLI (`gh`),
in addition to the development prerequisites and Git push access to `origin`.

Create a release branch from `origin/main` (any name other than `main`), move
the `Unreleased` changelog entry to `## [X.Y.Z] - YYYY-MM-DD` using today's
date, and commit the changelog. With a clean working tree, run:

```console
dev-bin/release.sh
```

The helper checks that the branch includes `origin/main`, updates the Cargo
version and lockfiles, and validates the crate, wheel, sdist, tests, and metadata.
It shows the diff and release notes, then asks for confirmation to commit any
version changes, push the branch to `origin`, and create the GitHub release from
that commit. Declining or failing validation restores the generated version
changes. After publication, open a pull request to merge the release branch into
`main`.

To validate the current committed package without changing versions, committing,
pushing, or creating a release, run:

```console
dev-bin/release.sh --dry-run
```

Both registry projects are established. The `release.yml` workflow publishes
to crates.io and PyPI with short-lived OIDC credentials through the `release`
and `pypi` environments, respectively; releases do not use local upload tokens.

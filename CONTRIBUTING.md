# Contributing to maxminddb-polars

API proposals are welcome. Include tests for behavior changes and measurements
for performance changes.

## Prerequisites

- Git
- the Rust toolchain selected by `rust-toolchain.toml`
- a standard CPython version from the
  [compatibility matrix](.github/workflows/compatibility.yml)
- [uv](https://docs.astral.sh/uv/)
- [Precious](https://github.com/houseabsolute/precious)
- Node.js and npm to install the pinned Prettier version for Markdown and
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

Development and test builds disable debug information and incremental caches
to save disk space. `scripts/check` uses one Cargo job to limit memory use.
Set `CARGO_BUILD_JOBS` to use more jobs if you have enough memory.
To enable debug information and incremental builds:

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

The script prepares the environment, rebuilds the extension, and runs tests,
lint, formatting, type, documentation, and package checks. The first Polars
build needs substantial time and memory.

After setup, common focused checks are:

```console
uv run --no-sync pytest
cargo test --locked
precious lint --all
```

See [`scripts/check`](scripts/check) and [`precious.toml`](precious.toml) for
the full checks. The Python suite also tests the release helper on Unix using
temporary repositories and stub tools.

Run `precious tidy --all` to fix formatting. For benchmarks, use release builds
and compare against a named baseline on the same machine. See
[Performance and benchmarks](docs/performance.md) for commands and workloads.
Never commit licensed databases or reports containing database contents.

## Pull requests

- Keep commits focused and use imperative commit subjects.
- Add tests for behavior changes and regression fixes.
- Update `CHANGELOG.md` under `Unreleased` for user-visible changes.
- Update documentation and type hints with public API changes.
- Include before/after benchmark results for performance-sensitive code.
- Keep `Cargo.lock`, `uv.lock`, and generated state synchronized.

Update Rust `polars` and `pyo3-polars` together. Test the built wheels against
the supported Python Polars versions.

## Releases

Releases also require Bash, Perl, an authenticated GitHub CLI (`gh`), and push
access to `origin`.

Create a release branch from `origin/main` (any name other than `main`), move
the `Unreleased` changelog entry to `## [X.Y.Z] - YYYY-MM-DD` using today's
date, and commit the changelog. With a clean working tree, run:

```console
dev-bin/release.sh
```

The helper checks that the branch includes `origin/main`, stages version and
lockfile updates, and validates the packages and tests. It shows the diff and
release notes, then asks before committing, pushing, and creating a release.
It stops if files change during validation or confirmation, or if the commit
differs from the validated files.

A canceled or failed release keeps local changes, including version updates.
Review and commit or discard them before retrying. After publication, open a
pull request to merge the release branch into `main`.

To validate the current commit without changing versions, committing, pushing,
or publishing:

```console
dev-bin/release.sh --dry-run
```

The release workflow publishes to crates.io and PyPI with short-lived
credentials. See [Maintainer operations](docs/maintaining.md#publishing) for
publisher settings.

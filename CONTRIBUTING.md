# Contributing to maxminddb-polars

Thank you for helping improve `maxminddb-polars`. The project is in its initial
`0.1` release line, so API proposals are welcome, but performance and schema
changes should include tests and evidence.

## Prerequisites

- Git
- the Rust toolchain selected by `rust-toolchain.toml`
- Python 3.10 or newer
- [uv](https://docs.astral.sh/uv/)
- [Precious](https://github.com/houseabsolute/precious)
- Prettier for Markdown and YAML formatting

## Setup

```console
git submodule update --init --recursive
uv sync --all-extras --locked --no-install-project
uv run --no-sync maturin develop
```

The MaxMind-DB submodule contains test fixtures. Do not commit proprietary or
locally installed `.mmdb` databases.

To enable the repository's pre-commit hook:

```console
git config core.hooksPath .githooks
```

## Tests and checks

Run the fast pull-request suite with:

```console
scripts/check
```

The equivalent individual commands are:

```console
uv run pytest
cargo test --locked
cargo fmt --all -- --check
cargo fmt --manifest-path fuzz/Cargo.toml --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo clippy --manifest-path fuzz/Cargo.toml --all-targets --locked -- -D warnings
uv run ruff check .
uv run ruff format --check .
uv run mypy
precious lint --all
```

Run `precious tidy --all` to apply supported formatting fixes. Benchmarks must
use release builds and should compare the candidate against a named baseline on
the same machine. Never commit a full licensed database or benchmark output
containing database contents.

Benchmark reports include the source revision, tracked-worktree state, report
schema version, workload cardinality, and platform-normalized peak RSS. Use
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

Create `release/vX.Y.Z` from `origin/main`, move the `Unreleased` changelog
entry to `## [X.Y.Z] - YYYY-MM-DD`, and run:

```console
dev-bin/release.sh
```

The helper validates the Cargo crate, wheel, sdist, tests, and metadata before
creating and pushing the release-preparation commit. Merge its pull request,
update local `main`, and start the release from the verified merge commit with:

```console
dev-bin/release.sh --publish
```

Both registry projects are established. The `release.yml` workflow publishes
to crates.io and PyPI with short-lived OIDC credentials through the `release`
and `pypi` environments, respectively; normal releases do not use local upload
tokens. The helper retains a guarded token-based bootstrap path only for a new
registry project.

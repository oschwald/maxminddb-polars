# Installation and compatibility

Install a release from [PyPI](https://pypi.org/project/maxminddb-polars/):

```console
python -m pip install maxminddb-polars
```

This checkout supports Python 3.10 or later and Polars 1.43.2, 1.44, and 2.0.
See [`pyproject.toml`](../pyproject.toml) for dependency bounds and the
[compatibility matrix](../.github/workflows/compatibility.yml) for tested versions.
Free-threaded CPython builds are not built or tested.

Polars 2.0 uses the same native plugin as the supported 1.x versions, but
defaults lazy queries to streaming. Use `collect(engine="in-memory")` to keep
the previous engine. See the
[Polars 2.0 migration guide](https://docs.pola.rs/releases/upgrade/2/) for
changes to operations outside this library, including row order in joins and
grouping.

The [artifact workflow](../.github/workflows/artifacts.yml) builds wheels for:

- manylinux x86-64 and AArch64
- musllinux x86-64 and AArch64
- macOS x86-64 and Apple silicon
- Windows x86-64

Source distributions are also published. Other platforms require a compatible
Rust toolchain and native dependencies, and are not necessarily tested or supported.

To install a checkout from source instead:

```console
git clone --recurse-submodules https://github.com/oschwald/maxminddb-polars.git
cd maxminddb-polars
uv sync --all-extras --locked --no-install-project
uv run --no-sync maturin develop
```

See [Contributing](../CONTRIBUTING.md) for prerequisites and development checks.

## Compatibility policy

New Polars minors require source and wheel tests before we expand the dependency
range. Rust `polars`, `polars-arrow`, and `pyo3-polars` must stay compatible with
the Python plugin interface. Update the version matrix when changing that range.
Prerelease testing does not imply support. Dropping a Python minor must be noted
in the release notes.

The Compatibility workflow tests every supported Python minor against the minimum
Polars version and selected patches. It runs monthly, on dependency pull requests,
and on demand. Run it for release candidates not covered by those checks.

CI builds from source on Linux, macOS, and Windows, and builds every wheel target.
Runtime smoke tests cover whole records, partial schemas, and paths with `auto`,
`in-memory`, and `streaming`:

- manylinux x86-64, both macOS architectures, and Windows x86-64 test the same
  wheel against each pinned Polars version.
- The Linux wheel also runs on the oldest and newest supported Python versions.
- Linux AArch64 and musllinux wheels are built and inspected, without runtime tests.

Lockfiles, the Rust toolchain, and test fixtures are pinned. Dependency or fixture
updates require the full schema and artifact checks.
The crates.io package shares the Python package version but has no supported Rust API.

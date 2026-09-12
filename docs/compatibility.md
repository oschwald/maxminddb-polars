# Installation and compatibility

See [PyPI](https://pypi.org/project/maxminddb-polars/) for the latest published
package, Python requirement, and release files. The checkout's Python and
Polars requirements live in [`pyproject.toml`](../pyproject.toml), and the
exact tested combinations live in the
[compatibility matrix](../.github/workflows/compatibility.yml).

The Python Polars range is intentionally narrow because a native expression
plugin must match Polars' plugin ABI. A new Polars minor is supported only after
source and built-wheel tests pass and the declared dependency interval changes.

Support refers to standard GIL-enabled CPython builds. Free-threaded CPython
builds are not currently built or tested.

The [artifact workflow](../.github/workflows/artifacts.yml) builds abi3 wheels
for:

- manylinux x86-64 and AArch64;
- musllinux x86-64 and AArch64;
- macOS x86-64 and Apple silicon;
- Windows x86-64.

An sdist is also published for source builds. Building on a platform without an
advertised wheel requires a compatible Rust toolchain and native dependencies;
it does not imply that platform is tested or supported.

Install a released wheel with:

```console
python -m pip install maxminddb-polars
```

To install a checkout from source instead:

```console
git clone --recurse-submodules https://github.com/oschwald/maxminddb-polars.git
cd maxminddb-polars
uv sync --all-extras --locked --no-install-project
uv run --no-sync maturin develop
```

See [`CONTRIBUTING.md`](../CONTRIBUTING.md) for toolchain prerequisites, memory
considerations, and the complete development checks.

## Compatibility policy

- Dropping a supported Python minor requires a documented release change.
- The [Compatibility workflow](../.github/workflows/compatibility.yml) tests
  every supported Python minor against each pinned Polars version monthly, on
  dependency-related pull requests, and on manual dispatch. For release
  candidates, run it manually if the dependency checks did not cover the
  candidate. The release artifact workflow installs the exact Linux abi3 wheel
  on the oldest and newest Python versions in its matrix against each pinned
  Polars version.
- The pinned Polars matrix covers the supported minimum and selected patches
  in each supported minor. Rust `polars`, `polars-arrow`, and `pyo3-polars` move
  together; update the matrix when changing the declared Python interval.
- Linux, macOS, and Windows source builds run in CI. Every advertised wheel
  target is built by the reusable artifact workflow. It installs and
  smoke-tests manylinux x86-64, both macOS architectures, and Windows x86-64.
  The AArch64 Linux and musllinux wheels are built and inspected without runtime
  smoke tests.
- `Cargo.lock`, `uv.lock`, the release Rust toolchain, and the MaxMind-DB test
  fixture revision are committed. Dependency or fixture updates require the
  complete schema and artifact checks.
- The native implementation is also distributed on crates.io so releases have
  one version across both registries. Only the Python package has a supported
  public API; the Rust crate is the native implementation of the Python plugin.

Polars prereleases and future minors may be tested experimentally, but they are
not supported until the declared dependency interval changes.

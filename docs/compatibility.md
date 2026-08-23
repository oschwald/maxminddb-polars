# Installation and compatibility

The unreleased `0.1` line targets CPython 3.10 through 3.14 and Polars
`>=1.43,<1.44`. The Python Polars range is intentionally narrow because a
native expression plugin must match Polars' plugin ABI. A future Polars minor
will be added only after source and built-wheel tests pass.

Release artifacts are configured as `abi3-py310` wheels for:

- manylinux x86-64 and AArch64;
- musllinux x86-64 and AArch64;
- macOS x86-64 and Apple silicon;
- Windows x86-64;
- an sdist for other systems with a supported Rust toolchain.

Until the first release is published, install a checkout from source:

```console
git clone --recurse-submodules https://github.com/oschwald/maxminddb-polars.git
cd maxminddb-polars
uv sync --all-extras --locked
uv run maturin develop
```

Published wheels will install with `pip install maxminddb-polars`; this command
is not expected to work before an actual release exists.

## Compatibility policy

- Python 3.10 is the initial floor. Dropping a supported Python minor requires
  a documented release change.
- Every Python minor in the supported interval is tested monthly and before a
  release; the floor and ceiling also install the exact built abi3 wheel.
- The declared Python Polars interval is tested at its minimum and latest
  patch. Rust `polars`, `polars-arrow`, and `pyo3-polars` move together.
- Linux, macOS, and Windows source builds run in CI. Every advertised wheel
  target is built by the reusable artifact workflow; native architectures are
  installed and smoke-tested.
- `Cargo.lock`, `uv.lock`, the release Rust toolchain, and the MaxMind-DB test
  fixture revision are committed. Dependency or fixture updates require the
  complete schema and artifact checks.
- The `0.1` Rust crate remains `publish = false`; only the Python package has a
  supported public API.

Polars prereleases and future minors may be tested experimentally, but they are
not supported until the declared dependency interval changes.

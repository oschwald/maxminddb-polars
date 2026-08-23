# maxminddb-polars

Fast Polars expressions for MaxMind DB lookups, implemented in Rust.

> [!WARNING]
> This project is pre-release. Its public API and compatibility policy are
> still being implemented and may change before `0.1.0`.

The package will provide nested whole-record lookups for recognized MaxMind DB
schemas and selective path lookups for efficient enrichment. Callers supply
their own licensed or GeoLite database; no database is bundled or downloaded.

The repository currently contains the standalone package skeleton and an
internal native-expression smoke test. The user-facing lookup API described in
[the API decision record](docs/adr/0001-public-api.md) is not available yet.

## Development

Initialize the fixtures and run the checks with:

```console
git submodule update --init --recursive
uv sync --all-extras --locked
uv run maturin develop
uv run pytest
cargo test --locked
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the complete development workflow.

## Project policies

- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [ISC license](LICENSE)


# maxminddb-polars

Fast Polars expressions for MaxMind DB lookups, implemented in Rust.

> [!WARNING]
> This project is pre-release. Its public API and compatibility policy are
> still being implemented and may change before `0.1.0`.

The package will provide nested whole-record lookups for recognized MaxMind DB
schemas and selective path lookups for efficient enrichment. Callers supply
their own licensed or GeoLite database; no database is bundled or downloaded.

The scalar `lookup_path` API is available for selective enrichment. Known City
and ASN databases infer path dtypes; other MMDB schemas use an explicit dtype.
Whole-record and custom nested-schema support remain under development.

```python
from pathlib import Path

import polars as pl
import maxminddb_polars as mmp

database = Path("/data/GeoLite2-City.mmdb")
frame = pl.DataFrame({"ip": ["81.2.69.142", None]})

result = frame.select(
    mmp.lookup_path("ip", database, ("country", "iso_code")).alias("country")
)

# The expression namespace is equivalent:
country = pl.col("ip").mmdb.lookup_path(database, ("country", "iso_code"))
```

Inputs must have String dtype. Null inputs, lookup misses, and missing paths
produce null. Invalid IP strings raise by default; pass `strict=False` to turn
them into nulls. The package owns a strong byte snapshot for each database file
generation, so atomically replace database files and construct new expressions
to refresh them.

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

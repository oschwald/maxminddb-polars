# maxminddb-polars

Fast Polars expressions for MaxMind DB lookups, implemented in Rust.

> [!WARNING]
> This project is pre-release. Its public API and compatibility policy are
> still being implemented and may change before `0.1.0`.

The package provides nested whole-record lookups for recognized MaxMind DB
schemas and selective path lookups for efficient enrichment. Callers supply
their own licensed or GeoLite database; no database is bundled or downloaded.

```python
from pathlib import Path

import polars as pl
import maxminddb_polars as mmp

database = Path("/data/GeoLite2-City.mmdb")
frame = pl.DataFrame({"ip": ["81.2.69.142", None]})

result = frame.select(
    country=mmp.lookup_path("ip", database, ("country", "iso_code")),
    city=mmp.lookup("ip", database),
)

# The expression namespace is equivalent:
country = pl.col("ip").mmdb.lookup_path(database, ("country", "iso_code"))
```

Whole records infer one of the nine standard schemas from database metadata:
City, Country, Enterprise, ISP, Connection Type, Anonymous IP,
Density/Income, Domain, or ASN. Their stable Polars dtypes are exported from
`maxminddb_polars.schemas`.

Pass a nested mapping or `pl.Struct` as `dtype` for a partial known record or a
custom database. See [Custom and partial schemas](docs/custom-schemas.md).

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

Documentation:

- [API and supported databases](docs/api.md)
- [Custom and partial schemas](docs/custom-schemas.md)
- [Installation and compatibility](docs/compatibility.md)
- [Performance and benchmarks](docs/performance.md)
- [Competitor comparison](docs/comparison.md)
- [Security testing](docs/security-testing.md)

## Project policies

- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)
- [Security policy](SECURITY.md)
- [ISC license](LICENSE)

# maxminddb-polars

Fast Polars expressions for MaxMind DB lookups, implemented in Rust.

The Python API follows Semantic Versioning. The similarly named crates.io
package is the native implementation used to build the Python plugin; it does
not currently expose a supported Rust API.

The package provides nested whole-record lookups for recognized MaxMind DB
schemas and selective path lookups for efficient enrichment. Callers supply
their own licensed or GeoLite database; no database is bundled or downloaded.

Install it from PyPI with:

```console
python -m pip install maxminddb-polars
```

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

Whole records infer one of 14 standard schemas from database metadata: City,
Country, Enterprise, ISP, Connection Type, Anonymous IP, Anonymous Plus,
Residential Proxy, IP Risk, Static IP Score, User Count, Density/Income,
Domain, or ASN. Their stable Polars dtypes are exported from
`maxminddb_polars.schemas`.

Pass a nested mapping or `pl.Struct` as `dtype` for a partial known record or a
custom database. See
[Custom and partial schemas](https://github.com/oschwald/maxminddb-polars/blob/main/docs/custom-schemas.md).

Inputs must have String dtype. Null inputs, lookup misses, and paths absent
from a record produce null. Known databases validate paths against their schema
during planning. Invalid IP strings raise by default; pass `strict=False` to
turn them into nulls. Database errors, including decoder resource-limit errors,
still raise. The package caches generation-safe byte snapshots up to a
documented process limit, so atomically replace database files and construct new
expressions to refresh them.

## Development

With the prerequisites in
[CONTRIBUTING.md](https://github.com/oschwald/maxminddb-polars/blob/main/CONTRIBUTING.md#prerequisites)
installed, initialize the fixtures and run the development checks:

```console
git submodule update --init --recursive
scripts/check
```

See
[CONTRIBUTING.md](https://github.com/oschwald/maxminddb-polars/blob/main/CONTRIBUTING.md)
for the complete development workflow.

Documentation:

- [API and supported databases](https://github.com/oschwald/maxminddb-polars/blob/main/docs/api.md)
- [Custom and partial schemas](https://github.com/oschwald/maxminddb-polars/blob/main/docs/custom-schemas.md)
- [Installation and compatibility](https://github.com/oschwald/maxminddb-polars/blob/main/docs/compatibility.md)
- [Performance and benchmarks](https://github.com/oschwald/maxminddb-polars/blob/main/docs/performance.md)
- [Competitor comparison](https://github.com/oschwald/maxminddb-polars/blob/main/docs/comparison.md)
- [Security testing](https://github.com/oschwald/maxminddb-polars/blob/main/docs/security-testing.md)
- [Maintainer operations](https://github.com/oschwald/maxminddb-polars/blob/main/docs/maintaining.md)

## Project policies

- [Changelog](https://github.com/oschwald/maxminddb-polars/blob/main/CHANGELOG.md)
- [Contributing](https://github.com/oschwald/maxminddb-polars/blob/main/CONTRIBUTING.md)
- [Security policy](https://github.com/oschwald/maxminddb-polars/blob/main/SECURITY.md)
- [ISC license](https://github.com/oschwald/maxminddb-polars/blob/main/LICENSE)

# maxminddb-polars

Look up IP addresses in MaxMind databases with Polars expressions, implemented
in Rust. Read a single field with `lookup_path` or a record with `lookup`.

You supply the database. The package does not bundle or download one.

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

`lookup` detects the schema for
[14 standard database families](https://github.com/oschwald/maxminddb-polars/blob/main/docs/api.md#supported-database-metadata).
Pass `dtype` to read only selected fields or use a custom database. See
[Custom and partial schemas](https://github.com/oschwald/maxminddb-polars/blob/main/docs/custom-schemas.md).

The IP column must have String dtype. Null inputs, lookup misses, and missing
paths return null. Invalid IP strings raise an error by default. Pass
`strict=False` to turn them into nulls. Database and schema errors still raise.

To update a database, replace the file atomically and create new expressions.
See [Database updates](https://github.com/oschwald/maxminddb-polars/blob/main/docs/api.md#database-updates)
for cache behavior and memory limits.

## Development

Install the prerequisites in
[CONTRIBUTING.md](https://github.com/oschwald/maxminddb-polars/blob/main/CONTRIBUTING.md#prerequisites)
and run:

```console
git submodule update --init --recursive
scripts/check
```

Documentation:

- [API and supported databases](https://github.com/oschwald/maxminddb-polars/blob/main/docs/api.md)
- [Custom and partial schemas](https://github.com/oschwald/maxminddb-polars/blob/main/docs/custom-schemas.md)
- [Installation and compatibility](https://github.com/oschwald/maxminddb-polars/blob/main/docs/compatibility.md)
- [Performance and benchmarks](https://github.com/oschwald/maxminddb-polars/blob/main/docs/performance.md)
- [Competitor comparison](https://github.com/oschwald/maxminddb-polars/blob/main/docs/comparison.md)
- [Security testing](https://github.com/oschwald/maxminddb-polars/blob/main/docs/security-testing.md)
- [Maintainer operations](https://github.com/oschwald/maxminddb-polars/blob/main/docs/maintaining.md)

## Project policies

The Python API follows Semantic Versioning. The crates.io package is the native
implementation of the Python plugin and has no supported Rust API.

- [Changelog](https://github.com/oschwald/maxminddb-polars/blob/main/CHANGELOG.md)
- [Contributing](https://github.com/oschwald/maxminddb-polars/blob/main/CONTRIBUTING.md)
- [Security policy](https://github.com/oschwald/maxminddb-polars/blob/main/SECURITY.md)
- [ISC license](https://github.com/oschwald/maxminddb-polars/blob/main/LICENSE)

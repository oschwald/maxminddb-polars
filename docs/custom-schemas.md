# Custom and partial schemas

Pass `dtype` to `lookup` to read selected fields or describe a custom database.
Use a `pl.Struct` or a nested mapping of Polars dtypes. For example, to read a
country code and coordinates:

```python
import polars as pl
import maxminddb_polars as mmp

frame = pl.DataFrame({"ip": ["89.160.20.128", None]})
projection = {
    "country": {"iso_code": pl.String},
    "location": {
        "latitude": pl.Float64,
        "longitude": pl.Float64,
    },
}

result = frame.with_columns(
    geo=mmp.lookup("ip", "/data/GeoLite2-City.mmdb", dtype=projection)
)
```

For a standard database, the library checks fields and dtypes during schema
planning. For a custom database, it checks them while decoding records.
Mappings preserve field order and give the same results as an equivalent
`pl.Struct`.

Custom unsigned integer dtypes accept any encoded unsigned width when the value
fits the requested dtype exactly. This applies to scalar paths, List elements,
and whole-record projections. Out-of-range values raise `ComputeError`, including
with `strict=False`; they are never truncated. This does not enable conversions
from signed integers, floats, strings, or booleans.

See the [API reference](api.md#dtypes-and-validity) for supported dtypes and
missing-value behavior.

Use `lookup_path` for one field. Use `lookup` with a partial dtype for several
fields so they share one database search per valid IP. Selecting fields after
a full `lookup` still decodes the whole record.

Decoder resource limits also apply to custom and partial records. Exceeding
them raises a Polars `ComputeError`, even with `strict=False`.

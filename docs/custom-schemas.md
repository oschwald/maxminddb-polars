# Custom and partial schemas

`lookup` infers the complete dtype for the 14 standard MaxMind database
families. Pass `dtype` to select a smaller known record or to describe an
otherwise unknown MMDB database. The dtype must be a `pl.Struct` or a nested
mapping whose leaves are supported Polars dtypes.

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

For a known database, every requested field and leaf dtype is validated during
Polars schema planning. Unknown databases are validated while values decode.
Mappings preserve insertion order. An equivalent `pl.Struct` produces the same
output dtype and values.

Supported leaves are Boolean, signed and unsigned integers through 128 bits,
Float32, Float64, String, and Binary. `pl.List` and nested `pl.Struct` can occur
at any depth. Missing scalar fields are null, missing declared Lists are empty,
and missing declared Structs apply these defaults recursively to their fields.
A lookup miss, null input, or invalid IP with `strict=False` makes the outer
record null.

For one field, prefer `lookup_path`. Its explicit dtype must exactly match the
inferred dtype for a known path; partial known Structs use `lookup` instead.
A partial Struct is the fused API for several related fields: it performs one
search-tree lookup per valid non-null IP, decodes selected leaves once per
unique record offset, and assembles their arrays into the declared nested
Struct. Decoder resource limits apply to custom and partial records too;
exceeding them raises a Polars `ComputeError` even with `strict=False`.

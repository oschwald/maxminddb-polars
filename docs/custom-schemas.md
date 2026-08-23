# Custom and partial schemas

`lookup` infers the complete dtype for the nine standard MaxMind database
families. Pass `dtype` to select a smaller known record or to describe an
otherwise unknown MMDB database. The dtype must be a `pl.Struct` or a nested
mapping whose leaves are supported Polars dtypes.

```python
import polars as pl
import maxminddb_polars as mmp

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
at any depth. Missing scalar fields are null; missing declared Struct fields
are present with null descendants, and missing declared Lists are empty. A
lookup miss or null input makes the outer record null.

For one field, prefer `lookup_path`. A partial Struct is the fused API for
several related fields: it performs one search-tree lookup per IP, decodes
selected leaves once per unique record offset, and assembles their arrays into
the declared nested Struct.

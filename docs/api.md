# API reference

Importing `maxminddb_polars` registers the `.mmdb` expression namespace and
exposes four public symbols: `__version__`, `lookup`, `lookup_path`, and
`schemas`.

## Whole records

```python
def lookup(
    expr,
    database,
    *,
    dtype=None,
    strict=True,
) -> pl.Expr: ...
```

`expr` must resolve to a Polars String column. Set `database` to the path of
your MMDB file. For standard databases, `lookup` infers the full Struct dtype
from metadata. Pass `dtype` to read selected fields. Custom databases require
`dtype`. See [Custom and partial schemas](custom-schemas.md).

The equivalent namespace method is
`pl.col("ip").mmdb.lookup(database, dtype=dtype, strict=strict)`.

## Paths

```python
def lookup_path(
    expr,
    database,
    path,
    *,
    dtype=None,
    strict=True,
) -> pl.Expr: ...
```

Pass `path` as a non-empty list or tuple, such as `("country", "iso_code")`.
Use strings for map keys and integers for List indexes. Negative indexes count
from the end. Dot-separated strings are not supported because MMDB keys can
contain dots.

For standard databases, `lookup_path` infers the result dtype and rejects
invalid paths during planning, even with `strict=False`. If you pass `dtype`,
it must match exactly, including every field of a nested Struct. To select part
of a Struct, use `lookup(..., dtype=...)`. Custom databases require `dtype`.

The equivalent namespace method is
`pl.col("ip").mmdb.lookup_path(database, path, dtype=dtype, strict=strict)`.

## Supported database metadata

| Output schema     | Recognized `database_type` values                                                                                    |
| ----------------- | -------------------------------------------------------------------------------------------------------------------- |
| City              | `GeoIP2-City`, `GeoLite2-City`, `GeoIP2-City-Shield`                                                                 |
| Country           | `GeoIP2-Country`, `GeoLite2-Country`, `GeoIP2-Country-Shield`                                                        |
| Enterprise        | `GeoIP2-Enterprise`, `GeoIP2-Enterprise-Shield`, `GeoIP2-Precision-Enterprise`, `GeoIP2-Precision-Enterprise-Shield` |
| ISP               | `GeoIP2-ISP`                                                                                                         |
| Connection Type   | `GeoIP2-Connection-Type`                                                                                             |
| Anonymous IP      | `GeoIP2-Anonymous-IP`                                                                                                |
| Anonymous Plus    | `GeoIP-Anonymous-Plus`                                                                                               |
| Residential Proxy | `GeoIP-Residential-Proxy`                                                                                            |
| IP Risk           | `GeoIP2-IP-Risk`                                                                                                     |
| Static IP Score   | `GeoIP2-Static-IP-Score`                                                                                             |
| User Count        | `GeoIP2-User-Count`                                                                                                  |
| Density/Income    | `GeoIP2-DensityIncome`                                                                                               |
| Domain            | `GeoIP2-Domain`                                                                                                      |
| ASN               | `GeoIP2-ASN`, `GeoLite2-ASN`                                                                                         |

Stable dtypes are available as uppercase constants in `maxminddb_polars.schemas`.
Any `database_type` not listed above is treated as a custom database.

Output field names are serialized MMDB keys: for example `names.en`,
`names.pt-BR`, and `represented_country.type`.

## Dtypes and validity

Supported leaf dtypes are Boolean, signed and unsigned integers through 128
bits, Float32, Float64, String, and Binary. Lists and Structs may be nested.
Unsupported logical types fail during expression construction.

| Condition                                            | Result                             |
| ---------------------------------------------------- | ---------------------------------- |
| Null input                                           | null output                        |
| Valid IP with no record                              | null output                        |
| Path absent from a record or List index out of range | null output                        |
| Missing scalar field                                 | null field                         |
| Missing declared nested Struct                       | present Struct with field defaults |
| Missing declared List                                | empty List                         |
| Invalid IP, `strict=True`                            | Polars compute error               |
| Invalid IP, `strict=False`                           | null output                        |
| Unknown database without dtype                       | schema-planning error              |
| Known field/dtype mismatch                           | schema-planning error              |
| Path not valid for a known schema                    | schema-planning error              |
| Corrupt data or custom dtype mismatch                | Polars compute error               |
| Decoder or metadata resource limit exceeded          | Polars compute error               |

`strict=False` only converts invalid IP strings to nulls. Database, decode,
resource-limit, and schema errors still raise. Decoder limits also apply to
structurally valid records and metadata. See [Security testing](security-testing.md).

For `lookup`, a null outer Struct indicates a lookup miss, null input, or an
invalid IP with `strict=False`. A Struct-valued `lookup_path` also returns null
when the selected path is absent from a record.

Missing Structs apply the table's defaults to their fields recursively.
The output does not distinguish a missing map from one containing those defaults.

## Database updates

Replace the MMDB file atomically and create a new expression to use the update.
Schema planning and execution share a cached snapshot of the database bytes.
If an in-place change is detected while opening the file, the lookup fails.

The process-wide cache retains up to 512 MiB by default, evicting the oldest
snapshots first. Set `MAXMINDDB_POLARS_CACHE_MAX_BYTES` before the first lookup
to change the limit. It accepts a non-negative byte count. The cache always
retains the newest snapshot, even if that snapshot exceeds the limit.

An existing expression uses its old snapshot while it remains cached. After
eviction, it reopens the unchanged file or raises an error asking you to recreate
the expression. Running evaluations keep their snapshots until they finish.
Those snapshots and output arrays can use memory beyond the cache limit.

Snapshots are identified by the canonical path, size, nanosecond modification
time, metadata-change or creation time, and file identity. Unix detects
same-size rewrites even when modification time is restored. Windows detects
atomic replacements, but might miss such an in-place rewrite. Use atomic
replacement on both platforms.

You are responsible for obtaining, updating, and licensing your database.

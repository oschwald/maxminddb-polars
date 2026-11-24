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

`expr` must resolve to a Polars String column. Other dtypes raise
`MMDB lookup input must have String dtype`. Cast a Categorical or Enum column
with `.cast(pl.String)` first. Set `database` to the path of your MMDB file.
For standard databases, `lookup` infers the full Struct dtype from metadata.
Pass `dtype` to read selected fields. Custom databases require `dtype`. See
[Custom and partial schemas](custom-schemas.md).

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
from the end. A bare string or bytes-like value raises `TypeError`, so write a
one-key path as `("uint32",)`, not `"uint32"`. A set, frozenset or mapping
also raises `TypeError` because it has no fixed component order. Dot-separated
strings are not supported because MMDB keys can contain dots.

For standard databases, `lookup_path` infers the result dtype and rejects
paths that its schema does not list or that do not fit the schema during
planning, even with `strict=False`. If you pass `dtype` for a field the
standard schema lists, it must match exactly, including every field of a nested
Struct. To select part of a Struct, use `lookup(..., dtype=...)`. If the path
names a field the standard schema does not list, such as a field added by a
newer database, pass `dtype` to read it as you would from a custom database.
Custom databases require `dtype`.

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
| Missing declared List field in a Struct              | empty List                         |
| Invalid IP, `strict=True`                            | Polars compute error               |
| Invalid IP, `strict=False`                           | null output                        |
| IPv6 input on IPv4-only database, `strict=True`      | Polars compute error               |
| IPv6 input on IPv4-only database, `strict=False`     | null output                        |
| Unknown database without dtype                       | schema-planning error              |
| Known field/dtype mismatch                           | schema-planning error              |
| Path that does not fit a field a known schema lists  | schema-planning error              |
| Field a known schema does not list, without dtype    | schema-planning error              |
| Field a known schema does not list, with dtype       | decoded as for a custom database   |
| Corrupt data or custom dtype mismatch                | Polars compute error               |
| Decoder or metadata resource limit exceeded          | Polars compute error               |

`strict=False` only converts invalid IP strings, and IPv6 addresses looked up
in an IPv4-only database (`ip_version` 4 in the metadata), to nulls. This
includes IPv4-mapped IPv6 addresses such as `::ffff:1.1.1.1`. Database, decode,
resource-limit, and schema errors still raise. Decoder limits also apply to
structurally valid records and metadata. See [Security testing](security-testing.md).

For `lookup`, a null outer Struct indicates a lookup miss, null input, or, with
`strict=False`, an invalid IP or an IPv6 address in an IPv4-only database. A
Struct- or List-valued `lookup_path` also returns null when the selected path
is absent from a record, so an absent List is distinct from a stored empty
array.

Missing Structs apply the table's defaults to their fields recursively.
The output does not distinguish a missing map from one containing those defaults.

## Database updates

Replace the MMDB file atomically and create a new expression to use the update.
Schema planning and execution share a cached snapshot of the database bytes.
If an in-place change is detected while opening the file, the lookup fails.

The process-wide cache retains up to 512 MiB by default, evicting the least
recently used snapshots first. Set `MAXMINDDB_POLARS_CACHE_MAX_BYTES` before the
first lookup to change the limit. It accepts a non-negative byte count. The cache
always retains the snapshot it most recently opened or reused, even if that
snapshot exceeds the limit.

An existing expression uses its old snapshot while it remains cached. Running
evaluations keep their snapshots until they finish, and lookups reuse an evicted
snapshot while any evaluation still holds it. Once no evaluation holds it, the
expression reopens the unchanged file or raises an error asking you to recreate
the expression. Those snapshots and output arrays can use memory beyond the cache
limit.

Set the limit high enough to hold every database that one query uses. Streaming
queries look up each batch separately. When the databases in a query do not fit,
batches that do not overlap can evict each other's snapshots and read the files
again.

Snapshots are identified by the canonical path, size, nanosecond modification
time, metadata-change or creation time, and file identity. Unix detects
same-size rewrites even when modification time is restored. Windows detects
atomic replacements, but might miss such an in-place rewrite. Use atomic
replacement on both platforms.

You are responsible for obtaining, updating, and licensing your database.

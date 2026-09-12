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

`expr` must resolve to a Polars String column. `database` is a filesystem path
to a caller-supplied MMDB file. Standard databases infer their complete Struct
dtype from metadata. An unknown database requires a `pl.Struct` or nested
mapping in `dtype`; the same argument selects a validated partial Struct from a
known database.

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

Pass `path` as a non-empty list or tuple of string map keys and integer List
indexes, for example `("country", "iso_code")`. Strings are not interpreted as
dot-separated paths because MMDB map keys can contain dots. Negative indexes
count from the end. Known schemas infer
scalar, Struct, or List results and reject unknown fields or incompatible path
components during planning, even with `strict=False`.

For a known schema, an explicit dtype must exactly match the inferred path
dtype, including all fields of a nested Struct. Use `lookup(..., dtype=...)`
to select a partial known Struct. An unknown schema requires an explicit dtype.

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

Exact stable dtypes are available as uppercase values in
`maxminddb_polars.schemas`, including `RESIDENTIAL_PROXY`, `ANONYMOUS_PLUS`,
`IP_RISK`, `STATIC_IP_SCORE`, and `USER_COUNT`. Other metadata names are
intentionally treated as custom databases, even if their names resemble a
standard product.

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
resource-limit, and schema errors still raise. The `maxminddb` decoder bounds
container values and aggregate string/byte payloads; a structurally valid
record or metadata section can exceed those bounds. See
[`security-testing.md`](security-testing.md) for the regression coverage.

For `lookup`, a null outer Struct indicates a lookup miss, null input, or an
invalid IP with `strict=False`. A Struct-valued `lookup_path` also returns null
when the selected path is absent from a record.

Missing declared Structs apply field defaults recursively: scalar fields are
null, Struct fields are present, and List fields are empty. These defaults do
not prove whether the source map was physically absent.

## Database updates

An expression captures the canonical path, byte size, nanosecond modification
time, filesystem metadata-change or creation time, and the file identity on
Unix and Windows. Schema planning and execution share a strong in-memory byte
snapshot for that generation.
Atomically replace an MMDB file and construct a new expression to use the
replacement. In-place changes detected during open fail rather than silently
mixing generations.

Unix change time and file identity distinguish same-size replacements and
in-place rewrites whose modification time is restored. The Windows volume and
file identity distinguish atomic replacements, but a same-size in-place rewrite
with a restored modification time might not be detected. Prefer atomic
replacement on all platforms.

The process-wide snapshot cache retains up to 512 MiB in insertion order by
default. Set `MAXMINDDB_POLARS_CACHE_MAX_BYTES` before the first lookup to choose
a different non-negative byte limit. The newest snapshot is retained even when
it alone exceeds the limit. An already planned expression uses its old snapshot
while it remains cached; after eviction it either reopens unchanged bytes or
returns an error asking the caller to reconstruct the expression.

Eviction removes the cache's reference to a snapshot; an evaluation already
using it retains its own reference until it completes. The byte limit applies
to cached database snapshots, with the newest-snapshot exception above. Active
evaluations and output arrays can use additional memory.

The package never downloads a database. Users are responsible for obtaining,
updating, and licensing their MMDB files.

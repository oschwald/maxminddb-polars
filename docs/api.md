# API reference

Importing `maxminddb_polars` registers the `.mmdb` expression namespace and
exposes four public symbols: `__version__`, `lookup`, `lookup_path`, and
`schemas`.

## Whole records

```python
lookup(
    expr,
    database,
    *,
    dtype=None,
    strict=True,
) -> pl.Expr
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
lookup_path(
    expr,
    database,
    path,
    *,
    dtype=None,
    strict=True,
) -> pl.Expr
```

`path` is a non-empty sequence of string map keys and integer List indexes.
Negative indexes count from the end. Known schemas infer scalar, Struct, or
List results. An unknown schema requires an explicit dtype.

The equivalent namespace method is
`pl.col("ip").mmdb.lookup_path(database, path, dtype=dtype, strict=strict)`.

## Supported database metadata

| Output schema   | Recognized `database_type` values                                                                                    |
| --------------- | -------------------------------------------------------------------------------------------------------------------- |
| City            | `GeoIP2-City`, `GeoLite2-City`, `GeoIP2-City-Shield`                                                                 |
| Country         | `GeoIP2-Country`, `GeoLite2-Country`, `GeoIP2-Country-Shield`                                                        |
| Enterprise      | `GeoIP2-Enterprise`, `GeoIP2-Enterprise-Shield`, `GeoIP2-Precision-Enterprise`, `GeoIP2-Precision-Enterprise-Shield` |
| ISP             | `GeoIP2-ISP`                                                                                                         |
| Connection Type | `GeoIP2-Connection-Type`                                                                                             |
| Anonymous IP    | `GeoIP2-Anonymous-IP`                                                                                                |
| Density/Income  | `GeoIP2-DensityIncome`                                                                                               |
| Domain          | `GeoIP2-Domain`                                                                                                      |
| ASN             | `GeoIP2-ASN`, `GeoLite2-ASN`                                                                                         |

Exact stable dtypes are available as uppercase values in
`maxminddb_polars.schemas`. Other metadata names are intentionally treated as
custom databases, even if their names resemble a standard product.

Output field names are serialized MMDB keys: for example `names.en`,
`names.pt-BR`, and `represented_country.type`.

## Dtypes and validity

Supported leaf dtypes are Boolean, signed and unsigned integers through 128
bits, Float32, Float64, String, and Binary. Lists and Structs may be nested.
Unsupported logical types fail during expression construction.

| Condition                             | Result                          |
| ------------------------------------- | ------------------------------- |
| Null input                            | null output                     |
| Valid IP with no record               | null output                     |
| Missing path                          | null output                     |
| Missing scalar field                  | null field                      |
| Missing declared nested Struct        | present Struct with null leaves |
| Missing declared List                 | empty List                      |
| Invalid IP, `strict=True`             | Polars compute error            |
| Invalid IP, `strict=False`            | null output                     |
| Unknown database without dtype        | schema-planning error           |
| Known field/dtype mismatch            | schema-planning error           |
| Corrupt data or custom dtype mismatch | Polars compute error            |

A null outer Struct means the IP had no record. An all-null nested Struct does
not prove whether its source map was physically absent.

## Database updates

An expression captures the canonical path, byte size, nanosecond modification
time, and filesystem metadata-change or creation time. Schema planning and
execution share a strong in-memory byte snapshot for that generation.
Atomically replace an MMDB file and construct a new expression to use the
replacement. In-place changes detected during open fail rather than silently
mixing generations.

The process-wide snapshot cache retains up to 512 MiB in insertion order by
default. Set `MAXMINDDB_POLARS_CACHE_MAX_BYTES` before the first lookup to choose
a different non-negative byte limit. The newest snapshot is retained even when
it alone exceeds the limit. An already planned expression uses its old snapshot
while it remains cached; after eviction it either reopens unchanged bytes or
returns an error asking the caller to reconstruct the expression.

The package never downloads a database. Users are responsible for obtaining,
updating, and licensing their MMDB files.

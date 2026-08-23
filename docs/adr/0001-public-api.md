# ADR 0001: Initial public API

- Status: Accepted for implementation
- Date: 2026-08-23

## Context

Polars users need both complete MaxMind DB records and efficient selective
lookups. Current Polars releases do not push downstream Struct-field selections
into FFI expression plugins, so whole-record lookup alone would force callers
to decode and materialize fields they do not use.

## Decision

The initial Python package is `maxminddb_polars`, and importing it registers the
`.mmdb` expression namespace. It exposes these equivalent standalone and
namespace operations:

```python
def lookup(
    expr: IntoExpr,
    database: str | Path,
    *,
    dtype: DTypeLike | None = None,
    strict: bool = True,
) -> pl.Expr: ...

def lookup_path(
    expr: IntoExpr,
    database: str | Path,
    path: Sequence[str | int],
    *,
    dtype: DTypeLike | None = None,
    strict: bool = True,
) -> pl.Expr: ...
```

For recognized database types, `lookup` infers a complete nested Struct dtype
and `lookup_path` infers its selected dtype. Other databases require an explicit
dtype. A Struct dtype supplied to `lookup` is the exact requested output shape;
for a recognized database it may be a validated partial Struct and therefore a
fused multi-field projection.

Paths are non-empty sequences of string map keys and integer list indexes.
Negative indexes count from the end. Dotted strings are not accepted as a path
shorthand because MMDB map keys may contain dots.

Null inputs and lookup misses produce null output. Invalid IP addresses raise a
Polars compute error when `strict=True` and produce nulls when `strict=False`.
The initial release accepts String input only.

The public package will export `lookup`, `lookup_path`, `schemas`, and the
`.mmdb` namespace methods. It will not expose reader-cache controls, native
plugin symbols, a JSON API, or a separate multi-path operation in `0.1`.

## Consequences

`lookup_path` remains a supported performance API until native Polars plugin
output projection provides near-parity in every supported execution mode. The
Rust engine will keep one internal projection representation so path lookup,
partial Struct lookup, and a future optimizer integration share implementation.


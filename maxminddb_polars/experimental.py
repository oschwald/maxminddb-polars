"""Experimental projection-aware lookups.

This module requires Polars 2.0 or later. It uses the unstable Polars
``LazyFrameResolver`` API, which can change in any Polars release.
"""

from __future__ import annotations

from collections.abc import Iterator
from pathlib import Path
from typing import TYPE_CHECKING, Any

import polars as pl

from maxminddb_polars._api import lookup

try:
    from polars.lazyframe.resolver import LazyFrameResolver
except ImportError as error:
    raise ImportError(
        "maxminddb_polars.experimental requires Polars 2.0 or later; "
        f"found Polars {pl.__version__}"
    ) from error

if TYPE_CHECKING:
    from polars._typing import IntoExpr, PolarsDataType
    from polars.lazyframe.resolver import FilterExpr

    from maxminddb_polars._api import DTypeLike

__all__ = ["lookup_frame"]

_RECORD = "__maxminddb_polars_record"


def lookup_frame(
    frame: pl.LazyFrame | pl.DataFrame,
    ip: IntoExpr,
    database: str | Path,
    *,
    dtype: DTypeLike | None = None,
    strict: bool = True,
) -> pl.LazyFrame:
    """Return ``frame`` with one column for each leaf field of the MMDB record.

    Nested Struct fields become columns with dotted names, for example
    ``country.iso_code``. A List field is one column. A query decodes only the
    fields that it uses, with one lookup for each IP address.

    A query that uses no MMDB column does not run the lookup, so it does not
    raise for invalid IP addresses. ``dtype`` and ``strict`` have the same
    meaning as in ``lookup``.
    """
    frame = frame.lazy()
    record = (
        pl.LazyFrame(schema={"ip": pl.String})
        .select(lookup("ip", database, dtype=dtype, strict=strict))
        .collect_schema()["ip"]
    )
    leaves: dict[str, tuple[tuple[str, ...], PolarsDataType]] = {}
    for path, leaf in _leaves(record, ()):
        name = ".".join(path)
        if name in leaves:
            raise ValueError(f"MMDB fields produce the column name {name!r} twice")
        leaves[name] = (path, leaf)
    schema: dict[str, PolarsDataType] = dict(frame.collect_schema())
    duplicates = sorted((leaves.keys() | {_RECORD}) & schema.keys())
    if duplicates:
        raise ValueError(
            f"frame columns conflict with MMDB columns: {duplicates}; "
            "rename the frame columns"
        )
    schema.update({name: leaf for name, (_, leaf) in leaves.items()})
    return _LookupFrame(frame, ip, database, strict, leaves, schema).lazy()


class _LookupFrame(LazyFrameResolver):
    def __init__(
        self,
        frame: pl.LazyFrame,
        ip: IntoExpr,
        database: str | Path,
        strict: bool,
        leaves: dict[str, tuple[tuple[str, ...], PolarsDataType]],
        schema: dict[str, PolarsDataType],
    ) -> None:
        self._frame = frame
        self._ip = ip
        self._database = database
        self._strict = strict
        self._leaves = leaves
        self._schema = schema

    def schema(self) -> dict[str, PolarsDataType]:
        return self._schema

    def resolve_lazyframe(
        self,
        *,
        projection: list[str] | None,
        limit: int | None,
        filters: list[FilterExpr],
        filter_columns: list[str],
        filter_drop_columns_idx: int | None,
        existing_resolved_version_key: str | None,
    ) -> pl.LazyFrame:
        # Polars applies the filters and the limit to the returned plan and
        # pushes them below the elementwise lookup. ``projection`` already
        # contains the columns that the filters use.
        frame = self._frame
        if projection is None:
            projection = list(self._schema)
        if not projection:
            return frame.drop("*")  # keeps the height for a row count
        fields = {name for name in projection if name in self._leaves}
        if not fields:
            return frame.select(projection)
        # shortcut: lookup() reads the database identity at each new
        # projection, not once in lookup_frame(). Pass a stored identity if
        # one LazyFrame must keep one database snapshot.
        record = lookup(
            self._ip, self._database, dtype=self._pruned(fields), strict=self._strict
        )
        return frame.with_columns(record.alias(_RECORD)).select(
            self._field(name) if name in fields else pl.col(name) for name in projection
        )

    def _pruned(self, fields: set[str]) -> dict[str, Any]:
        dtype: dict[str, Any] = {}
        # Iterate the record schema so that field order matches the database.
        for name, (path, leaf) in self._leaves.items():
            if name in fields:
                node = dtype
                for part in path[:-1]:
                    node = node.setdefault(part, {})
                node[path[-1]] = leaf
        return dtype

    def _field(self, name: str) -> pl.Expr:
        expr = pl.col(_RECORD)
        for part in self._leaves[name][0]:
            expr = expr.struct.field(part)
        return expr.alias(name)


def _leaves(
    dtype: PolarsDataType, path: tuple[str, ...]
) -> Iterator[tuple[tuple[str, ...], PolarsDataType]]:
    if isinstance(dtype, pl.Struct) and dtype.fields:
        for field in dtype.fields:
            yield from _leaves(field.dtype, (*path, field.name))
    else:
        yield path, dtype

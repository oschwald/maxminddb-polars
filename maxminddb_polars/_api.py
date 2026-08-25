"""Public expression construction helpers."""

from __future__ import annotations

import os
from collections.abc import Mapping, Sequence
from pathlib import Path
from typing import TYPE_CHECKING

import polars as pl
from polars.plugins import register_plugin_function

from maxminddb_polars._schema import normalize_dtype

if TYPE_CHECKING:
    from polars._typing import IntoExpr, PolarsDataType

    DTypeLike = PolarsDataType | Mapping[str, "DTypeLike"]


_PLUGIN_PATH = Path(__file__).parent


def _database_identity(database: str | Path) -> dict[str, str | int]:
    path = Path(database).expanduser().resolve(strict=True)
    stat = path.stat()
    if not path.is_file():
        raise ValueError(f"MMDB path is not a file: {path}")
    return {
        "canonical_path": str(path),
        "size": stat.st_size,
        "modified_ns": stat.st_mtime_ns,
        "changed_ns": stat.st_ctime_ns,
        # Rust's stable Windows metadata API does not expose the file index.
        "file_id": 0 if os.name == "nt" else stat.st_ino,
    }


def _normalize_path(path: Sequence[str | int]) -> list[str | int]:
    normalized = list(path)
    if not normalized:
        raise ValueError("MMDB lookup path must not be empty")
    for part in normalized:
        if isinstance(part, bool) or not isinstance(part, (str, int)):
            raise TypeError(
                f"MMDB path components must be strings or integers; got {part!r}"
            )
        if isinstance(part, int) and not -(2**63) <= part < 2**63:
            raise ValueError(
                f"MMDB path index is outside the signed 64-bit range: {part}"
            )
    return normalized


def lookup_path(
    expr: IntoExpr,
    database: str | Path,
    path: Sequence[str | int],
    *,
    dtype: DTypeLike | None = None,
    strict: bool = True,
) -> pl.Expr:
    """Look up one value at ``path`` for each IP address in ``expr``.

    Null IPs, lookup misses, and missing paths produce null. Invalid IP strings
    raise a Polars ``ComputeError`` unless ``strict=False``, in which case they
    also produce null.
    """
    if not isinstance(strict, bool):
        raise TypeError(f"strict must be a bool, got {strict!r}")
    return register_plugin_function(
        plugin_path=_PLUGIN_PATH,
        function_name="mmdb_lookup_path",
        args=[expr],
        kwargs={
            "database": _database_identity(database),
            "path": _normalize_path(path),
            "dtype": None if dtype is None else normalize_dtype(dtype),
            "strict": strict,
        },
        # Keep the logical Series together so the native plugin can coalesce
        # tiny physical chunks into bounded parallel tasks.
        is_elementwise=False,
    )


def lookup(
    expr: IntoExpr,
    database: str | Path,
    *,
    dtype: DTypeLike | None = None,
    strict: bool = True,
) -> pl.Expr:
    """Look up one whole MMDB record for each IP address in ``expr``.

    Standard MaxMind database schemas are inferred from metadata. Unknown
    databases require an explicit Struct dtype or field-to-dtype mapping. A
    partial Struct selects only those fields from a known database.
    """
    if not isinstance(strict, bool):
        raise TypeError(f"strict must be a bool, got {strict!r}")
    return register_plugin_function(
        plugin_path=_PLUGIN_PATH,
        function_name="mmdb_lookup",
        args=[expr],
        kwargs={
            "database": _database_identity(database),
            "dtype": None if dtype is None else normalize_dtype(dtype),
            "strict": strict,
        },
        is_elementwise=True,
    )


@pl.api.register_expr_namespace("mmdb")
class MaxMindDbNameSpace:
    """MaxMind DB operations for a Polars expression."""

    def __init__(self, expr: pl.Expr) -> None:
        self._expr = expr

    def lookup_path(
        self,
        database: str | Path,
        path: Sequence[str | int],
        *,
        dtype: DTypeLike | None = None,
        strict: bool = True,
    ) -> pl.Expr:
        """Return the value at ``path`` for every IP in this expression."""
        return lookup_path(self._expr, database, path, dtype=dtype, strict=strict)

    def lookup(
        self,
        database: str | Path,
        *,
        dtype: DTypeLike | None = None,
        strict: bool = True,
    ) -> pl.Expr:
        """Return a whole or projected record for every IP."""
        return lookup(self._expr, database, dtype=dtype, strict=strict)

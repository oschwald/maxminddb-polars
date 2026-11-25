"""Public expression construction helpers."""

from __future__ import annotations

from collections.abc import ItemsView, KeysView, Mapping, Sequence
from collections.abc import Set as AbstractSet
from pathlib import Path
from typing import TYPE_CHECKING

import polars as pl
from polars.plugins import register_plugin_function

from maxminddb_polars._maxminddb_polars import database_identity
from maxminddb_polars._schema import normalize_dtype

if TYPE_CHECKING:
    from polars._typing import IntoExpr, PolarsDataType

    DTypeLike = PolarsDataType | Mapping[str, "DTypeLike"]


_PLUGIN_PATH = Path(__file__).parent


def _database_identity(database: str | Path) -> dict[str, str | int]:
    path = Path(database).expanduser().resolve(strict=True)
    if not path.is_file():
        raise ValueError(f"MMDB path is not a file: {path}")
    (
        canonical_path,
        size,
        modified_ns,
        changed_ns,
        volume_id,
        file_id,
        file_id_high,
    ) = database_identity(str(path))
    return {
        "canonical_path": canonical_path,
        "size": size,
        "modified_ns": modified_ns,
        "changed_ns": changed_ns,
        "volume_id": volume_id,
        "file_id": file_id,
        "file_id_high": file_id_high,
    }


def _normalize_path(path: Sequence[str | int]) -> list[str | int]:
    # A str is a Sequence[str], so a bare key would otherwise be split into
    # one-character components. Bytes-like values iterate as integers and
    # would silently become List indexes.
    if isinstance(path, str):
        raise TypeError(
            "MMDB lookup path must be a sequence of path components, not a "
            f"string; pass ({path!r},) to look up a single key"
        )
    if isinstance(path, (bytes, bytearray, memoryview)):
        raise TypeError(
            "MMDB lookup path must be a sequence of path components, not "
            f"{type(path).__name__}"
        )
    # Sets have no stable iteration order, so their components could be read
    # in a different order from one run to the next. Dict key and item views
    # are also Sets but keep insertion order, so they are left alone.
    if isinstance(path, AbstractSet) and not isinstance(path, (KeysView, ItemsView)):
        raise TypeError(
            "MMDB lookup path must be an ordered sequence such as a list or "
            f"tuple, not {type(path).__name__}"
        )
    # Mappings iterate over their keys only, which is unlikely to be what the
    # caller meant.
    if isinstance(path, Mapping):
        raise TypeError(
            "MMDB lookup path must be a sequence of path components, not a "
            f"mapping ({type(path).__name__}); pass a list or tuple of keys"
        )
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

    Null IPs, lookup misses, and paths absent from a record produce null. Known
    schemas validate paths during planning; an explicit dtype must exactly
    match the inferred dtype of a field the schema lists. A field the known
    schema does not list, and any field of an unknown schema, requires an
    explicit dtype.

    Invalid IP strings raise a Polars ``ComputeError`` unless ``strict=False``,
    in which case they also produce null. Database and decoder resource-limit
    errors still raise regardless of strictness.
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
        # Each output row depends only on its input row, so Polars may push
        # filters and slices below the lookup and split it into batches.
        is_elementwise=True,
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
    partial Struct selects only those fields from a known database. Fields the
    known schema lists must match its dtypes; other fields are decoded with the
    requested dtype, as for an unknown database.

    Null IPs and lookup misses produce null records. Invalid IP strings raise
    a Polars ``ComputeError`` unless ``strict=False``. Database and decoder
    resource-limit errors still raise regardless of strictness.
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

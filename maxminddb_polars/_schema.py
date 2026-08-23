"""Polars dtype normalization for the native plugin boundary."""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any

import polars as pl

_SCALAR_TYPES: dict[pl.DataType | type[pl.DataType], str] = {
    pl.Boolean: "boolean",
    pl.UInt8: "uint8",
    pl.UInt16: "uint16",
    pl.UInt32: "uint32",
    pl.UInt64: "uint64",
    pl.UInt128: "uint128",
    pl.Int8: "int8",
    pl.Int16: "int16",
    pl.Int32: "int32",
    pl.Int64: "int64",
    pl.Int128: "int128",
    pl.Float32: "float32",
    pl.Float64: "float64",
    pl.String: "string",
    pl.Binary: "binary",
}


def normalize_dtype(dtype: Any) -> dict[str, Any]:
    """Convert a supported Polars dtype or mapping to a Serde-friendly spec."""
    if isinstance(dtype, Mapping):
        return {
            "type": "struct",
            "fields": [
                {"name": name, "dtype": normalize_dtype(field_dtype)}
                for name, field_dtype in dtype.items()
            ],
        }

    dtype = pl.datatypes.parse_into_dtype(dtype)
    scalar_type = _SCALAR_TYPES.get(dtype)
    if scalar_type is not None:
        return {"type": scalar_type}
    if isinstance(dtype, pl.List):
        return {"type": "list", "inner": normalize_dtype(dtype.inner)}
    if isinstance(dtype, pl.Struct):
        return {
            "type": "struct",
            "fields": [
                {"name": field.name, "dtype": normalize_dtype(field.dtype)}
                for field in dtype.fields
            ],
        }
    raise TypeError(
        "MMDB output does not support the Polars dtype "
        f"{dtype!r}; use a Boolean, integer, float, String, Binary, List, or Struct"
    )

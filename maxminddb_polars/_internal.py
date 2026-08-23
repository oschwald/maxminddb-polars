"""Private packaging checks for the native expression plugin."""

from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING

from polars.plugins import register_plugin_function

if TYPE_CHECKING:
    import polars as pl
    from polars._typing import IntoExpr


_PLUGIN_PATH = Path(__file__).parent


def identity(expr: IntoExpr) -> pl.Expr:
    """Return a private native identity expression used by smoke tests."""
    return register_plugin_function(
        plugin_path=_PLUGIN_PATH,
        function_name="identity",
        args=expr,
        is_elementwise=True,
    )

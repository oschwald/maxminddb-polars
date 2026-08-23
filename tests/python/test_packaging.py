import importlib
import importlib.metadata

import polars as pl

import maxminddb_polars as mmp
from maxminddb_polars._internal import identity


def test_public_surface_is_frozen_for_zero_one() -> None:
    assert mmp.__all__ == ["__version__", "lookup", "lookup_path", "schemas"]


def test_native_module_is_importable() -> None:
    native = importlib.import_module("maxminddb_polars._maxminddb_polars")

    assert native.__version__ == importlib.metadata.version("maxminddb-polars")


def test_native_expression_runs_eagerly() -> None:
    frame = pl.DataFrame({"value": ["one", None, "two"]})

    result = frame.select(identity("value"))

    assert result.to_dict(as_series=False) == {"value": ["one", None, "two"]}


def test_native_expression_runs_lazily() -> None:
    frame = pl.DataFrame({"value": ["one", None, "two"]}).lazy()

    result = frame.select(identity("value"))

    assert result.collect().to_dict(as_series=False) == {"value": ["one", None, "two"]}
    assert result.collect(engine="streaming").to_dict(as_series=False) == {
        "value": ["one", None, "two"]
    }

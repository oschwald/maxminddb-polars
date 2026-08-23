import polars as pl

from maxminddb_polars._internal import identity


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

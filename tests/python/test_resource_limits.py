from __future__ import annotations

from pathlib import Path

import polars as pl
import pytest

import maxminddb_polars as mmp

DATA = Path(__file__).parents[1] / "data"
DATABASES = DATA / "test-data"


@pytest.mark.parametrize("strict", [True, False])
def test_nested_pointer_expansion_raises_compute_error(strict: bool) -> None:
    # The fixture has 40 levels of arrays sharing their two child pointers.
    # Selecting the first child leaves 39 levels and an enormous expansion.
    dtype: pl.DataType | type[pl.DataType] = pl.UInt16
    for _ in range(39):
        dtype = pl.List(dtype)
    expression = mmp.lookup_path(
        "ip",
        DATABASES / "MaxMind-DB-test-pointer-decoder-dos.mmdb",
        [0],
        dtype=dtype,
        strict=strict,
    )

    with pytest.raises(
        pl.exceptions.ComputeError,
        match="resource limit exceeded.*maximum number of data structure values",
    ):
        pl.DataFrame({"ip": ["1.2.3.4"]}).select(expression)


@pytest.mark.parametrize("strict", [True, False])
@pytest.mark.parametrize("rows", [1, 8192])
@pytest.mark.parametrize("projection", [False, True])
def test_path_navigation_and_selected_value_share_a_resource_budget(
    strict: bool, rows: int, projection: bool
) -> None:
    database = DATABASES / "MaxMind-DB-test-decode-path-shared-budget.mmdb"
    frame = pl.DataFrame({"ip": ["1.2.3.4"] * rows})
    expression = (
        mmp.lookup("ip", database, dtype={"target": pl.String}, strict=strict)
        if projection
        else mmp.lookup_path("ip", database, ["target"], dtype=pl.String, strict=strict)
    )

    with pytest.raises(pl.exceptions.ComputeError, match="resource limit exceeded"):
        frame.select(expression)


@pytest.mark.parametrize("projection", [False, True])
def test_metadata_resource_limits_raise_compute_error(projection: bool) -> None:
    database = DATABASES / "MaxMind-DB-test-metadata-payload-limit.mmdb"

    with pytest.raises(pl.exceptions.ComputeError, match="resource limit exceeded"):
        expression = (
            mmp.lookup("ip", database, dtype={"ip": pl.String})
            if projection
            else mmp.lookup_path("ip", database, ["ip"], dtype=pl.String)
        )
        pl.DataFrame({"ip": ["1.2.3.4"]}).select(expression)


@pytest.mark.parametrize("container", ["array", "map"])
def test_empty_containers_at_end_of_metadata_are_valid(container: str) -> None:
    database = (
        DATA
        / "bad-data/libmaxminddb"
        / f"libmaxminddb-empty-{container}-last-in-metadata.mmdb"
    )
    frame = pl.DataFrame({"ip": ["1.1.1.1", None]})

    result = frame.select(
        scalar=mmp.lookup_path("ip", database, ["ip"], dtype=pl.String),
        record=mmp.lookup("ip", database, dtype={"ip": pl.String}),
    )

    assert result.to_dict(as_series=False) == {
        "scalar": ["test", None],
        "record": [{"ip": "test"}, None],
    }

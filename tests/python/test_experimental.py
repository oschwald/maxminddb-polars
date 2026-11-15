from __future__ import annotations

from pathlib import Path
from typing import TYPE_CHECKING, Any, Literal

import polars as pl
import pytest

import maxminddb_polars as mmp

if TYPE_CHECKING:
    from polars._typing import PolarsDataType

pytest.importorskip(
    "polars.lazyframe.resolver", reason="lookup_frame requires Polars 2.0 or later"
)

from maxminddb_polars import experimental  # noqa: E402
from maxminddb_polars.experimental import lookup_frame  # noqa: E402

DATA = Path(__file__).parents[1] / "data" / "test-data"
CITY_DB = DATA / "GeoIP2-City-Test.mmdb"
CUSTOM_DB = DATA / "MaxMind-DB-test-decoder.mmdb"

Engine = Literal["auto", "in-memory", "streaming"]
ENGINES: list[Engine] = ["auto", "in-memory", "streaming"]

IPS = ["89.160.20.128", None, "203.0.113.1", "2.125.160.216"]


def _input() -> pl.LazyFrame:
    return pl.LazyFrame({"id": [1, 2, 3, 4], "ip": IPS})


@pytest.fixture
def dtypes(monkeypatch: pytest.MonkeyPatch) -> list[Any]:
    """Record the dtype of each lookup that a resolved plan contains."""
    seen: list[Any] = []

    def spy(*args: Any, **kwargs: Any) -> pl.Expr:
        seen.append(kwargs["dtype"])
        return mmp.lookup(*args, **kwargs)

    monkeypatch.setattr(experimental, "lookup", spy)
    return seen


def _leaf_names(dtype: PolarsDataType, prefix: str = "") -> list[str]:
    if not isinstance(dtype, pl.Struct):
        return [prefix]
    return [
        name
        for field in dtype.fields
        for name in _leaf_names(
            field.dtype, f"{prefix}.{field.name}" if prefix else field.name
        )
    ]


def test_schema_has_input_columns_then_leaves_in_record_order() -> None:
    schema = lookup_frame(_input(), "ip", CITY_DB).collect_schema()

    assert list(schema) == ["id", "ip", *_leaf_names(mmp.schemas.CITY)]
    assert schema["country.iso_code"] == pl.String
    assert schema["location.latitude"] == pl.Float64
    assert schema["city.names.en"] == pl.String
    assert isinstance(schema["subdivisions"], pl.List)


@pytest.mark.parametrize("engine", ENGINES)
def test_unprojected_query_matches_a_full_lookup(engine: Engine) -> None:
    result = lookup_frame(_input(), "ip", CITY_DB).collect(engine=engine)
    record = _input().select(record=mmp.lookup("ip", CITY_DB)).collect()["record"]

    assert result.columns == ["id", "ip", *_leaf_names(mmp.schemas.CITY)]
    assert result["id"].to_list() == [1, 2, 3, 4]
    for name in _leaf_names(mmp.schemas.CITY):
        expected = record
        for part in name.split("."):
            expected = expected.struct.field(part)
        assert result[name].equals(expected.alias(name)), name


@pytest.mark.parametrize("engine", ENGINES)
def test_selected_fields_use_one_pruned_lookup(
    engine: Engine, dtypes: list[Any]
) -> None:
    query = lookup_frame(_input(), "ip", CITY_DB).select(
        "id",
        "location.latitude",
        country="country.iso_code",
        city=pl.col("city.names.en").str.to_uppercase(),
    )
    dtypes.clear()
    result = query.collect(engine=engine)

    # The pruned dtype uses the field order of the database, not of the query.
    assert dtypes == [
        {
            "city": {"names": {"en": pl.String}},
            "country": {"iso_code": pl.String},
            "location": {"latitude": pl.Float64},
        }
    ]
    assert query.explain().count("mmdb_lookup") == 1
    assert result.to_dict(as_series=False) == {
        "id": [1, 2, 3, 4],
        "location.latitude": [58.4167, None, None, 51.75],
        "country": ["SE", None, None, "GB"],
        "city": ["LINKÖPING", None, None, "BOXFORD"],
    }
    assert result.equals(
        _input()
        .select(
            "id",
            mmp.lookup_path("ip", CITY_DB, ["location", "latitude"]).alias(
                "location.latitude"
            ),
            country=mmp.lookup_path("ip", CITY_DB, ["country", "iso_code"]),
            city=mmp.lookup_path(
                "ip", CITY_DB, ["city", "names", "en"]
            ).str.to_uppercase(),
        )
        .collect()
    )


def test_list_field_is_one_column(dtypes: list[Any]) -> None:
    query = lookup_frame(_input(), "ip", CITY_DB).select("subdivisions")
    dtypes.clear()
    result = query.collect()

    assert list(dtypes[0]) == ["subdivisions"]
    assert result.equals(
        _input()
        .select(subdivisions=mmp.lookup_path("ip", CITY_DB, ["subdivisions"]))
        .collect()
    )


@pytest.mark.parametrize("engine", ENGINES)
def test_filter_field_is_decoded_but_not_returned(
    engine: Engine, dtypes: list[Any]
) -> None:
    query = (
        lookup_frame(_input(), "ip", CITY_DB)
        .filter(pl.col("country.iso_code") == "GB", pl.col("id") > 1)
        .select("id", "city.names.en")
    )
    dtypes.clear()
    result = query.collect(engine=engine)

    assert dtypes == [
        {"city": {"names": {"en": pl.String}}, "country": {"iso_code": pl.String}}
    ]
    assert result.to_dict(as_series=False) == {"id": [4], "city.names.en": ["Boxford"]}


def test_ip_expression(dtypes: list[Any]) -> None:
    frame = pl.LazyFrame({"raw": [" 89.160.20.128 ", "\t2.125.160.216"]})
    query = lookup_frame(frame, pl.col("raw").str.strip_chars(), CITY_DB)
    dtypes.clear()

    assert query.select("raw", "country.iso_code").collect().to_dict(
        as_series=False
    ) == {
        "raw": [" 89.160.20.128 ", "\t2.125.160.216"],
        "country.iso_code": ["SE", "GB"],
    }
    # The IP expression needs ``raw`` although the query does not select it.
    assert query.select("country.iso_code").collect().to_series().to_list() == [
        "SE",
        "GB",
    ]


def test_null_unmatched_and_invalid_ips() -> None:
    frame = pl.LazyFrame({"ip": ["not-an-ip", None, "203.0.113.1", "89.160.20.128"]})

    with pytest.raises(pl.exceptions.ComputeError, match="invalid IP address"):
        lookup_frame(frame, "ip", CITY_DB).select("country.iso_code").collect()

    lenient = lookup_frame(frame, "ip", CITY_DB, strict=False)
    assert lenient.select("ip", "country.iso_code").collect().to_dict(
        as_series=False
    ) == {
        "ip": ["not-an-ip", None, "203.0.113.1", "89.160.20.128"],
        "country.iso_code": [None, None, None, "SE"],
    }

    with pytest.raises(TypeError, match="strict must be a bool"):
        lookup_frame(frame, "ip", CITY_DB, strict=1)  # type: ignore[arg-type]


def test_query_without_mmdb_columns_skips_the_lookup(dtypes: list[Any]) -> None:
    frame = pl.LazyFrame({"id": [1, 2], "ip": ["not-an-ip", "89.160.20.128"]})
    query = lookup_frame(frame, "ip", CITY_DB).select("id")
    dtypes.clear()

    assert query.collect().to_dict(as_series=False) == {"id": [1, 2]}
    assert dtypes == []
    assert "mmdb_lookup" not in query.explain()


@pytest.mark.parametrize("engine", ENGINES)
def test_chunked_input(engine: Engine) -> None:
    frame = pl.concat(
        [pl.DataFrame({"ip": IPS}, schema={"ip": pl.String})] * 5
        + [pl.DataFrame({"ip": []}, schema={"ip": pl.String})],
        rechunk=False,
    )
    assert frame.n_chunks() > 1

    result = (
        lookup_frame(frame, "ip", CITY_DB)
        .select("country.iso_code", "location.latitude")
        .collect(engine=engine)
    )
    assert result.to_dict(as_series=False) == {
        "country.iso_code": ["SE", None, None, "GB"] * 5,
        "location.latitude": [58.4167, None, None, 51.75] * 5,
    }


def test_empty_input_keeps_the_schema() -> None:
    frame = pl.LazyFrame(schema={"ip": pl.String})
    result = lookup_frame(frame, "ip", CITY_DB).select("country.iso_code").collect()

    assert result.schema == {"country.iso_code": pl.String}
    assert result.height == 0


@pytest.mark.parametrize("engine", ENGINES)
def test_limits(engine: Engine) -> None:
    query = lookup_frame(_input(), "ip", CITY_DB).select("id", "country.iso_code")

    assert query.head(1).collect(engine=engine).rows() == [(1, "SE")]
    assert query.slice(2, 2).collect(engine=engine).rows() == [(3, None), (4, "GB")]
    assert query.tail(1).collect(engine=engine).rows() == [(4, "GB")]
    assert query.filter(pl.col("country.iso_code").is_not_null()).head(2).collect(
        engine=engine
    ).rows() == [(1, "SE"), (4, "GB")]
    assert query.head(3).filter(pl.col("country.iso_code").is_not_null()).collect(
        engine=engine
    ).rows() == [(1, "SE")]


def test_limit_runs_before_the_lookup() -> None:
    # A strict lookup of the second row raises, so success proves that the
    # lookup only received the first row.
    frame = pl.LazyFrame({"ip": ["89.160.20.128", "not-an-ip"]})
    query = lookup_frame(frame, "ip", CITY_DB).select("country.iso_code")

    assert query.head(1).collect(engine="in-memory").rows() == [("SE",)]


def test_row_count_and_empty_projection() -> None:
    query = lookup_frame(_input(), "ip", CITY_DB)
    assert query.select(pl.len()).collect().item() == 4

    # Polars 2.0 and 2.1 never pass an empty projection. Later versions
    # document it as a request for the row count only.
    resolver = experimental._LookupFrame(_input(), "ip", CITY_DB, True, {}, {})
    resolved = resolver.resolve_lazyframe(
        projection=[],
        limit=None,
        filters=[],
        filter_columns=[],
        filter_drop_columns_idx=None,
        existing_resolved_version_key=None,
    ).collect()
    assert resolved.shape == (4, 0)


def test_repeated_resolution(dtypes: list[Any]) -> None:
    frame = lookup_frame(_input(), "ip", CITY_DB)
    dtypes.clear()
    country = frame.select("country.iso_code")
    latitude = frame.select("location.latitude")

    first = country.collect()
    assert country.collect().equals(first)
    assert latitude.collect().to_series().to_list() == [58.4167, None, None, 51.75]
    assert country.collect().equals(first)
    # Polars caches one resolved plan for each projection.
    assert dtypes == [
        {"country": {"iso_code": pl.String}},
        {"location": {"latitude": pl.Float64}},
    ]


def test_self_join_resolves_each_side_with_its_own_fields(dtypes: list[Any]) -> None:
    frame = lookup_frame(_input(), "ip", CITY_DB)
    dtypes.clear()
    result = (
        frame.select("id", "country.iso_code")
        .join(frame.select("id", "location.latitude"), on="id")
        .sort("id")
        .collect()
    )

    assert sorted(dtypes, key=str) == [
        {"country": {"iso_code": pl.String}},
        {"location": {"latitude": pl.Float64}},
    ]
    assert result.rows() == [
        (1, "SE", 58.4167),
        (2, None, None),
        (3, None, None),
        (4, "GB", 51.75),
    ]


def test_group_by_prunes_fields(dtypes: list[Any]) -> None:
    query = (
        lookup_frame(_input(), "ip", CITY_DB)
        .group_by("country.iso_code")
        .agg(pl.len())
        .sort("country.iso_code", nulls_last=True)
    )
    dtypes.clear()

    assert query.collect().rows() == [("GB", 1), ("SE", 1), (None, 2)]
    assert dtypes == [{"country": {"iso_code": pl.String}}]


def test_unpruned_query_decodes_every_field(dtypes: list[Any]) -> None:
    # ``with_columns`` keeps every column, so Polars passes no projection.
    query = lookup_frame(_input(), "ip", CITY_DB).with_columns(
        upper=pl.col("country.iso_code").str.to_lowercase()
    )
    dtypes.clear()
    result = query.collect()

    assert len(dtypes) == 1
    assert pl.Struct(_as_struct(dtypes[0])) == mmp.schemas.CITY
    assert result["upper"].to_list() == ["se", None, None, "gb"]
    assert result.width == 2 + len(_leaf_names(mmp.schemas.CITY)) + 1


def _as_struct(dtype: dict[str, Any]) -> dict[str, Any]:
    return {
        name: pl.Struct(_as_struct(value)) if isinstance(value, dict) else value
        for name, value in dtype.items()
    }


def test_custom_database_requires_and_prunes_dtype(dtypes: list[Any]) -> None:
    frame = pl.LazyFrame({"ip": ["::1.1.1.0", "::9.9.9.9"]})

    with pytest.raises(
        pl.exceptions.ComputeError, match="pass a Struct dtype explicitly"
    ):
        lookup_frame(frame, "ip", CUSTOM_DB)

    query = lookup_frame(
        frame,
        "ip",
        CUSTOM_DB,
        dtype={"uint32": pl.UInt32, "map": {"mapX": {"utf8_stringX": pl.String}}},
    )
    assert list(query.collect_schema()) == ["ip", "uint32", "map.mapX.utf8_stringX"]
    dtypes.clear()

    assert query.select("map.mapX.utf8_stringX").collect().to_series().to_list() == [
        "hello",
        None,
    ]
    assert dtypes == [{"map": {"mapX": {"utf8_stringX": pl.String}}}]


def test_rejects_conflicting_column_names() -> None:
    frame = pl.LazyFrame({"ip": ["89.160.20.128"], "country.iso_code": ["XX"]})
    with pytest.raises(ValueError, match=r"conflict.*country\.iso_code"):
        lookup_frame(frame, "ip", CITY_DB)

    with pytest.raises(ValueError, match="twice"):
        lookup_frame(
            pl.LazyFrame({"ip": ["::1.1.1.0"]}),
            "ip",
            CUSTOM_DB,
            dtype={"a": {"b": pl.String}, "a.b": pl.String},
        )


def test_accepts_a_dataframe() -> None:
    result = lookup_frame(_input().collect(), "ip", CITY_DB).select("country.iso_code")
    assert result.collect().to_series().to_list() == ["SE", None, None, "GB"]

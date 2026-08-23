from __future__ import annotations

from pathlib import Path
from typing import Any, Literal

import polars as pl
import pytest

import maxminddb_polars as mmp

DATABASES = Path(__file__).parents[1] / "data" / "test-data"
CUSTOM_DB = DATABASES / "MaxMind-DB-test-decoder.mmdb"
CITY_DB = DATABASES / "GeoIP2-City-Test.mmdb"

CUSTOM_MAPPING: dict[str, Any] = {
    "uint16": pl.UInt16,
    "array": pl.List(pl.UInt32),
    "map": {
        "mapX": {
            "utf8_stringX": pl.String,
            "arrayX": pl.List(pl.UInt32),
        }
    },
}
CUSTOM_STRUCT = pl.Struct(
    {
        "uint16": pl.UInt16,
        "array": pl.List(pl.UInt32),
        "map": pl.Struct(
            {
                "mapX": pl.Struct(
                    {
                        "utf8_stringX": pl.String,
                        "arrayX": pl.List(pl.UInt32),
                    }
                )
            }
        ),
    }
)


@pytest.mark.parametrize("engine", ["auto", "streaming"])
def test_arbitrary_nested_custom_record(
    engine: Literal["auto", "streaming"],
) -> None:
    frame = pl.DataFrame({"ip": ["::1.1.1.0", None, "2001:db8::1"]}).lazy()

    result = frame.select(
        mmp.lookup("ip", CUSTOM_DB, dtype=CUSTOM_MAPPING).alias("record")
    ).collect(engine=engine)

    assert result.schema == {"record": CUSTOM_STRUCT}
    assert result.to_dicts() == [
        {
            "record": {
                "uint16": 100,
                "array": [1, 2, 3],
                "map": {
                    "mapX": {
                        "utf8_stringX": "hello",
                        "arrayX": [7, 8, 9],
                    }
                },
            }
        },
        {"record": None},
        {"record": None},
    ]


def test_mapping_and_polars_struct_are_identical() -> None:
    frame = pl.DataFrame({"ip": ["::1.1.1.0"]})
    result = frame.select(
        mapping=mmp.lookup("ip", CUSTOM_DB, dtype=CUSTOM_MAPPING),
        struct=mmp.lookup("ip", CUSTOM_DB, dtype=CUSTOM_STRUCT),
    )

    assert result.schema == {"mapping": CUSTOM_STRUCT, "struct": CUSTOM_STRUCT}
    assert result["mapping"].equals(result["struct"])


def test_every_supported_scalar_dtype_crosses_the_plugin_boundary() -> None:
    dtype = {
        "bytes": pl.Binary,
        "boolean": pl.Boolean,
        "double": pl.Float64,
        "float": pl.Float32,
        "int32": pl.Int32,
        "uint16": pl.UInt16,
        "uint32": pl.UInt32,
        "uint64": pl.UInt64,
        "uint128": pl.UInt128,
        "utf8_string": pl.String,
    }
    result = pl.DataFrame({"ip": ["::1.1.1.0"]}).select(
        mmp.lookup("ip", CUSTOM_DB, dtype=dtype).alias("record")
    )

    assert result.schema == {"record": pl.Struct(dtype)}
    record = result.item()
    assert record["bytes"] == b"\x00\x00\x00*"
    assert record["boolean"] is True
    assert record["double"] == 42.123456
    assert record["float"] == pytest.approx(1.1)
    assert record["int32"] == -(2**28)
    assert record["uint16"] == 100
    assert record["uint32"] == 2**28
    assert record["uint64"] == 2**60
    assert record["uint128"] == 2**120
    assert record["utf8_string"] == "unicode! ☯ - ♫"


def test_partial_known_schema_selects_nested_fields() -> None:
    projection: dict[str, Any] = {
        "country": {"iso_code": pl.String},
        "location": {"latitude": pl.Float64, "longitude": pl.Float64},
        "subdivisions": pl.List(pl.Struct({"iso_code": pl.String})),
    }
    result = pl.DataFrame({"ip": ["89.160.20.128"]}).select(
        mmp.lookup("ip", CITY_DB, dtype=projection).alias("record")
    )

    expected = pl.Struct(
        {
            "country": pl.Struct({"iso_code": pl.String}),
            "location": pl.Struct({"latitude": pl.Float64, "longitude": pl.Float64}),
            "subdivisions": pl.List(pl.Struct({"iso_code": pl.String})),
        }
    )
    assert result.schema == {"record": expected}
    assert result.item() == {
        "country": {"iso_code": "SE"},
        "location": {"latitude": 58.4167, "longitude": 15.6167},
        "subdivisions": [{"iso_code": "E"}],
    }


def test_struct_and_list_path_outputs_use_the_same_nested_schema_builder() -> None:
    frame = pl.DataFrame({"ip": ["89.160.20.128", None]})
    result = frame.select(
        country=mmp.lookup_path("ip", CITY_DB, ["country"]),
        subdivisions=mmp.lookup_path("ip", CITY_DB, ["subdivisions"]),
    )

    city_fields = {field.name: field.dtype for field in mmp.schemas.CITY.fields}
    assert result.schema == {
        "country": city_fields["country"],
        "subdivisions": city_fields["subdivisions"],
    }
    assert result["country"].struct.field("iso_code").to_list() == ["SE", None]
    assert result["subdivisions"].list.len().to_list() == [1, None]


def test_custom_nested_path_accepts_a_mapping_dtype() -> None:
    dtype = {"mapX": {"arrayX": pl.List(pl.UInt32)}}
    result = pl.DataFrame({"ip": ["::1.1.1.0"]}).select(
        mmp.lookup_path("ip", CUSTOM_DB, ["map"], dtype=dtype).alias("value")
    )

    expected = pl.Struct({"mapX": pl.Struct({"arrayX": pl.List(pl.UInt32)})})
    assert result.schema == {"value": expected}
    assert result.item() == {"mapX": {"arrayX": [7, 8, 9]}}


def test_partial_known_schema_errors_during_planning() -> None:
    frame = pl.DataFrame({"ip": ["89.160.20.128"]}).lazy()

    unknown = frame.select(mmp.lookup("ip", CITY_DB, dtype={"unknown": pl.String}))
    with pytest.raises(pl.exceptions.ComputeError, match="has no field"):
        unknown.collect_schema()

    wrong = frame.select(
        mmp.lookup("ip", CITY_DB, dtype={"country": {"iso_code": pl.UInt32}})
    )
    with pytest.raises(pl.exceptions.ComputeError, match="does not match"):
        wrong.collect_schema()


def test_custom_schema_type_mismatch_is_deterministic() -> None:
    expression = mmp.lookup("ip", CUSTOM_DB, dtype={"uint16": pl.String})
    frame = pl.DataFrame({"ip": ["::1.1.1.0"]})

    with pytest.raises(
        pl.exceptions.ComputeError, match="could not decode MMDB record"
    ):
        frame.select(expression)


def test_unknown_whole_record_requires_a_struct_dtype() -> None:
    frame = pl.DataFrame({"ip": ["::1.1.1.0"]}).lazy()
    missing = frame.select(mmp.lookup("ip", CUSTOM_DB))
    with pytest.raises(pl.exceptions.ComputeError, match="pass a Struct dtype"):
        missing.collect_schema()

    scalar = frame.select(mmp.lookup("ip", CUSTOM_DB, dtype=pl.UInt32))
    with pytest.raises(pl.exceptions.ComputeError, match="requires a Struct dtype"):
        scalar.collect_schema()

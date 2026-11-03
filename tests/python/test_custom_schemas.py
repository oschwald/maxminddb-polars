from __future__ import annotations

from pathlib import Path
from typing import Any, Literal

import polars as pl
import pytest

import maxminddb_polars as mmp

DATABASES = Path(__file__).parents[1] / "data" / "test-data"
CUSTOM_DB = DATABASES / "MaxMind-DB-test-decoder.mmdb"
CITY_DB = DATABASES / "GeoIP2-City-Test.mmdb"


def test_range_safe_uint32_scalar_to_uint16() -> None:
    result = pl.DataFrame({"ip": ["::1.1.1.0"]}).select(
        mmp.lookup_path("ip", CUSTOM_DB, ["array", 0], dtype=pl.UInt16).alias("value")
    )
    assert result.schema == {"value": pl.UInt16}
    assert result["value"].to_list() == [1]


def test_range_safe_unsigned_projection() -> None:
    dtype: dict[str, Any] = {"uint16": pl.UInt32, "array": pl.List(pl.UInt16)}
    result = pl.DataFrame({"ip": ["::1.1.1.0"]}).select(
        mmp.lookup("ip", CUSTOM_DB, dtype=dtype).alias("record")
    )
    assert result.schema == {"record": pl.Struct(dtype)}
    assert result.item() == {"uint16": 100, "array": [1, 2, 3]}


@pytest.mark.parametrize("strict", [True, False])
@pytest.mark.parametrize(
    "dtype,bits",
    [
        (pl.UInt8, 8),
        (pl.UInt16, 16),
        (pl.UInt32, 32),
        (pl.UInt64, 64),
        (pl.UInt128, 128),
    ],
)
@pytest.mark.parametrize(
    "field,source_bits",
    [("uint16", 16), ("uint32", 32), ("uint64", 64), ("uint128", 128)],
)
def test_unsigned_width_boundaries(
    strict: bool, dtype: Any, bits: int, field: str, source_bits: int
) -> None:
    zero = pl.DataFrame({"ip": ["::0.0.0.0"]})
    maximum = pl.DataFrame({"ip": ["::255.255.255.255"]})
    expressions = [
        mmp.lookup_path("ip", CUSTOM_DB, [field], dtype=dtype, strict=strict).alias(
            "value"
        ),
        mmp.lookup("ip", CUSTOM_DB, dtype={field: dtype}, strict=strict)
        .struct.field(field)
        .alias("value"),
    ]
    for expression in expressions:
        result = zero.select(expression)
        assert result.schema == {"value": dtype}
        assert result["value"].to_list() == [0]
        if source_bits <= bits:
            result = maximum.select(expression)
            assert result.schema == {"value": dtype}
            assert result["value"].to_list() == [2**source_bits - 1]
        else:
            with pytest.raises(
                pl.exceptions.ComputeError, match="could not decode MMDB"
            ):
                maximum.select(expression)


def test_unsigned_narrowing_boundary_is_inclusive(tmp_path: Path) -> None:
    database = tmp_path / "narrow.mmdb"
    # Both values are encoded as uint16.
    write_mmdb(database, "Custom-Narrow", {1: {"n": 255}, 2: {"n": 256}})
    expressions = [
        mmp.lookup_path("ip", database, ["n"], dtype=pl.UInt8),
        mmp.lookup("ip", database, dtype={"n": pl.UInt8}).struct.field("n"),
    ]
    for expression in expressions:
        result = pl.DataFrame({"ip": ["1.0.0.1"]}).select(expression)
        assert result.to_series().to_list() == [255]
        with pytest.raises(
            pl.exceptions.ComputeError,
            match="MMDB value 256 does not fit in requested dtype u8",
        ):
            pl.DataFrame({"ip": ["2.0.0.1"]}).select(expression)


@pytest.mark.parametrize("engine", ["auto", "in-memory", "streaming"])
@pytest.mark.parametrize("strict", [True, False])
@pytest.mark.parametrize("rows", [4, 8192, 8193])
def test_unsigned_scalar_list_and_projection_agree(
    engine: Literal["auto", "in-memory", "streaming"], strict: bool, rows: int
) -> None:
    ips = ["::1.1.1.0", None, "2001:db8::1", "::1.1.1.0"]
    frame = pl.DataFrame({"ip": [ips[i % 4] for i in range(rows)]})
    expressions = [
        mmp.lookup_path(
            "ip", CUSTOM_DB, ["array", 0], dtype=pl.UInt16, strict=strict
        ).alias("scalar"),
        mmp.lookup_path(
            "ip", CUSTOM_DB, ["array"], dtype=pl.List(pl.UInt16), strict=strict
        ).alias("list"),
        mmp.lookup(
            "ip", CUSTOM_DB, dtype={"array": pl.List(pl.UInt16)}, strict=strict
        ).alias("record"),
    ]
    eager = frame.select(expressions)
    result = frame.lazy().select(expressions).collect(engine=engine)
    assert result.equals(eager)
    assert result.schema == {
        "scalar": pl.UInt16,
        "list": pl.List(pl.UInt16),
        "record": pl.Struct({"array": pl.List(pl.UInt16)}),
    }
    assert result["scalar"].to_list() == [
        1 if ips[i % 4] == "::1.1.1.0" else None for i in range(rows)
    ]
    assert result["list"].list.first().equals(result["scalar"])
    assert result["record"].struct.field("array").equals(result["list"])


@pytest.mark.parametrize("strict", [True, False])
@pytest.mark.parametrize(
    "dtype", [pl.UInt8, pl.UInt16, pl.UInt32, pl.UInt64, pl.UInt128]
)
@pytest.mark.parametrize(
    "field", ["float", "double", "boolean", "utf8_string", "bytes"]
)
@pytest.mark.parametrize("ip", ["::0.0.0.0", "::1.1.1.0"])
def test_unsigned_conversion_rejects_non_integer_values(
    strict: bool, dtype: Any, field: str, ip: str
) -> None:
    frame = pl.DataFrame({"ip": [ip]})
    for expression in [
        mmp.lookup_path("ip", CUSTOM_DB, [field], dtype=dtype, strict=strict),
        mmp.lookup("ip", CUSTOM_DB, dtype={field: dtype}, strict=strict),
    ]:
        with pytest.raises(pl.exceptions.ComputeError, match="could not decode MMDB"):
            frame.select(expression)


@pytest.mark.parametrize("field", ["array", "map"])
def test_unsigned_conversion_rejects_containers(field: str) -> None:
    frame = pl.DataFrame({"ip": ["::1.1.1.0"]})
    for expression in [
        mmp.lookup_path("ip", CUSTOM_DB, [field], dtype=pl.UInt16),
        mmp.lookup("ip", CUSTOM_DB, dtype={field: pl.UInt16}),
    ]:
        with pytest.raises(
            pl.exceptions.ComputeError, match="expected an MMDB integer"
        ):
            frame.select(expression)


def test_unsigned_overflow_names_the_value() -> None:
    frame = pl.DataFrame({"ip": ["::255.255.255.255"]})
    for expression in [
        mmp.lookup_path("ip", CUSTOM_DB, ["uint32"], dtype=pl.UInt16),
        mmp.lookup("ip", CUSTOM_DB, dtype={"uint32": pl.UInt16}),
    ]:
        with pytest.raises(
            pl.exceptions.ComputeError,
            match="MMDB value 4294967295 does not fit in requested dtype u16",
        ):
            frame.select(expression)


@pytest.mark.parametrize(
    "dtype", [pl.UInt8, pl.UInt16, pl.UInt32, pl.UInt64, pl.UInt128]
)
def test_unsigned_dtype_accepts_signed_values_that_fit(dtype: Any) -> None:
    frame = pl.DataFrame({"ip": ["::0.0.0.0"]})
    assert frame.select(
        mmp.lookup_path("ip", CUSTOM_DB, ["int32"], dtype=dtype)
    ).to_series().to_list() == [0]
    assert frame.select(
        mmp.lookup("ip", CUSTOM_DB, dtype={"int32": dtype})
    ).to_series().to_list() == [{"int32": 0}]


@pytest.mark.parametrize(
    "dtype,ip,value",
    [
        (pl.UInt8, "::255.255.255.255", 2147483647),
        (pl.UInt16, "::255.255.255.255", 2147483647),
        (pl.UInt32, "::1.1.1.0", -268435456),
        (pl.UInt128, "::1.1.1.0", -268435456),
    ],
)
def test_unsigned_signed_overflow_names_the_value(
    dtype: Any, ip: str, value: int
) -> None:
    frame = pl.DataFrame({"ip": [ip]})
    for expression in [
        mmp.lookup_path("ip", CUSTOM_DB, ["int32"], dtype=dtype),
        mmp.lookup("ip", CUSTOM_DB, dtype={"int32": dtype}),
    ]:
        with pytest.raises(
            pl.exceptions.ComputeError,
            match=f"MMDB value {value} does not fit in requested dtype",
        ):
            frame.select(expression)


def test_unsigned_type_error_omits_stored_string() -> None:
    frame = pl.DataFrame({"ip": ["::1.1.1.0"]})
    with pytest.raises(
        pl.exceptions.ComputeError, match="expected an MMDB integer"
    ) as error:
        frame.select(mmp.lookup_path("ip", CUSTOM_DB, ["utf8_string"], dtype=pl.UInt16))
    assert "unicode" not in str(error.value)


def test_unsigned_field_can_mix_encoded_widths(tmp_path: Path) -> None:
    database = tmp_path / "mixed.mmdb"
    write_mmdb(
        database,
        "Custom-Mixed",
        {1: {"n": 100}, 2: {"n": 70_000}, 3: {"n": mmdb_int32(5)}},
    )
    frame = pl.DataFrame({"ip": ["1.0.0.1", "2.0.0.1", "3.0.0.1"]})
    assert frame.select(
        mmp.lookup_path("ip", database, ["n"], dtype=pl.UInt32)
    ).to_series().to_list() == [100, 70_000, 5]
    assert frame.select(
        mmp.lookup("ip", database, dtype={"n": pl.UInt32})
    ).to_series().to_list() == [{"n": 100}, {"n": 70_000}, {"n": 5}]
    with pytest.raises(
        pl.exceptions.ComputeError,
        match="MMDB value 70000 does not fit in requested dtype u16",
    ):
        frame.select(mmp.lookup_path("ip", database, ["n"], dtype=pl.UInt16))


def test_unsigned_dtype_reads_wide_values_and_long_keys(tmp_path: Path) -> None:
    database = tmp_path / "wide.mmdb"
    key = "k" * 300
    write_mmdb(database, "Custom-Wide", {1: {key: 2**40}, 2: {key: 2**100}})
    frame = pl.DataFrame({"ip": ["1.0.0.1", "2.0.0.1"]})
    assert frame.select(
        mmp.lookup_path("ip", database, [key], dtype=pl.UInt128)
    ).to_series().to_list() == [2**40, 2**100]


def test_inferred_unsigned_dtype_can_mix_encoded_widths(tmp_path: Path) -> None:
    database = tmp_path / "asn.mmdb"
    field = "autonomous_system_number"
    write_mmdb(database, "GeoLite2-ASN", {1: {field: 100}, 2: {field: 70_000}})
    frame = pl.DataFrame({"ip": ["1.0.0.1", "2.0.0.1"]})
    result = frame.select(mmp.lookup_path("ip", database, [field]))
    assert result.schema == {"ip": pl.UInt32}
    assert result.to_series().to_list() == [100, 70_000]


def test_unsigned_conversion_preserves_invalid_and_missing_paths() -> None:
    frame = pl.DataFrame({"ip": ["::1.1.1.0", "not-an-ip", None]})
    expression = mmp.lookup_path("ip", CUSTOM_DB, ["array", 0], dtype=pl.UInt16)
    with pytest.raises(pl.exceptions.ComputeError, match="invalid IP address"):
        frame.select(expression)
    result = frame.select(
        mmp.lookup_path("ip", CUSTOM_DB, ["array", 0], dtype=pl.UInt16, strict=False)
    )
    assert result.to_series().to_list() == [1, None, None]
    paths: list[list[str | int]] = [["missing"], ["array", 10]]
    for path in paths:
        result = frame.select(
            mmp.lookup_path("ip", CUSTOM_DB, path, dtype=pl.UInt16, strict=False)
        )
        assert result.to_series().to_list() == [None, None, None]


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


@pytest.mark.parametrize("engine", ["auto", "in-memory", "streaming"])
def test_arbitrary_nested_custom_record(
    engine: Literal["auto", "in-memory", "streaming"],
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


def write_mmdb(path: Path, database_type: str, records: dict[int, Any]) -> None:
    """Write an IPv4 MMDB that maps each first octet to one record."""
    data = b""
    offsets = {}
    for octet, record in records.items():
        offsets[octet] = len(data)
        data += encode_mmdb(record)

    nodes: list[list[int | tuple[int] | None]] = [[None, None]]
    for octet, offset in offsets.items():
        bits = [(octet >> (7 - index)) & 1 for index in range(8)]
        node = 0
        for bit in bits[:-1]:
            child = nodes[node][bit]
            if child is None:
                nodes.append([None, None])
                child = len(nodes) - 1
                nodes[node][bit] = child
            assert isinstance(child, int)
            node = child
        nodes[node][bits[-1]] = (offset,)

    def record_bytes(value: int | tuple[int] | None) -> bytes:
        if value is None:
            number = len(nodes)
        elif isinstance(value, tuple):
            number = len(nodes) + 16 + value[0]
        else:
            number = value
        return number.to_bytes(3, "big")

    tree = b"".join(record_bytes(left) + record_bytes(right) for left, right in nodes)
    metadata = encode_mmdb(
        {
            "binary_format_major_version": 2,
            "binary_format_minor_version": 0,
            "build_epoch": mmdb_uint(1_700_000_000, 9),
            "database_type": database_type,
            "description": {},
            "ip_version": 4,
            "languages": [],
            "node_count": mmdb_uint(len(nodes), 6),
            "record_size": 24,
        }
    )
    path.write_bytes(tree + bytes(16) + data + b"\xab\xcd\xefMaxMind.com" + metadata)


def encode_mmdb(value: Any) -> bytes:
    """Encode a value with the same width rules as mmdb_writer int_type='auto'."""
    if isinstance(value, bytes):
        return value
    if isinstance(value, str):
        encoded = value.encode()
        return mmdb_control(2, len(encoded)) + encoded
    if isinstance(value, dict):
        fields = [encode_mmdb(key) + encode_mmdb(item) for key, item in value.items()]
        return mmdb_control(7, len(fields)) + b"".join(fields)
    if isinstance(value, list):
        items = [encode_mmdb(item) for item in value]
        return mmdb_control(11, len(items)) + b"".join(items)
    if isinstance(value, int) and value < 0:
        return mmdb_int32(value)
    if isinstance(value, int):
        for kind, bits in [(5, 16), (6, 32), (9, 64), (10, 128)]:
            if value < 2**bits:
                return mmdb_uint(value, kind)
    raise TypeError(f"unsupported MMDB test value: {value!r}")


def mmdb_uint(value: int, kind: int) -> bytes:
    encoded = value.to_bytes((value.bit_length() + 7) // 8, "big")
    return mmdb_control(kind, len(encoded)) + encoded


def mmdb_int32(value: int) -> bytes:
    return mmdb_control(8, 4) + value.to_bytes(4, "big", signed=True)


def mmdb_control(kind: int, size: int) -> bytes:
    if size < 29:
        size_bits, size_bytes = size, b""
    elif size < 29 + 2**8:
        size_bits, size_bytes = 29, (size - 29).to_bytes(1, "big")
    elif size < 285 + 2**16:
        size_bits, size_bytes = 30, (size - 285).to_bytes(2, "big")
    else:
        size_bits, size_bytes = 31, (size - 65_821).to_bytes(3, "big")
    if kind <= 7:
        return bytes([kind << 5 | size_bits]) + size_bytes
    return bytes([size_bits, kind - 7]) + size_bytes

"""Check an installed wheel without importing the source package."""

from importlib.metadata import version
from pathlib import Path

import polars as pl

import maxminddb_polars as mmp
from maxminddb_polars import _maxminddb_polars


def main() -> None:
    repository = Path(__file__).resolve().parents[1]
    assert Path(mmp.__file__).resolve().parent != repository / "maxminddb_polars"
    assert _maxminddb_polars.__version__ == version("maxminddb-polars")
    database = repository / "tests/data/test-data/GeoIP2-City-Test.mmdb"
    frame = pl.DataFrame({"ip": ["89.160.20.128", None, "203.0.113.1"]})
    record = mmp.lookup("ip", database)
    expressions = [
        record.alias("record"),
        record.struct.field("country").struct.field("iso_code").alias("country"),
        mmp.lookup_path("ip", database, ["country", "iso_code"]).alias("path"),
        mmp.lookup("ip", database, dtype={"country": {"iso_code": pl.String}}).alias(
            "partial"
        ),
    ]
    expected = frame.select(expressions)
    assert expected.schema["record"] == mmp.schemas.CITY
    assert expected["country"].to_list() == ["SE", None, None]
    assert expected["path"].to_list() == ["SE", None, None]
    assert expected["partial"].to_list() == [
        {"country": {"iso_code": "SE"}},
        None,
        None,
    ]
    asn = pl.LazyFrame({"ip": ["1.0.0.0"]}).select(
        mmp.lookup("ip", database.with_name("GeoLite2-ASN-Test.mmdb"))
    )
    for engine in ("auto", "in-memory", "streaming"):
        assert frame.lazy().select(expressions).collect(engine=engine).equals(expected)
        assert (
            asn.collect(engine=engine)
            .to_series()
            .struct.field("autonomous_system_number")
            .item()
            == 15169
        )
    print(f"Validated installed wheel with Polars {pl.__version__}")


if __name__ == "__main__":
    main()

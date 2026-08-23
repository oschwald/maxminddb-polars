from __future__ import annotations

import concurrent.futures
import ipaddress
import json
import os
import shutil
from pathlib import Path
from typing import Literal

import polars as pl
import pytest

import maxminddb_polars as mmp

DATA = Path(__file__).parents[1] / "data"
DATABASES = DATA / "test-data"
SOURCES = DATA / "source-data"


def _first_ip(source: str) -> str:
    records: list[dict[str, object]] = json.loads(
        (SOURCES / source).read_text(encoding="utf-8")
    )
    network = next(iter(records[0]))
    return str(ipaddress.ip_network(network).network_address)


STANDARD_FIXTURES = [
    ("GeoIP2-City-Test.mmdb", "GeoIP2-City-Test.json", mmp.schemas.CITY),
    ("GeoLite2-City-Test.mmdb", "GeoLite2-City-Test.json", mmp.schemas.CITY),
    ("GeoIP2-City-Shield-Test.mmdb", "GeoIP2-City-Test.json", mmp.schemas.CITY),
    ("GeoIP2-Country-Test.mmdb", "GeoIP2-Country-Test.json", mmp.schemas.COUNTRY),
    ("GeoLite2-Country-Test.mmdb", "GeoLite2-Country-Test.json", mmp.schemas.COUNTRY),
    (
        "GeoIP2-Country-Shield-Test.mmdb",
        "GeoIP2-Country-Test.json",
        mmp.schemas.COUNTRY,
    ),
    (
        "GeoIP2-Enterprise-Test.mmdb",
        "GeoIP2-Enterprise-Test.json",
        mmp.schemas.ENTERPRISE,
    ),
    (
        "GeoIP2-Enterprise-Shield-Test.mmdb",
        "GeoIP2-Enterprise-Test.json",
        mmp.schemas.ENTERPRISE,
    ),
    (
        "GeoIP2-Precision-Enterprise-Test.mmdb",
        "GeoIP2-Precision-Enterprise-Test.json",
        mmp.schemas.ENTERPRISE,
    ),
    (
        "GeoIP2-Precision-Enterprise-Shield-Test.mmdb",
        "GeoIP2-Precision-Enterprise-Test.json",
        mmp.schemas.ENTERPRISE,
    ),
    ("GeoIP2-ISP-Test.mmdb", "GeoIP2-ISP-Test.json", mmp.schemas.ISP),
    (
        "GeoIP2-Connection-Type-Test.mmdb",
        "GeoIP2-Connection-Type-Test.json",
        mmp.schemas.CONNECTION_TYPE,
    ),
    (
        "GeoIP2-Anonymous-IP-Test.mmdb",
        "GeoIP2-Anonymous-IP-Test.json",
        mmp.schemas.ANONYMOUS_IP,
    ),
    (
        "GeoIP2-DensityIncome-Test.mmdb",
        "GeoIP2-DensityIncome-Test.json",
        mmp.schemas.DENSITY_INCOME,
    ),
    ("GeoIP2-Domain-Test.mmdb", "GeoIP2-Domain-Test.json", mmp.schemas.DOMAIN),
    ("GeoLite2-ASN-Test.mmdb", "GeoLite2-ASN-Test.json", mmp.schemas.ASN),
]


@pytest.mark.parametrize(("database", "source", "dtype"), STANDARD_FIXTURES)
def test_every_standard_fixture_has_a_stable_materialized_schema(
    database: str, source: str, dtype: pl.Struct
) -> None:
    frame = pl.DataFrame({"ip": [None, _first_ip(source)]})

    result = frame.select(mmp.lookup("ip", DATABASES / database).alias("record"))

    assert result.schema == {"record": dtype}
    assert result.item(0, 0) is None
    assert result.item(1, 0) is not None


@pytest.mark.parametrize("engine", ["auto", "streaming"])
def test_whole_city_is_identical_eager_lazy_and_streaming(
    engine: Literal["auto", "streaming"],
) -> None:
    frame = pl.DataFrame({"ip": ["89.160.20.128", None, "203.0.113.1"]})
    expression = mmp.lookup("ip", DATABASES / "GeoIP2-City-Test.mmdb").alias("record")
    eager = frame.select(expression)
    lazy = frame.lazy().select(expression).collect(engine=engine)

    assert eager.schema == {"record": mmp.schemas.CITY}
    assert eager["record"].struct.field("country").struct.field(
        "iso_code"
    ).to_list() == [
        "SE",
        None,
        None,
    ]
    assert lazy.equals(eager)


def test_repeated_whole_city_uses_arrow_gathers() -> None:
    rows = 100_000
    ips = ["89.160.20.128", "89.160.20.129", None, "203.0.113.1"]
    frame = pl.DataFrame({"ip": (ips * ((rows + len(ips) - 1) // len(ips)))[:rows]})

    result = frame.select(
        mmp.lookup("ip", DATABASES / "GeoIP2-City-Test.mmdb").alias("record")
    )

    assert result.height == rows
    assert result["record"].null_count() == rows // 2
    assert result["record"].struct.field("country").struct.field("iso_code").head(
        4
    ).to_list() == ["SE", "SE", None, None]


def test_whole_record_namespace_matches_standalone() -> None:
    database = DATABASES / "GeoLite2-ASN-Test.mmdb"
    result = pl.DataFrame({"ip": ["1.0.0.0"]}).select(
        standalone=mmp.lookup("ip", database),
        namespace=pl.col("ip").mmdb.lookup(database),  # type: ignore[attr-defined]
    )
    assert result["standalone"].equals(result["namespace"])


def test_empty_and_all_null_input_keep_the_static_schema() -> None:
    database = DATABASES / "GeoIP2-Country-Test.mmdb"
    for frame in [
        pl.DataFrame({"ip": []}, schema={"ip": pl.String}),
        pl.DataFrame({"ip": [None, None]}, schema={"ip": pl.String}),
    ]:
        result = frame.select(mmp.lookup("ip", database).alias("record"))
        assert result.schema == {"record": mmp.schemas.COUNTRY}
        assert result.height == frame.height
        assert result["record"].null_count() == frame.height


def test_reader_cache_is_safe_across_concurrent_evaluations() -> None:
    database = DATABASES / "GeoIP2-City-Test.mmdb"
    expression = mmp.lookup_path("ip", database, ["country", "iso_code"])

    def evaluate(_: int) -> list[str | None]:
        return (
            pl.DataFrame({"ip": ["89.160.20.128", None] * 500})
            .select(expression)
            .to_series()
            .to_list()
        )

    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as executor:
        outputs = list(executor.map(evaluate, range(16)))
    assert all(output[:4] == ["SE", None, "SE", None] for output in outputs)


def test_planned_expression_keeps_its_snapshot_after_atomic_replacement(
    tmp_path: Path,
) -> None:
    database = tmp_path / "database.mmdb"
    shutil.copy2(DATABASES / "GeoIP2-City-Test.mmdb", database)
    old_query = (
        pl.DataFrame({"ip": ["89.160.20.128"]})
        .lazy()
        .select(mmp.lookup("ip", database).alias("record"))
    )
    assert old_query.collect_schema() == {"record": mmp.schemas.CITY}

    replacement = tmp_path / "replacement.mmdb"
    shutil.copy2(DATABASES / "GeoIP2-Country-Test.mmdb", replacement)
    os.replace(replacement, database)

    old_result = old_query.collect()
    assert old_result.schema == {"record": mmp.schemas.CITY}
    assert old_result["record"].struct.field("city").is_not_null().item()

    new_query = (
        pl.DataFrame({"ip": ["89.160.20.128"]})
        .lazy()
        .select(mmp.lookup("ip", database).alias("record"))
    )
    assert new_query.collect_schema() == {"record": mmp.schemas.COUNTRY}

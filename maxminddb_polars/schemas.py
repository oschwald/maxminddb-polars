"""Stable Polars Struct dtypes for standard MaxMind database records."""

from __future__ import annotations

import polars as pl


def _struct(**fields: pl.DataType | type[pl.DataType]) -> pl.Struct:
    return pl.Struct(fields)


NAMES = _struct(
    de=pl.String,
    en=pl.String,
    es=pl.String,
    fr=pl.String,
    ja=pl.String,
    **{"pt-BR": pl.String, "ru": pl.String, "zh-CN": pl.String},
)

CONTINENT = _struct(code=pl.String, geoname_id=pl.UInt32, names=NAMES)
COUNTRY_RECORD = _struct(
    geoname_id=pl.UInt32,
    is_in_european_union=pl.Boolean,
    iso_code=pl.String,
    names=NAMES,
)
REPRESENTED_COUNTRY = pl.Struct([*COUNTRY_RECORD.fields, pl.Field("type", pl.String)])
COUNTRY_TRAITS = _struct(is_anycast=pl.Boolean)

COUNTRY = _struct(
    continent=CONTINENT,
    country=COUNTRY_RECORD,
    registered_country=COUNTRY_RECORD,
    represented_country=REPRESENTED_COUNTRY,
    traits=COUNTRY_TRAITS,
)

SUBDIVISION = _struct(geoname_id=pl.UInt32, iso_code=pl.String, names=NAMES)
CITY = _struct(
    city=_struct(geoname_id=pl.UInt32, names=NAMES),
    continent=CONTINENT,
    country=COUNTRY_RECORD,
    location=_struct(
        accuracy_radius=pl.UInt16,
        latitude=pl.Float64,
        longitude=pl.Float64,
        metro_code=pl.UInt16,
        time_zone=pl.String,
    ),
    postal=_struct(code=pl.String),
    registered_country=COUNTRY_RECORD,
    represented_country=REPRESENTED_COUNTRY,
    subdivisions=pl.List(SUBDIVISION),
    traits=COUNTRY_TRAITS,
)

ENTERPRISE_COUNTRY = _struct(
    confidence=pl.UInt8,
    geoname_id=pl.UInt32,
    is_in_european_union=pl.Boolean,
    iso_code=pl.String,
    names=NAMES,
)
ENTERPRISE_SUBDIVISION = _struct(
    confidence=pl.UInt8,
    geoname_id=pl.UInt32,
    iso_code=pl.String,
    names=NAMES,
)
ENTERPRISE = _struct(
    city=_struct(confidence=pl.UInt8, geoname_id=pl.UInt32, names=NAMES),
    continent=CONTINENT,
    country=ENTERPRISE_COUNTRY,
    location=_struct(
        accuracy_radius=pl.UInt16,
        latitude=pl.Float64,
        longitude=pl.Float64,
        metro_code=pl.UInt16,
        time_zone=pl.String,
    ),
    postal=_struct(code=pl.String, confidence=pl.UInt8),
    registered_country=ENTERPRISE_COUNTRY,
    represented_country=REPRESENTED_COUNTRY,
    subdivisions=pl.List(ENTERPRISE_SUBDIVISION),
    traits=_struct(
        autonomous_system_number=pl.UInt32,
        autonomous_system_organization=pl.String,
        connection_type=pl.String,
        domain=pl.String,
        is_anonymous=pl.Boolean,
        is_anonymous_vpn=pl.Boolean,
        is_anycast=pl.Boolean,
        is_hosting_provider=pl.Boolean,
        isp=pl.String,
        is_public_proxy=pl.Boolean,
        is_residential_proxy=pl.Boolean,
        is_tor_exit_node=pl.Boolean,
        mobile_country_code=pl.String,
        mobile_network_code=pl.String,
        organization=pl.String,
        user_type=pl.String,
    ),
)

ISP = _struct(
    autonomous_system_number=pl.UInt32,
    autonomous_system_organization=pl.String,
    isp=pl.String,
    mobile_country_code=pl.String,
    mobile_network_code=pl.String,
    organization=pl.String,
)
CONNECTION_TYPE = _struct(connection_type=pl.String)
ANONYMOUS_IP = _struct(
    is_anonymous=pl.Boolean,
    is_anonymous_vpn=pl.Boolean,
    is_hosting_provider=pl.Boolean,
    is_public_proxy=pl.Boolean,
    is_residential_proxy=pl.Boolean,
    is_tor_exit_node=pl.Boolean,
)
DENSITY_INCOME = _struct(
    average_income=pl.UInt32,
    population_density=pl.UInt32,
)
DOMAIN = _struct(domain=pl.String)
ASN = _struct(
    autonomous_system_number=pl.UInt32,
    autonomous_system_organization=pl.String,
)

__all__ = [
    "ANONYMOUS_IP",
    "ASN",
    "CITY",
    "CONNECTION_TYPE",
    "COUNTRY",
    "DENSITY_INCOME",
    "DOMAIN",
    "ENTERPRISE",
    "ISP",
]

use std::borrow::Cow;

use maxminddb::{LookupResult, geoip2};
use polars::prelude::*;

use crate::schema::KnownRecord;
use crate::value::Value;

pub fn decode_known<'a>(
    result: &LookupResult<'a, Vec<u8>>,
    record: KnownRecord,
) -> PolarsResult<Option<Value<'a>>> {
    macro_rules! decode {
        ($ty:ty, $convert:expr, $name:literal) => {
            result
                .decode::<$ty>()
                .map_err(|error| {
                    polars_err!(ComputeError: "could not decode {} record: {error}", $name)
                })?
                .map($convert)
        };
    }

    Ok(match record {
        KnownRecord::City => decode!(geoip2::City<'a>, city_record, "City"),
        KnownRecord::Country => decode!(geoip2::Country<'a>, country_record, "Country"),
        KnownRecord::Enterprise => {
            decode!(geoip2::Enterprise<'a>, enterprise_record, "Enterprise")
        }
        KnownRecord::Isp => decode!(geoip2::Isp<'a>, isp_record, "ISP"),
        KnownRecord::ConnectionType => decode!(
            geoip2::ConnectionType<'a>,
            connection_type_record,
            "Connection Type"
        ),
        KnownRecord::AnonymousIp => {
            decode!(geoip2::AnonymousIp, anonymous_ip_record, "Anonymous IP")
        }
        KnownRecord::DensityIncome => decode!(
            geoip2::DensityIncome,
            density_income_record,
            "Density/Income"
        ),
        KnownRecord::Domain => decode!(geoip2::Domain<'a>, domain_record, "Domain"),
        KnownRecord::Asn => decode!(geoip2::Asn<'a>, asn_record, "ASN"),
    })
}

fn map<'a>(fields: Vec<(&'static str, Value<'a>)>) -> Value<'a> {
    Value::Map(
        fields
            .into_iter()
            .map(|(name, value)| (Cow::Borrowed(name), value))
            .collect(),
    )
}

fn string(value: Option<&str>) -> Value<'_> {
    value
        .map(|value| Value::String(Cow::Borrowed(value)))
        .unwrap_or(Value::Null)
}

fn boolean(value: Option<bool>) -> Value<'static> {
    value.map(Value::Boolean).unwrap_or(Value::Null)
}

fn uint8(value: Option<u8>) -> Value<'static> {
    value.map(Value::UInt8).unwrap_or(Value::Null)
}

fn uint16(value: Option<u16>) -> Value<'static> {
    value.map(Value::UInt16).unwrap_or(Value::Null)
}

fn uint32(value: Option<u32>) -> Value<'static> {
    value.map(Value::UInt32).unwrap_or(Value::Null)
}

fn float64(value: Option<f64>) -> Value<'static> {
    value.map(Value::Float64).unwrap_or(Value::Null)
}

fn names(names: geoip2::Names<'_>) -> Value<'_> {
    let geoip2::Names {
        german,
        english,
        spanish,
        french,
        japanese,
        brazilian_portuguese,
        russian,
        simplified_chinese,
    } = names;
    map(vec![
        ("de", string(german)),
        ("en", string(english)),
        ("es", string(spanish)),
        ("fr", string(french)),
        ("ja", string(japanese)),
        ("pt-BR", string(brazilian_portuguese)),
        ("ru", string(russian)),
        ("zh-CN", string(simplified_chinese)),
    ])
}

fn continent(continent: geoip2::country::Continent<'_>) -> Value<'_> {
    let geoip2::country::Continent {
        code,
        geoname_id,
        names: localized_names,
    } = continent;
    map(vec![
        ("code", string(code)),
        ("geoname_id", uint32(geoname_id)),
        ("names", names(localized_names)),
    ])
}

fn country(country: geoip2::country::Country<'_>) -> Value<'_> {
    let geoip2::country::Country {
        geoname_id,
        is_in_european_union,
        iso_code,
        names: localized_names,
    } = country;
    map(vec![
        ("geoname_id", uint32(geoname_id)),
        ("is_in_european_union", boolean(is_in_european_union)),
        ("iso_code", string(iso_code)),
        ("names", names(localized_names)),
    ])
}

fn represented_country(country: geoip2::country::RepresentedCountry<'_>) -> Value<'_> {
    let geoip2::country::RepresentedCountry {
        geoname_id,
        is_in_european_union,
        iso_code,
        names: localized_names,
        representation_type,
    } = country;
    map(vec![
        ("geoname_id", uint32(geoname_id)),
        ("is_in_european_union", boolean(is_in_european_union)),
        ("iso_code", string(iso_code)),
        ("names", names(localized_names)),
        ("type", string(representation_type)),
    ])
}

fn country_traits(traits: geoip2::country::Traits) -> Value<'static> {
    let geoip2::country::Traits { is_anycast } = traits;
    map(vec![("is_anycast", boolean(is_anycast))])
}

fn country_record(record: geoip2::Country<'_>) -> Value<'_> {
    let geoip2::Country {
        continent: record_continent,
        country: record_country,
        registered_country,
        represented_country: record_represented_country,
        traits,
    } = record;
    map(vec![
        ("continent", continent(record_continent)),
        ("country", country(record_country)),
        ("registered_country", country(registered_country)),
        (
            "represented_country",
            represented_country(record_represented_country),
        ),
        ("traits", country_traits(traits)),
    ])
}

fn city(record: geoip2::city::City<'_>) -> Value<'_> {
    let geoip2::city::City {
        geoname_id,
        names: localized_names,
    } = record;
    map(vec![
        ("geoname_id", uint32(geoname_id)),
        ("names", names(localized_names)),
    ])
}

fn location(record: geoip2::city::Location<'_>) -> Value<'_> {
    let geoip2::city::Location {
        accuracy_radius,
        latitude,
        longitude,
        metro_code,
        time_zone,
    } = record;
    map(vec![
        ("accuracy_radius", uint16(accuracy_radius)),
        ("latitude", float64(latitude)),
        ("longitude", float64(longitude)),
        ("metro_code", uint16(metro_code)),
        ("time_zone", string(time_zone)),
    ])
}

fn postal(record: geoip2::city::Postal<'_>) -> Value<'_> {
    let geoip2::city::Postal { code } = record;
    map(vec![("code", string(code))])
}

fn subdivision(record: geoip2::city::Subdivision<'_>) -> Value<'_> {
    let geoip2::city::Subdivision {
        geoname_id,
        iso_code,
        names: localized_names,
    } = record;
    map(vec![
        ("geoname_id", uint32(geoname_id)),
        ("iso_code", string(iso_code)),
        ("names", names(localized_names)),
    ])
}

fn city_record(record: geoip2::City<'_>) -> Value<'_> {
    let geoip2::City {
        city: record_city,
        continent: record_continent,
        country: record_country,
        location: record_location,
        postal: record_postal,
        registered_country,
        represented_country: record_represented_country,
        subdivisions,
        traits,
    } = record;
    map(vec![
        ("city", city(record_city)),
        ("continent", continent(record_continent)),
        ("country", country(record_country)),
        ("location", location(record_location)),
        ("postal", postal(record_postal)),
        ("registered_country", country(registered_country)),
        (
            "represented_country",
            represented_country(record_represented_country),
        ),
        (
            "subdivisions",
            Value::List(subdivisions.into_iter().map(subdivision).collect()),
        ),
        ("traits", country_traits(traits)),
    ])
}

fn enterprise_city(record: geoip2::enterprise::City<'_>) -> Value<'_> {
    let geoip2::enterprise::City {
        confidence,
        geoname_id,
        names: localized_names,
    } = record;
    map(vec![
        ("confidence", uint8(confidence)),
        ("geoname_id", uint32(geoname_id)),
        ("names", names(localized_names)),
    ])
}

fn enterprise_country(record: geoip2::enterprise::Country<'_>) -> Value<'_> {
    let geoip2::enterprise::Country {
        confidence,
        geoname_id,
        is_in_european_union,
        iso_code,
        names: localized_names,
    } = record;
    map(vec![
        ("confidence", uint8(confidence)),
        ("geoname_id", uint32(geoname_id)),
        ("is_in_european_union", boolean(is_in_european_union)),
        ("iso_code", string(iso_code)),
        ("names", names(localized_names)),
    ])
}

fn enterprise_location(record: geoip2::enterprise::Location<'_>) -> Value<'_> {
    let geoip2::enterprise::Location {
        accuracy_radius,
        latitude,
        longitude,
        metro_code,
        time_zone,
    } = record;
    map(vec![
        ("accuracy_radius", uint16(accuracy_radius)),
        ("latitude", float64(latitude)),
        ("longitude", float64(longitude)),
        ("metro_code", uint16(metro_code)),
        ("time_zone", string(time_zone)),
    ])
}

fn enterprise_postal(record: geoip2::enterprise::Postal<'_>) -> Value<'_> {
    let geoip2::enterprise::Postal { code, confidence } = record;
    map(vec![
        ("code", string(code)),
        ("confidence", uint8(confidence)),
    ])
}

fn enterprise_subdivision(record: geoip2::enterprise::Subdivision<'_>) -> Value<'_> {
    let geoip2::enterprise::Subdivision {
        confidence,
        geoname_id,
        iso_code,
        names: localized_names,
    } = record;
    map(vec![
        ("confidence", uint8(confidence)),
        ("geoname_id", uint32(geoname_id)),
        ("iso_code", string(iso_code)),
        ("names", names(localized_names)),
    ])
}

fn enterprise_traits(record: geoip2::enterprise::Traits<'_>) -> Value<'_> {
    let geoip2::enterprise::Traits {
        autonomous_system_number,
        autonomous_system_organization,
        connection_type,
        domain,
        is_anonymous,
        is_anonymous_vpn,
        is_anycast,
        is_hosting_provider,
        isp,
        is_public_proxy,
        is_residential_proxy,
        is_tor_exit_node,
        mobile_country_code,
        mobile_network_code,
        organization,
        user_type,
    } = record;
    map(vec![
        ("autonomous_system_number", uint32(autonomous_system_number)),
        (
            "autonomous_system_organization",
            string(autonomous_system_organization),
        ),
        ("connection_type", string(connection_type)),
        ("domain", string(domain)),
        ("is_anonymous", boolean(is_anonymous)),
        ("is_anonymous_vpn", boolean(is_anonymous_vpn)),
        ("is_anycast", boolean(is_anycast)),
        ("is_hosting_provider", boolean(is_hosting_provider)),
        ("isp", string(isp)),
        ("is_public_proxy", boolean(is_public_proxy)),
        ("is_residential_proxy", boolean(is_residential_proxy)),
        ("is_tor_exit_node", boolean(is_tor_exit_node)),
        ("mobile_country_code", string(mobile_country_code)),
        ("mobile_network_code", string(mobile_network_code)),
        ("organization", string(organization)),
        ("user_type", string(user_type)),
    ])
}

fn enterprise_record(record: geoip2::Enterprise<'_>) -> Value<'_> {
    let geoip2::Enterprise {
        city,
        continent: record_continent,
        country,
        location,
        postal,
        registered_country,
        represented_country: record_represented_country,
        subdivisions,
        traits,
    } = record;
    map(vec![
        ("city", enterprise_city(city)),
        ("continent", continent(record_continent)),
        ("country", enterprise_country(country)),
        ("location", enterprise_location(location)),
        ("postal", enterprise_postal(postal)),
        ("registered_country", enterprise_country(registered_country)),
        (
            "represented_country",
            represented_country(record_represented_country),
        ),
        (
            "subdivisions",
            Value::List(
                subdivisions
                    .into_iter()
                    .map(enterprise_subdivision)
                    .collect(),
            ),
        ),
        ("traits", enterprise_traits(traits)),
    ])
}

fn isp_record(record: geoip2::Isp<'_>) -> Value<'_> {
    let geoip2::Isp {
        autonomous_system_number,
        autonomous_system_organization,
        isp,
        mobile_country_code,
        mobile_network_code,
        organization,
    } = record;
    map(vec![
        ("autonomous_system_number", uint32(autonomous_system_number)),
        (
            "autonomous_system_organization",
            string(autonomous_system_organization),
        ),
        ("isp", string(isp)),
        ("mobile_country_code", string(mobile_country_code)),
        ("mobile_network_code", string(mobile_network_code)),
        ("organization", string(organization)),
    ])
}

fn connection_type_record(record: geoip2::ConnectionType<'_>) -> Value<'_> {
    let geoip2::ConnectionType { connection_type } = record;
    map(vec![("connection_type", string(connection_type))])
}

fn anonymous_ip_record(record: geoip2::AnonymousIp) -> Value<'static> {
    let geoip2::AnonymousIp {
        is_anonymous,
        is_anonymous_vpn,
        is_hosting_provider,
        is_public_proxy,
        is_residential_proxy,
        is_tor_exit_node,
    } = record;
    map(vec![
        ("is_anonymous", boolean(is_anonymous)),
        ("is_anonymous_vpn", boolean(is_anonymous_vpn)),
        ("is_hosting_provider", boolean(is_hosting_provider)),
        ("is_public_proxy", boolean(is_public_proxy)),
        ("is_residential_proxy", boolean(is_residential_proxy)),
        ("is_tor_exit_node", boolean(is_tor_exit_node)),
    ])
}

fn density_income_record(record: geoip2::DensityIncome) -> Value<'static> {
    let geoip2::DensityIncome {
        average_income,
        population_density,
    } = record;
    map(vec![
        ("average_income", uint32(average_income)),
        ("population_density", uint32(population_density)),
    ])
}

fn domain_record(record: geoip2::Domain<'_>) -> Value<'_> {
    let geoip2::Domain { domain } = record;
    map(vec![("domain", string(domain))])
}

fn asn_record(record: geoip2::Asn<'_>) -> Value<'_> {
    let geoip2::Asn {
        autonomous_system_number,
        autonomous_system_organization,
    } = record;
    map(vec![
        ("autonomous_system_number", uint32(autonomous_system_number)),
        (
            "autonomous_system_organization",
            string(autonomous_system_organization),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use std::net::IpAddr;

    use maxminddb::Reader;

    use super::*;

    #[test]
    fn typed_city_conversion_retains_declared_nested_containers() {
        let reader = Reader::open_readfile("tests/data/test-data/GeoIP2-City-Test.mmdb").unwrap();
        let result = reader
            .lookup("89.160.20.128".parse::<IpAddr>().unwrap())
            .unwrap();
        let value = decode_known(&result, KnownRecord::City).unwrap().unwrap();
        let Value::Map(fields) = value else {
            panic!("expected map")
        };
        assert!(fields.iter().any(|(name, _)| name == "subdivisions"));
        assert!(fields.iter().any(|(name, _)| name == "traits"));
    }
}

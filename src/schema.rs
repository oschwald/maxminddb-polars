use maxminddb::PathElement;
use polars::prelude::*;
use serde::{Deserialize, Serialize};

use crate::cache::CachedReader;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum PathPart {
    Key(String),
    Index(i64),
}

impl PathPart {
    fn render(&self) -> String {
        match self {
            Self::Key(key) => key.clone(),
            Self::Index(index) => index.to_string(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum SchemaSpec {
    Boolean,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    UInt128,
    Int8,
    Int16,
    Int32,
    Int64,
    Int128,
    Float32,
    Float64,
    String,
    Binary,
    List { inner: Box<SchemaSpec> },
    Struct { fields: Vec<SchemaField> },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SchemaField {
    pub name: String,
    pub dtype: SchemaSpec,
}

impl SchemaSpec {
    pub fn to_polars(&self) -> DataType {
        match self {
            Self::Boolean => DataType::Boolean,
            Self::UInt8 => DataType::UInt8,
            Self::UInt16 => DataType::UInt16,
            Self::UInt32 => DataType::UInt32,
            Self::UInt64 => DataType::UInt64,
            Self::UInt128 => DataType::UInt128,
            Self::Int8 => DataType::Int8,
            Self::Int16 => DataType::Int16,
            Self::Int32 => DataType::Int32,
            Self::Int64 => DataType::Int64,
            Self::Int128 => DataType::Int128,
            Self::Float32 => DataType::Float32,
            Self::Float64 => DataType::Float64,
            Self::String => DataType::String,
            Self::Binary => DataType::Binary,
            Self::List { inner } => DataType::List(Box::new(inner.to_polars())),
            Self::Struct { fields } => DataType::Struct(
                fields
                    .iter()
                    .map(|field| Field::new(field.name.clone().into(), field.dtype.to_polars()))
                    .collect(),
            ),
        }
    }
}

pub fn resolve_path_dtype(
    reader: &CachedReader,
    path: &[PathPart],
    explicit: Option<&SchemaSpec>,
) -> PolarsResult<SchemaSpec> {
    if path.is_empty() {
        polars_bail!(InvalidOperation: "MMDB lookup path must not be empty")
    }

    let inferred = known_schema(&reader.metadata().database_type)
        .map(|schema| dtype_at_path(schema, path))
        .transpose()?;

    match (explicit, inferred) {
        (Some(explicit), Some(inferred)) if explicit != &inferred => {
            polars_bail!(
                InvalidOperation:
                "explicit dtype {} does not match known MMDB path dtype {} at /{}",
                explicit.to_polars(),
                inferred.to_polars(),
                render_path(path)
            )
        }
        (Some(explicit), _) => Ok(explicit.clone()),
        (None, Some(inferred)) => Ok(inferred),
        (None, None) => {
            let database_type = &reader.metadata().database_type;
            polars_bail!(
                InvalidOperation:
                "dtype cannot be inferred for MMDB database_type {database_type:?}; pass dtype explicitly"
            )
        }
    }
}

pub fn dtype_at_path(mut dtype: SchemaSpec, path: &[PathPart]) -> PolarsResult<SchemaSpec> {
    for (position, part) in path.iter().enumerate() {
        dtype = match (part, dtype) {
            (PathPart::Key(key), SchemaSpec::Struct { fields }) => fields
                .into_iter()
                .find(|field| field.name == *key)
                .map(|field| field.dtype)
                .ok_or_else(|| {
                    polars_err!(
                        InvalidOperation:
                        "known MMDB schema has no field {key:?} at path /{}",
                        render_path(&path[..position])
                    )
                })?,
            (PathPart::Index(_), SchemaSpec::List { inner }) => *inner,
            (part, dtype) => {
                polars_bail!(
                    InvalidOperation:
                    "cannot apply path component {:?} to inferred dtype {} at /{}",
                    part.render(),
                    dtype.to_polars(),
                    render_path(&path[..position])
                )
            }
        };
    }
    Ok(dtype)
}

pub fn to_mmdb_path(path: &[PathPart]) -> PolarsResult<Vec<PathElement<'_>>> {
    path.iter()
        .map(|part| match part {
            PathPart::Key(key) => Ok(PathElement::Key(key)),
            PathPart::Index(index) if *index >= 0 => usize::try_from(*index)
                .map(PathElement::Index)
                .map_err(|_| polars_err!(InvalidOperation: "path index {index} is too large")),
            PathPart::Index(index) => index
                .checked_neg()
                .and_then(|value| value.checked_sub(1))
                .and_then(|value| usize::try_from(value).ok())
                .map(PathElement::IndexFromEnd)
                .ok_or_else(|| polars_err!(InvalidOperation: "path index {index} is too small")),
        })
        .collect()
}

fn render_path(path: &[PathPart]) -> String {
    path.iter()
        .map(PathPart::render)
        .collect::<Vec<_>>()
        .join("/")
}

fn field(name: &str, dtype: SchemaSpec) -> SchemaField {
    SchemaField {
        name: name.to_owned(),
        dtype,
    }
}

fn struct_(fields: Vec<SchemaField>) -> SchemaSpec {
    SchemaSpec::Struct { fields }
}

fn names() -> SchemaSpec {
    struct_(
        ["de", "en", "es", "fr", "ja", "pt-BR", "ru", "zh-CN"]
            .into_iter()
            .map(|name| field(name, SchemaSpec::String))
            .collect(),
    )
}

fn continent() -> SchemaSpec {
    struct_(vec![
        field("code", SchemaSpec::String),
        field("geoname_id", SchemaSpec::UInt32),
        field("names", names()),
    ])
}

fn country() -> SchemaSpec {
    struct_(vec![
        field("geoname_id", SchemaSpec::UInt32),
        field("is_in_european_union", SchemaSpec::Boolean),
        field("iso_code", SchemaSpec::String),
        field("names", names()),
    ])
}

fn represented_country() -> SchemaSpec {
    let SchemaSpec::Struct { mut fields } = country() else {
        unreachable!()
    };
    fields.push(field("type", SchemaSpec::String));
    struct_(fields)
}

fn country_traits() -> SchemaSpec {
    struct_(vec![field("is_anycast", SchemaSpec::Boolean)])
}

fn country_schema() -> SchemaSpec {
    struct_(vec![
        field("continent", continent()),
        field("country", country()),
        field("registered_country", country()),
        field("represented_country", represented_country()),
        field("traits", country_traits()),
    ])
}

fn city_schema() -> SchemaSpec {
    let subdivision = struct_(vec![
        field("geoname_id", SchemaSpec::UInt32),
        field("iso_code", SchemaSpec::String),
        field("names", names()),
    ]);
    struct_(vec![
        field(
            "city",
            struct_(vec![
                field("geoname_id", SchemaSpec::UInt32),
                field("names", names()),
            ]),
        ),
        field("continent", continent()),
        field("country", country()),
        field(
            "location",
            struct_(vec![
                field("accuracy_radius", SchemaSpec::UInt16),
                field("latitude", SchemaSpec::Float64),
                field("longitude", SchemaSpec::Float64),
                field("metro_code", SchemaSpec::UInt16),
                field("time_zone", SchemaSpec::String),
            ]),
        ),
        field("postal", struct_(vec![field("code", SchemaSpec::String)])),
        field("registered_country", country()),
        field("represented_country", represented_country()),
        field(
            "subdivisions",
            SchemaSpec::List {
                inner: Box::new(subdivision),
            },
        ),
        field("traits", country_traits()),
    ])
}

fn asn_schema() -> SchemaSpec {
    struct_(vec![
        field("autonomous_system_number", SchemaSpec::UInt32),
        field("autonomous_system_organization", SchemaSpec::String),
    ])
}

fn enterprise_schema() -> SchemaSpec {
    let enterprise_country = struct_(vec![
        field("confidence", SchemaSpec::UInt8),
        field("geoname_id", SchemaSpec::UInt32),
        field("is_in_european_union", SchemaSpec::Boolean),
        field("iso_code", SchemaSpec::String),
        field("names", names()),
    ]);
    let enterprise_subdivision = struct_(vec![
        field("confidence", SchemaSpec::UInt8),
        field("geoname_id", SchemaSpec::UInt32),
        field("iso_code", SchemaSpec::String),
        field("names", names()),
    ]);
    struct_(vec![
        field(
            "city",
            struct_(vec![
                field("confidence", SchemaSpec::UInt8),
                field("geoname_id", SchemaSpec::UInt32),
                field("names", names()),
            ]),
        ),
        field("continent", continent()),
        field("country", enterprise_country.clone()),
        field(
            "location",
            struct_(vec![
                field("accuracy_radius", SchemaSpec::UInt16),
                field("latitude", SchemaSpec::Float64),
                field("longitude", SchemaSpec::Float64),
                field("metro_code", SchemaSpec::UInt16),
                field("time_zone", SchemaSpec::String),
            ]),
        ),
        field(
            "postal",
            struct_(vec![
                field("code", SchemaSpec::String),
                field("confidence", SchemaSpec::UInt8),
            ]),
        ),
        field("registered_country", enterprise_country),
        field("represented_country", represented_country()),
        field(
            "subdivisions",
            SchemaSpec::List {
                inner: Box::new(enterprise_subdivision),
            },
        ),
        field(
            "traits",
            struct_(vec![
                field("autonomous_system_number", SchemaSpec::UInt32),
                field("autonomous_system_organization", SchemaSpec::String),
                field("connection_type", SchemaSpec::String),
                field("domain", SchemaSpec::String),
                field("is_anonymous", SchemaSpec::Boolean),
                field("is_anonymous_vpn", SchemaSpec::Boolean),
                field("is_anycast", SchemaSpec::Boolean),
                field("is_hosting_provider", SchemaSpec::Boolean),
                field("isp", SchemaSpec::String),
                field("is_public_proxy", SchemaSpec::Boolean),
                field("is_residential_proxy", SchemaSpec::Boolean),
                field("is_tor_exit_node", SchemaSpec::Boolean),
                field("mobile_country_code", SchemaSpec::String),
                field("mobile_network_code", SchemaSpec::String),
                field("organization", SchemaSpec::String),
                field("user_type", SchemaSpec::String),
            ]),
        ),
    ])
}

fn isp_schema() -> SchemaSpec {
    struct_(vec![
        field("autonomous_system_number", SchemaSpec::UInt32),
        field("autonomous_system_organization", SchemaSpec::String),
        field("isp", SchemaSpec::String),
        field("mobile_country_code", SchemaSpec::String),
        field("mobile_network_code", SchemaSpec::String),
        field("organization", SchemaSpec::String),
    ])
}

fn anonymous_ip_schema() -> SchemaSpec {
    struct_(
        [
            "is_anonymous",
            "is_anonymous_vpn",
            "is_hosting_provider",
            "is_public_proxy",
            "is_residential_proxy",
            "is_tor_exit_node",
        ]
        .into_iter()
        .map(|name| field(name, SchemaSpec::Boolean))
        .collect(),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KnownRecord {
    City,
    Country,
    Enterprise,
    Isp,
    ConnectionType,
    AnonymousIp,
    DensityIncome,
    Domain,
    Asn,
}

impl KnownRecord {
    pub fn from_database_type(database_type: &str) -> Option<Self> {
        match database_type {
            "GeoIP2-City" | "GeoLite2-City" | "GeoIP2-City-Shield" => Some(Self::City),
            "GeoIP2-Country" | "GeoLite2-Country" | "GeoIP2-Country-Shield" => Some(Self::Country),
            "GeoIP2-Enterprise"
            | "GeoIP2-Enterprise-Shield"
            | "GeoIP2-Precision-Enterprise"
            | "GeoIP2-Precision-Enterprise-Shield" => Some(Self::Enterprise),
            "GeoIP2-ISP" => Some(Self::Isp),
            "GeoIP2-Connection-Type" => Some(Self::ConnectionType),
            "GeoIP2-Anonymous-IP" => Some(Self::AnonymousIp),
            "GeoIP2-DensityIncome" => Some(Self::DensityIncome),
            "GeoIP2-Domain" => Some(Self::Domain),
            "GeoIP2-ASN" | "GeoLite2-ASN" => Some(Self::Asn),
            _ => None,
        }
    }

    pub fn schema(self) -> SchemaSpec {
        match self {
            Self::City => city_schema(),
            Self::Country => country_schema(),
            Self::Enterprise => enterprise_schema(),
            Self::Isp => isp_schema(),
            Self::ConnectionType => struct_(vec![field("connection_type", SchemaSpec::String)]),
            Self::AnonymousIp => anonymous_ip_schema(),
            Self::DensityIncome => struct_(vec![
                field("average_income", SchemaSpec::UInt32),
                field("population_density", SchemaSpec::UInt32),
            ]),
            Self::Domain => struct_(vec![field("domain", SchemaSpec::String)]),
            Self::Asn => asn_schema(),
        }
    }
}

pub fn known_record(reader: &CachedReader) -> Option<KnownRecord> {
    KnownRecord::from_database_type(&reader.metadata().database_type)
}

pub fn known_schema(database_type: &str) -> Option<SchemaSpec> {
    KnownRecord::from_database_type(database_type).map(KnownRecord::schema)
}

pub fn resolve_record_dtype(
    reader: &CachedReader,
    explicit: Option<&SchemaSpec>,
) -> PolarsResult<(SchemaSpec, Option<KnownRecord>)> {
    let known = known_record(reader);
    match (explicit, known) {
        (None, Some(record)) => Ok((record.schema(), Some(record))),
        (None, None) => {
            let database_type = &reader.metadata().database_type;
            polars_bail!(
                InvalidOperation:
                "whole-record dtype cannot be inferred for MMDB database_type {database_type:?}; pass a Struct dtype explicitly"
            )
        }
        (Some(explicit @ SchemaSpec::Struct { .. }), known) => {
            if let Some(record) = known {
                validate_partial_schema(explicit, &record.schema(), &mut Vec::new())?;
            }
            Ok((explicit.clone(), known))
        }
        (Some(explicit), _) => {
            polars_bail!(
                InvalidOperation:
                "whole-record MMDB lookup requires a Struct dtype, got {}",
                explicit.to_polars()
            )
        }
    }
}

fn validate_partial_schema(
    requested: &SchemaSpec,
    known: &SchemaSpec,
    path: &mut Vec<String>,
) -> PolarsResult<()> {
    match (requested, known) {
        (
            SchemaSpec::Struct {
                fields: requested_fields,
            },
            SchemaSpec::Struct {
                fields: known_fields,
            },
        ) => {
            for requested_field in requested_fields {
                let Some(known_field) = known_fields
                    .iter()
                    .find(|field| field.name == requested_field.name)
                else {
                    polars_bail!(
                        InvalidOperation:
                        "known MMDB schema has no field {:?} at /{}",
                        requested_field.name,
                        path.join("/")
                    )
                };
                path.push(requested_field.name.clone());
                validate_partial_schema(&requested_field.dtype, &known_field.dtype, path)?;
                path.pop();
            }
            Ok(())
        }
        (SchemaSpec::List { inner: requested }, SchemaSpec::List { inner: known }) => {
            validate_partial_schema(requested, known, path)
        }
        (requested, known) if requested == known => Ok(()),
        (requested, known) => {
            polars_bail!(
                InvalidOperation:
                "requested dtype {} does not match known dtype {} at /{}",
                requested.to_polars(),
                known.to_polars(),
                path.join("/")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_supported_metadata_alias() {
        let cases = [
            ("GeoIP2-City", KnownRecord::City),
            ("GeoLite2-City", KnownRecord::City),
            ("GeoIP2-City-Shield", KnownRecord::City),
            ("GeoIP2-Country", KnownRecord::Country),
            ("GeoLite2-Country", KnownRecord::Country),
            ("GeoIP2-Country-Shield", KnownRecord::Country),
            ("GeoIP2-Enterprise", KnownRecord::Enterprise),
            ("GeoIP2-Enterprise-Shield", KnownRecord::Enterprise),
            ("GeoIP2-Precision-Enterprise", KnownRecord::Enterprise),
            (
                "GeoIP2-Precision-Enterprise-Shield",
                KnownRecord::Enterprise,
            ),
            ("GeoIP2-ISP", KnownRecord::Isp),
            ("GeoIP2-Connection-Type", KnownRecord::ConnectionType),
            ("GeoIP2-Anonymous-IP", KnownRecord::AnonymousIp),
            ("GeoIP2-DensityIncome", KnownRecord::DensityIncome),
            ("GeoIP2-Domain", KnownRecord::Domain),
            ("GeoIP2-ASN", KnownRecord::Asn),
            ("GeoLite2-ASN", KnownRecord::Asn),
        ];
        for (database_type, expected) in cases {
            assert_eq!(
                KnownRecord::from_database_type(database_type),
                Some(expected)
            );
        }
    }

    #[test]
    fn schema_spec_round_trips_through_kwargs_json() {
        let dtype = struct_(vec![field(
            "values",
            SchemaSpec::List {
                inner: Box::new(SchemaSpec::UInt32),
            },
        )]);
        let encoded = serde_json::to_string(&dtype).unwrap();
        assert_eq!(serde_json::from_str::<SchemaSpec>(&encoded).unwrap(), dtype);
    }

    #[test]
    fn traverses_structs_and_positive_or_negative_list_indexes() {
        let schema = city_schema();
        for index in [0, -1] {
            let dtype = dtype_at_path(
                schema.clone(),
                &[
                    PathPart::Key("subdivisions".to_owned()),
                    PathPart::Index(index),
                    PathPart::Key("iso_code".to_owned()),
                ],
            )
            .unwrap();
            assert_eq!(dtype, SchemaSpec::String);
        }
    }

    #[test]
    fn rejects_unknown_fields_and_wrong_component_kinds() {
        let error =
            dtype_at_path(city_schema(), &[PathPart::Key("missing".to_owned())]).unwrap_err();
        assert!(error.to_string().contains("has no field"));

        let error = dtype_at_path(city_schema(), &[PathPart::Index(0)]).unwrap_err();
        assert!(error.to_string().contains("cannot apply path component"));
    }

    #[test]
    fn maps_only_exact_known_database_types() {
        assert!(known_schema("GeoIP2-City").is_some());
        assert!(known_schema("GeoIP2-City-Shield").is_some());
        assert!(known_schema("custom-City").is_none());
    }
}

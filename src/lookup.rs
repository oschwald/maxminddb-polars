use std::borrow::Cow;
use std::collections::HashMap;
use std::net::IpAddr;

use maxminddb::LookupResult;
use polars::prelude::*;
use polars_arrow::bitmap::Bitmap;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::cache::{CachedReader, DatabaseIdentity, reader_for};
use crate::known::decode_known;
use crate::schema::{PathPart, SchemaSpec, resolve_path_dtype, resolve_record_dtype, to_mmdb_path};
use crate::value::{Value, decode_projected_path, values_to_series, with_projected_schema};

#[derive(Clone, Debug, Deserialize)]
pub struct LookupPathKwargs {
    pub database: DatabaseIdentity,
    pub path: Vec<PathPart>,
    #[serde(default)]
    pub dtype: Option<SchemaSpec>,
    #[serde(default = "default_strict")]
    pub strict: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LookupRecordKwargs {
    pub database: DatabaseIdentity,
    #[serde(default)]
    pub dtype: Option<SchemaSpec>,
    #[serde(default = "default_strict")]
    pub strict: bool,
}

fn default_strict() -> bool {
    true
}

pub fn output_dtype(kwargs: &LookupPathKwargs) -> PolarsResult<SchemaSpec> {
    let reader = reader_for(&kwargs.database)?;
    resolve_path_dtype(&reader, &kwargs.path, kwargs.dtype.as_ref())
}

pub fn record_output_dtype(kwargs: &LookupRecordKwargs) -> PolarsResult<SchemaSpec> {
    let reader = reader_for(&kwargs.database)?;
    resolve_record_dtype(&reader, kwargs.dtype.as_ref()).map(|(dtype, _)| dtype)
}

pub fn lookup_path_series(inputs: &[Series], kwargs: &LookupPathKwargs) -> PolarsResult<Series> {
    let [input] = inputs else {
        polars_bail!(InvalidOperation: "MMDB lookup expects exactly one input column")
    };
    let ips = input
        .str()
        .map_err(|_| polars_err!(InvalidOperation: "MMDB lookup input must have String dtype"))?;
    let reader = reader_for(&kwargs.database)?;
    let dtype = resolve_path_dtype(&reader, &kwargs.path, kwargs.dtype.as_ref())?;
    let path = to_mmdb_path(&kwargs.path)?;
    let name = input.name().clone();

    macro_rules! primitive {
        ($ty:ty, $chunked:ty) => {{
            let values = decode_scalar_values::<$ty>(ips, &reader, &path, kwargs)?;
            Ok(<$chunked>::from_iter_options(name, values.into_iter()).into_series())
        }};
    }

    match dtype {
        SchemaSpec::Boolean => primitive!(bool, BooleanChunked),
        SchemaSpec::UInt8 => primitive!(u8, UInt8Chunked),
        SchemaSpec::UInt16 => primitive!(u16, UInt16Chunked),
        SchemaSpec::UInt32 => primitive!(u32, UInt32Chunked),
        SchemaSpec::UInt64 => primitive!(u64, UInt64Chunked),
        SchemaSpec::UInt128 => primitive!(u128, UInt128Chunked),
        SchemaSpec::Int8 => primitive!(i8, Int8Chunked),
        SchemaSpec::Int16 => primitive!(i16, Int16Chunked),
        SchemaSpec::Int32 => primitive!(i32, Int32Chunked),
        SchemaSpec::Int64 => primitive!(i64, Int64Chunked),
        SchemaSpec::Int128 => primitive!(i128, Int128Chunked),
        SchemaSpec::Float32 => primitive!(f32, Float32Chunked),
        SchemaSpec::Float64 => primitive!(f64, Float64Chunked),
        SchemaSpec::String => primitive!(String, StringChunked),
        SchemaSpec::Binary => primitive!(Vec<u8>, BinaryChunked),
        dtype => decode_nested_path_values(ips, &reader, &path, &dtype, kwargs),
    }
}

pub fn lookup_record_series(
    inputs: &[Series],
    kwargs: &LookupRecordKwargs,
) -> PolarsResult<Series> {
    let [input] = inputs else {
        polars_bail!(InvalidOperation: "MMDB lookup expects exactly one input column")
    };
    let ips = input
        .str()
        .map_err(|_| polars_err!(InvalidOperation: "MMDB lookup input must have String dtype"))?;
    let reader = reader_for(&kwargs.database)?;
    let (dtype, known_record) = resolve_record_dtype(&reader, kwargs.dtype.as_ref())?;
    let direct_record = known_record.filter(|record| dtype == record.schema());
    let batch = lookup_batch(ips, &reader, &kwargs.database, kwargs.strict)?;
    let Some(record) = direct_record else {
        return decode_projection_series(
            input.name().clone(),
            &batch.unique,
            &batch.rows,
            &dtype,
            &kwargs.database,
        );
    };
    let unique_values = batch
        .unique
        .iter()
        .map(|result| decode_known(result, record))
        .collect::<PolarsResult<Vec<_>>>()?;
    let values = gather_values(batch.rows, &unique_values);
    values_to_series(input.name().clone(), &dtype, values)
}

pub(crate) struct LookupBatch<'a> {
    pub(crate) unique: Vec<LookupResult<'a, Vec<u8>>>,
    pub(crate) rows: Vec<Option<usize>>,
}

pub(crate) fn lookup_batch<'a>(
    ips: &StringChunked,
    reader: &'a CachedReader,
    database: &DatabaseIdentity,
    strict: bool,
) -> PolarsResult<LookupBatch<'a>> {
    let mut offsets = HashMap::<usize, usize>::new();
    let mut unique = Vec::new();
    let mut rows = Vec::with_capacity(ips.len());

    for value in ips.iter() {
        let Some(value) = value else {
            rows.push(None);
            continue;
        };
        let ip = match value.parse::<IpAddr>() {
            Ok(ip) => ip,
            Err(_) if !strict => {
                rows.push(None);
                continue;
            }
            Err(error) => {
                polars_bail!(
                    ComputeError:
                    "invalid IP address {value:?} for MMDB {:?}: {error}",
                    database.canonical_path
                )
            }
        };
        let result = reader.lookup(ip).map_err(|error| {
            polars_err!(
                ComputeError:
                "MMDB lookup failed for {ip} in {:?}: {error}",
                database.canonical_path
            )
        })?;
        let Some(offset) = result.offset() else {
            rows.push(None);
            continue;
        };
        let unique_index = match offsets.get(&offset) {
            Some(index) => *index,
            None => {
                let index = unique.len();
                offsets.insert(offset, index);
                unique.push(result);
                index
            }
        };
        rows.push(Some(unique_index));
    }

    Ok(LookupBatch { unique, rows })
}

fn decode_scalar_values<T>(
    ips: &StringChunked,
    reader: &CachedReader,
    path: &[maxminddb::PathElement<'_>],
    kwargs: &LookupPathKwargs,
) -> PolarsResult<Vec<Option<T>>>
where
    T: DeserializeOwned + Clone,
{
    let batch = lookup_batch(ips, reader, &kwargs.database, kwargs.strict)?;
    let unique_values = batch
        .unique
        .iter()
        .map(|result| {
            result.decode_path(path).map_err(|error| {
                polars_err!(
                    ComputeError:
                    "could not decode MMDB value at path in {:?}: {error}",
                    kwargs.database.canonical_path
                )
            })
        })
        .collect::<PolarsResult<Vec<Option<T>>>>()?;

    Ok(batch
        .rows
        .into_iter()
        .map(|row| row.and_then(|index| unique_values[index].clone()))
        .collect())
}

fn decode_nested_path_values(
    ips: &StringChunked,
    reader: &CachedReader,
    path: &[maxminddb::PathElement<'_>],
    dtype: &SchemaSpec,
    kwargs: &LookupPathKwargs,
) -> PolarsResult<Series> {
    let batch = lookup_batch(ips, reader, &kwargs.database, kwargs.strict)?;
    let unique_values = with_projected_schema(dtype, || {
        batch
            .unique
            .iter()
            .map(|result| {
                decode_projected_path(result, path).map_err(|error| {
                    polars_err!(
                        ComputeError:
                        "could not decode MMDB value at path in {:?}: {error}",
                        kwargs.database.canonical_path
                    )
                })
            })
            .collect::<PolarsResult<Vec<_>>>()
    })?;
    let values = gather_values(batch.rows, &unique_values);
    values_to_series(ips.name().clone(), dtype, values)
}

struct ProjectionLeaf<'a> {
    path: Vec<&'a str>,
    dtype: &'a SchemaSpec,
}

fn decode_projection_series<'a>(
    name: PlSmallStr,
    results: &[LookupResult<'a, Vec<u8>>],
    rows: &[Option<usize>],
    dtype: &SchemaSpec,
    database: &DatabaseIdentity,
) -> PolarsResult<Series> {
    let mut leaves = Vec::new();
    flatten_projection(dtype, &mut Vec::new(), &mut leaves);
    let indices = IdxCa::from_iter_options(
        PlSmallStr::EMPTY,
        rows.iter().map(|row| row.map(|index| index as IdxSize)),
    );
    let validity = Bitmap::from_iter(rows.iter().map(Option::is_some));
    let mut leaf_series = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        let path = leaf
            .path
            .iter()
            .map(|name| maxminddb::PathElement::Key(name))
            .collect::<Vec<_>>();
        let leaf_values = with_projected_schema(leaf.dtype, || {
            results
                .iter()
                .map(|result| decode_leaf(result, &path, leaf.dtype))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| {
            polars_err!(
                ComputeError:
                "could not decode MMDB record in {:?}: {error}",
                database.canonical_path
            )
        })?;
        let values = leaf_values
            .into_iter()
            .map(|value| Some(value.unwrap_or_else(|| crate::value::default_value(leaf.dtype))))
            .collect();
        let unique = values_to_series(PlSmallStr::EMPTY, leaf.dtype, values)?;
        leaf_series.push(unique.take(&indices)?);
    }
    assemble_projection_series(
        name,
        dtype,
        rows.len(),
        &validity,
        &mut leaf_series.into_iter(),
    )
}

fn flatten_projection<'a>(
    dtype: &'a SchemaSpec,
    path: &mut Vec<&'a str>,
    leaves: &mut Vec<ProjectionLeaf<'a>>,
) {
    if let SchemaSpec::Struct { fields } = dtype {
        for field in fields {
            path.push(&field.name);
            flatten_projection(&field.dtype, path, leaves);
            path.pop();
        }
    } else {
        leaves.push(ProjectionLeaf {
            path: path.clone(),
            dtype,
        });
    }
}

fn decode_leaf<'a>(
    result: &LookupResult<'a, Vec<u8>>,
    path: &[maxminddb::PathElement<'_>],
    dtype: &SchemaSpec,
) -> Result<Option<Value<'a>>, maxminddb::MaxMindDbError> {
    macro_rules! scalar {
        ($ty:ty, $variant:path) => {
            result
                .decode_path::<$ty>(path)
                .map(|value| value.map($variant))
        };
    }

    match dtype {
        SchemaSpec::Boolean => scalar!(bool, Value::Boolean),
        SchemaSpec::UInt8 => scalar!(u8, Value::UInt8),
        SchemaSpec::UInt16 => scalar!(u16, Value::UInt16),
        SchemaSpec::UInt32 => scalar!(u32, Value::UInt32),
        SchemaSpec::UInt64 => scalar!(u64, Value::UInt64),
        SchemaSpec::UInt128 => scalar!(u128, Value::UInt128),
        SchemaSpec::Int8 => scalar!(i8, Value::Int8),
        SchemaSpec::Int16 => scalar!(i16, Value::Int16),
        SchemaSpec::Int32 => scalar!(i32, Value::Int32),
        SchemaSpec::Int64 => scalar!(i64, Value::Int64),
        SchemaSpec::Int128 => scalar!(i128, Value::Int128),
        SchemaSpec::Float32 => scalar!(f32, Value::Float32),
        SchemaSpec::Float64 => scalar!(f64, Value::Float64),
        SchemaSpec::String => result
            .decode_path::<&'a str>(path)
            .map(|value| value.map(|value| Value::String(Cow::Borrowed(value)))),
        SchemaSpec::Binary => result
            .decode_path::<&'a [u8]>(path)
            .map(|value| value.map(|value| Value::Binary(Cow::Borrowed(value)))),
        SchemaSpec::List { .. } => decode_projected_path(result, path),
        SchemaSpec::Struct { .. } => unreachable!("Struct projections are flattened"),
    }
}

fn assemble_projection_series(
    name: PlSmallStr,
    dtype: &SchemaSpec,
    length: usize,
    validity: &Bitmap,
    leaves: &mut impl Iterator<Item = Series>,
) -> PolarsResult<Series> {
    let SchemaSpec::Struct { fields } = dtype else {
        let mut series = leaves.next().expect("each projection leaf has a series");
        series.rename(name);
        return Ok(series);
    };
    let children = fields
        .iter()
        .map(|field| {
            assemble_projection_series(
                field.name.clone().into(),
                &field.dtype,
                length,
                validity,
                leaves,
            )
        })
        .collect::<PolarsResult<Vec<_>>>()?;
    Ok(StructChunked::from_series(name, length, children.iter())?
        .with_outer_validity(Some(validity.clone()))
        .into_series())
}

fn gather_values<'a>(
    rows: Vec<Option<usize>>,
    unique_values: &'a [Option<Value<'a>>],
) -> Vec<Option<Value<'a>>> {
    rows.into_iter()
        .map(|row| row.and_then(|index| unique_values[index].clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::UNIX_EPOCH;

    use super::*;

    const CITY_DB: &str = "tests/data/test-data/GeoIP2-City-Test.mmdb";

    fn identity(path: &str) -> DatabaseIdentity {
        let path = Path::new(path).canonicalize().unwrap();
        let metadata = fs::metadata(&path).unwrap();
        DatabaseIdentity {
            canonical_path: path.to_string_lossy().into_owned(),
            size: metadata.len(),
            modified_ns: metadata
                .modified()
                .unwrap()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                .try_into()
                .unwrap(),
        }
    }

    fn kwargs(strict: bool) -> LookupPathKwargs {
        LookupPathKwargs {
            database: identity(CITY_DB),
            path: vec![
                PathPart::Key("country".to_owned()),
                PathPart::Key("iso_code".to_owned()),
            ],
            dtype: None,
            strict,
        }
    }

    fn series(values: &[Option<&str>]) -> Series {
        StringChunked::from_iter_options("ip".into(), values.iter().copied()).into_series()
    }

    #[test]
    fn infers_and_decodes_a_known_scalar_path() {
        let output = lookup_path_series(
            &[series(&[Some("89.160.20.128"), None, Some("203.0.113.1")])],
            &kwargs(true),
        )
        .unwrap();
        assert_eq!(output.dtype(), &DataType::String);
        assert_eq!(
            output.str().unwrap().iter().collect::<Vec<_>>(),
            vec![Some("SE"), None, None]
        );
    }

    #[test]
    fn strict_invalid_ips_error_and_non_strict_ips_become_null() {
        let input = series(&[Some("not-an-ip")]);
        let error = lookup_path_series(std::slice::from_ref(&input), &kwargs(true)).unwrap_err();
        assert!(error.to_string().contains("invalid IP address"));

        let output = lookup_path_series(&[input], &kwargs(false)).unwrap();
        assert_eq!(output.null_count(), 1);
    }

    #[test]
    fn deduplicates_records_before_decoding() {
        let kwargs = kwargs(true);
        let reader = reader_for(&kwargs.database).unwrap();
        let ips = series(&[Some("89.160.20.128"), Some("89.160.20.129")]);
        let batch =
            lookup_batch(ips.str().unwrap(), &reader, &kwargs.database, kwargs.strict).unwrap();
        assert_eq!(batch.unique.len(), 1);
        assert_eq!(batch.rows, vec![Some(0), Some(0)]);
    }
}

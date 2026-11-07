use std::borrow::Cow;
use std::net::IpAddr;

use maxminddb::LookupResult;
use maxminddb::geoip2::MAX_SUBDIVISIONS;
use polars::prelude::*;
use polars_arrow::array::Utf8ViewArray;
use polars_arrow::bitmap::Bitmap;
use polars_core::runtime::RAYON;
use polars_utils::aliases::{InitHashMaps, PlHashMap};
use rayon::prelude::*;
use serde::Deserialize;

#[cfg(test)]
use crate::cache::identity_for_path;
use crate::cache::{CachedReader, DatabaseIdentity, reader_for};
use crate::guard::catch_mmdb_unwind;
use crate::known::decode_known;
use crate::schema::{
    KnownRecord, PathPart, SchemaSpec, resolve_path_dtype, resolve_record_dtype, to_mmdb_path,
};
use crate::value::{
    Double, Integer, Value, decode_projected_path, values_to_series, with_projected_schema,
};

// Smaller batches do not repay Rayon scheduling and multi-chunk output costs.
const PARALLEL_SCALAR_MIN_ROWS: usize = 8_192;
// Bound each task's temporary decoded values independently of the batch size.
const PARALLEL_SCALAR_MAX_CHUNK_ROWS: usize = 2_048;

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
    lookup_path_series_with_workers(inputs, kwargs, RAYON.current_num_threads())
}

fn lookup_path_series_with_workers(
    inputs: &[Series],
    kwargs: &LookupPathKwargs,
    scalar_workers: usize,
) -> PolarsResult<Series> {
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
        ($ty:ty, $chunked:ty) => {
            primitive!($ty, $chunked, std::convert::identity)
        };
        ($ty:ty, $chunked:ty, $native:expr) => {{
            if ips.len() >= PARALLEL_SCALAR_MIN_ROWS && scalar_workers > 2 {
                let chunk_name = name.clone();
                let chunks = decode_scalar_chunks_parallel::<$ty, _, _>(
                    ips,
                    &reader,
                    &path,
                    kwargs,
                    move |values| {
                        <$chunked>::from_iter_options(
                            chunk_name.clone(),
                            values.into_iter().map(|value| value.map($native)),
                        )
                    },
                )?;
                let mut chunks = chunks.into_iter();
                let mut output = chunks.next().ok_or_else(|| {
                    polars_err!(ComputeError: "parallel MMDB lookup returned no output chunks")
                })?;
                for chunk in chunks {
                    output.append(&chunk)?;
                }
                Ok(output.into_series())
            } else {
                let (unique_values, rows) =
                    decode_scalar_values::<$ty>(ips, &reader, &path, kwargs)?;
                let values = rows
                    .into_iter()
                    .map(|row| row.and_then(|index| unique_values[index]).map($native));
                Ok(<$chunked>::from_iter_options(name, values).into_series())
            }
        }};
    }

    macro_rules! integer {
        ($ty:ty, $chunked:ty) => {
            primitive!(Integer<$ty>, $chunked, |value: Integer<$ty>| value.0)
        };
    }

    match dtype {
        SchemaSpec::Boolean => primitive!(bool, BooleanChunked),
        SchemaSpec::UInt8 => integer!(u8, UInt8Chunked),
        SchemaSpec::UInt16 => integer!(u16, UInt16Chunked),
        SchemaSpec::UInt32 => integer!(u32, UInt32Chunked),
        SchemaSpec::UInt64 => integer!(u64, UInt64Chunked),
        SchemaSpec::UInt128 => integer!(u128, UInt128Chunked),
        SchemaSpec::Int8 => integer!(i8, Int8Chunked),
        SchemaSpec::Int16 => integer!(i16, Int16Chunked),
        SchemaSpec::Int32 => integer!(i32, Int32Chunked),
        SchemaSpec::Int64 => integer!(i64, Int64Chunked),
        SchemaSpec::Int128 => integer!(i128, Int128Chunked),
        SchemaSpec::Float32 => primitive!(f32, Float32Chunked),
        SchemaSpec::Float64 => primitive!(Double, Float64Chunked, |value: Double| value.0),
        SchemaSpec::String => primitive!(&str, StringChunked),
        SchemaSpec::Binary => primitive!(&[u8], BinaryChunked),
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
    let unique_values = guard_mmdb_operation(&kwargs.database, || {
        // decode_known_record retries with the projected decoder, which reads
        // this schema.
        with_projected_schema(&dtype, || {
            batch
                .unique
                .iter()
                .map(|result| decode_known_record(result, record))
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| {
            polars_err!(
                ComputeError:
                "could not decode {} record in {:?}: {error}",
                record.name(),
                kwargs.database.canonical_path
            )
        })
    })?;
    decoded_values_to_series(input.name().clone(), &unique_values, &batch.rows, &dtype)
}

pub(crate) struct LookupBatch<'a> {
    pub(crate) unique: Vec<LookupResult<'a, Vec<u8>>>,
    pub(crate) rows: Vec<Option<usize>>,
}

type DecodedScalarValues<T> = (Vec<Option<T>>, Vec<Option<usize>>);

#[derive(Clone, Copy)]
struct ScalarInputSlice<'a> {
    array: &'a Utf8ViewArray,
    start: usize,
    end: usize,
}

impl ScalarInputSlice<'_> {
    fn len(self) -> usize {
        self.end - self.start
    }
}

fn scalar_input_tasks(ips: &StringChunked) -> Vec<Vec<ScalarInputSlice<'_>>> {
    let mut tasks = Vec::with_capacity(ips.len().div_ceil(PARALLEL_SCALAR_MAX_CHUNK_ROWS));
    let mut task = Vec::new();
    let mut task_rows = 0;

    for array in ips.downcast_iter() {
        let mut start = 0;
        while start < array.len() {
            let rows = (PARALLEL_SCALAR_MAX_CHUNK_ROWS - task_rows).min(array.len() - start);
            let end = start + rows;
            task.push(ScalarInputSlice { array, start, end });
            task_rows += rows;
            start = end;

            if task_rows == PARALLEL_SCALAR_MAX_CHUNK_ROWS {
                tasks.push(std::mem::take(&mut task));
                task_rows = 0;
            }
        }
    }
    if !task.is_empty() {
        tasks.push(task);
    }
    tasks
}

pub(crate) fn lookup_batch<'a>(
    ips: &StringChunked,
    reader: &'a CachedReader,
    database: &DatabaseIdentity,
    strict: bool,
) -> PolarsResult<LookupBatch<'a>> {
    guard_mmdb_operation(database, || {
        lookup_batch_inner(ips, reader, database, strict)
    })
}

fn lookup_batch_inner<'a>(
    ips: &StringChunked,
    reader: &'a CachedReader,
    database: &DatabaseIdentity,
    strict: bool,
) -> PolarsResult<LookupBatch<'a>> {
    let mut offsets = PlHashMap::<usize, usize>::new();
    let mut unique = Vec::new();
    let mut rows = Vec::with_capacity(ips.len());

    for value in ips.iter() {
        let Some(result) = lookup_one(value, reader, database, strict)? else {
            rows.push(None);
            continue;
        };
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

fn lookup_one<'a>(
    value: Option<&str>,
    reader: &'a CachedReader,
    database: &DatabaseIdentity,
    strict: bool,
) -> PolarsResult<Option<LookupResult<'a, Vec<u8>>>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let ip = match value.parse::<IpAddr>() {
        Ok(ip) => ip,
        Err(_) if !strict => return Ok(None),
        Err(error) => {
            polars_bail!(
                ComputeError:
                "invalid IP address {value:?} for MMDB {:?}: {error}",
                database.canonical_path
            )
        }
    };
    reader.lookup(ip).map(Some).map_err(|error| {
        polars_err!(
            ComputeError:
            "MMDB lookup failed for {ip} in {:?}: {error}",
            database.canonical_path
        )
    })
}

fn decode_scalar_values<'a, T>(
    ips: &StringChunked,
    reader: &'a CachedReader,
    path: &[maxminddb::PathElement<'_>],
    kwargs: &LookupPathKwargs,
) -> PolarsResult<DecodedScalarValues<T>>
where
    T: Deserialize<'a> + Copy,
{
    let batch = lookup_batch(ips, reader, &kwargs.database, kwargs.strict)?;
    let unique_values = guard_mmdb_operation(&kwargs.database, || {
        batch
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
            .collect::<PolarsResult<Vec<Option<T>>>>()
    })?;

    Ok((unique_values, batch.rows))
}

fn decode_scalar_chunks_parallel<'a, T, C, F>(
    ips: &StringChunked,
    reader: &'a CachedReader,
    path: &[maxminddb::PathElement<'_>],
    kwargs: &LookupPathKwargs,
    build: F,
) -> PolarsResult<Vec<C>>
where
    T: Deserialize<'a> + Copy + Send,
    C: Send,
    F: Fn(Vec<Option<T>>) -> C + Send + Sync,
{
    if ips.is_empty() {
        return Ok(Vec::new());
    }
    let tasks = scalar_input_tasks(ips);
    guard_mmdb_operation(&kwargs.database, || {
        RAYON.install(|| {
            tasks
                .into_par_iter()
                .map(|task| {
                    let task_rows = task.iter().copied().map(ScalarInputSlice::len).sum();
                    let mut values = Vec::with_capacity(task_rows);
                    for slice in task {
                        for index in slice.start..slice.end {
                            let Some(result) = lookup_one(
                                slice.array.get(index),
                                reader,
                                &kwargs.database,
                                kwargs.strict,
                            )?
                            else {
                                values.push(None);
                                continue;
                            };
                            values.push(result.decode_path(path).map_err(|error| {
                                polars_err!(
                                    ComputeError:
                                    "could not decode MMDB value at path in {:?}: {error}",
                                    kwargs.database.canonical_path
                                )
                            })?);
                        }
                    }
                    Ok(build(values))
                })
                .collect()
        })
    })
}

fn decode_nested_path_values(
    ips: &StringChunked,
    reader: &CachedReader,
    path: &[maxminddb::PathElement<'_>],
    dtype: &SchemaSpec,
    kwargs: &LookupPathKwargs,
) -> PolarsResult<Series> {
    let batch = lookup_batch(ips, reader, &kwargs.database, kwargs.strict)?;
    let unique_values = guard_mmdb_operation(&kwargs.database, || {
        with_projected_schema(dtype, || {
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
        })
    })?;
    decoded_values_to_series(ips.name().clone(), &unique_values, &batch.rows, dtype)
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
        let leaf_values = guard_mmdb_operation(database, || {
            with_projected_schema(leaf.dtype, || {
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
            })
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
        ($ty:ty, $convert:expr) => {
            result
                .decode_path::<$ty>(path)
                .map(|value| value.map($convert))
        };
    }

    macro_rules! integer {
        ($ty:ty, $variant:path) => {
            scalar!(Integer<$ty>, |value: Integer<$ty>| $variant(value.0))
        };
    }

    match dtype {
        SchemaSpec::Boolean => scalar!(bool, Value::Boolean),
        SchemaSpec::UInt8 => integer!(u8, Value::UInt8),
        SchemaSpec::UInt16 => integer!(u16, Value::UInt16),
        SchemaSpec::UInt32 => integer!(u32, Value::UInt32),
        SchemaSpec::UInt64 => integer!(u64, Value::UInt64),
        SchemaSpec::UInt128 => integer!(u128, Value::UInt128),
        SchemaSpec::Int8 => integer!(i8, Value::Int8),
        SchemaSpec::Int16 => integer!(i16, Value::Int16),
        SchemaSpec::Int32 => integer!(i32, Value::Int32),
        SchemaSpec::Int64 => integer!(i64, Value::Int64),
        SchemaSpec::Int128 => integer!(i128, Value::Int128),
        SchemaSpec::Float32 => scalar!(f32, Value::Float32),
        SchemaSpec::Float64 => scalar!(Double, |value: Double| Value::Float64(value.0)),
        SchemaSpec::String => scalar!(&'a str, |value| Value::String(Cow::Borrowed(value))),
        SchemaSpec::Binary => scalar!(&'a [u8], |value| Value::Binary(Cow::Borrowed(value))),
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

fn decode_known_record<'a>(
    result: &LookupResult<'a, Vec<u8>>,
    record: KnownRecord,
) -> Result<Option<Value<'a>>, maxminddb::MaxMindDbError> {
    // The typed record structs require the standard encoded types. A writer
    // can pick another type that fits, such as uint16 for an ASN, so retry
    // decoding errors with the projected decoder. Its error replaces the typed
    // one because it applies the rules that custom dtypes use. Other errors,
    // such as an invalid database or a resource limit, are returned as is.
    match decode_known(result, record) {
        Err(maxminddb::MaxMindDbError::Decoding { .. }) => decode_projected_record(result, record),
        decoded => decoded,
    }
}

/// Decodes a known record with the projected decoder.
///
/// The typed City and Enterprise structs reject more than `MAX_SUBDIVISIONS`
/// subdivisions with a decoding error. Apply the same limit here, so a retry
/// does not accept a record that the typed decode rejected. Check it before
/// the projected decode, which would otherwise build every subdivision first.
fn decode_projected_record<'a>(
    result: &LookupResult<'a, Vec<u8>>,
    record: KnownRecord,
) -> Result<Option<Value<'a>>, maxminddb::MaxMindDbError> {
    if matches!(record, KnownRecord::City | KnownRecord::Enterprise) {
        check_subdivision_count(result)?;
    }
    decode_projected_path(result, &[])
}

fn check_subdivision_count(
    result: &LookupResult<'_, Vec<u8>>,
) -> Result<(), maxminddb::MaxMindDbError> {
    match result.decode_path::<SubdivisionCount>(&[maxminddb::PathElement::Key("subdivisions")]) {
        Ok(Some(SubdivisionCount(count))) if count > MAX_SUBDIVISIONS => {
            Err(maxminddb::MaxMindDbError::decoding(format!(
                "subdivisions exceeds maximum length of {MAX_SUBDIVISIONS}"
            )))
        }
        // The projected decode reports a subdivisions value that is not an
        // array, with the same error that custom dtypes get.
        Ok(_) | Err(maxminddb::MaxMindDbError::Decoding { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}

/// The length of an MMDB array, up to one more than `MAX_SUBDIVISIONS`.
struct SubdivisionCount(usize);

impl<'de> Deserialize<'de> for SubdivisionCount {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_seq(SubdivisionCountVisitor)
    }
}

struct SubdivisionCountVisitor;

impl<'de> serde::de::Visitor<'de> for SubdivisionCountVisitor {
    type Value = SubdivisionCount;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("an MMDB array")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        if let Some(count) = sequence.size_hint() {
            return Ok(SubdivisionCount(count));
        }
        let mut count = 0;
        while count <= MAX_SUBDIVISIONS
            && sequence.next_element::<serde::de::IgnoredAny>()?.is_some()
        {
            count += 1;
        }
        Ok(SubdivisionCount(count))
    }
}

fn decoded_values_to_series(
    name: PlSmallStr,
    unique_values: &[Option<Value<'_>>],
    rows: &[Option<usize>],
    dtype: &SchemaSpec,
) -> PolarsResult<Series> {
    let mut leaves = Vec::new();
    flatten_projection(dtype, &mut Vec::new(), &mut leaves);
    let indices = IdxCa::from_iter_options(
        PlSmallStr::EMPTY,
        rows.iter().map(|row| row.map(|index| index as IdxSize)),
    );
    let validity = Bitmap::from_iter(rows.iter().map(|row| {
        row.and_then(|index| unique_values[index].as_ref())
            .is_some()
    }));
    let mut leaf_series = Vec::with_capacity(leaves.len());
    for leaf in leaves {
        let values = unique_values
            .iter()
            .map(|record| {
                Some(
                    record
                        .as_ref()
                        .and_then(|record| value_at_path(record, &leaf.path))
                        .cloned()
                        .unwrap_or_else(|| crate::value::default_value(leaf.dtype)),
                )
            })
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

fn value_at_path<'a>(mut value: &'a Value<'a>, path: &[&str]) -> Option<&'a Value<'a>> {
    for name in path {
        let Value::Map(fields) = value else {
            return None;
        };
        value = fields
            .iter()
            .find(|(field_name, _)| field_name.as_ref() == *name)
            .map(|(_, value)| value)?;
    }
    Some(value)
}

fn guard_mmdb_operation<T>(
    database: &DatabaseIdentity,
    operation: impl FnOnce() -> PolarsResult<T>,
) -> PolarsResult<T> {
    catch_mmdb_unwind(operation).map_err(|()| {
        polars_err!(
            ComputeError:
            "MMDB parser panicked while reading {:?}; the database may be corrupt",
            database.canonical_path
        )
    })?
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::fs;
    use std::net::{IpAddr, Ipv4Addr};
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::Path;

    use base64::Engine;
    use maxminddb::Reader;
    use proptest::prelude::*;

    use super::*;

    const CITY_DB: &str = "tests/data/test-data/GeoIP2-City-Test.mmdb";

    fn identity(path: &str) -> DatabaseIdentity {
        let path = Path::new(path).canonicalize().unwrap();
        identity_for_path(&path).unwrap()
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

    // A batch can mix typed and projected records, and values_to_series
    // accepts only the exact Value variant of each dtype. Both decoders must
    // give the same Value for every record.
    #[test]
    fn typed_and_projected_decoders_agree_on_known_fixtures() {
        let mut checked = std::collections::BTreeSet::new();
        for entry in fs::read_dir("tests/data/test-data").unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap();
            let standard = name.starts_with("GeoIP2-") || name.starts_with("GeoLite2-");
            if !standard || name.contains("Broken") || name.contains("Invalid") {
                continue;
            }
            let reader = Reader::open_readfile(&path).unwrap();
            let Some(record) = KnownRecord::from_database_type(&reader.metadata().database_type)
            else {
                continue;
            };
            let schema = record.schema();
            for network in reader.networks(Default::default()).unwrap() {
                let result = network.unwrap();
                let typed = decode_known(&result, record).unwrap();
                let projected =
                    with_projected_schema(&schema, || decode_projected_path(&result, &[])).unwrap();
                assert_eq!(typed, projected, "{name}");
                checked.insert(record.name());
            }
        }
        // One fixture for each KnownRecord.
        assert_eq!(checked.len(), 9, "{checked:?}");
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

    #[test]
    fn parallel_scalar_chunks_match_deduplicated_lookup() {
        let kwargs = kwargs(false);
        let reader = reader_for(&kwargs.database).unwrap();
        let path = to_mmdb_path(&kwargs.path).unwrap();
        let candidates = [
            Some("89.160.20.128"),
            None,
            Some("not-an-ip"),
            Some("203.0.113.1"),
        ];
        let inputs = (0..8_200)
            .map(|index| candidates[index % candidates.len()])
            .collect::<Vec<_>>();
        let input = series(&inputs);
        let ips = input.str().unwrap();

        let parallel_chunks =
            decode_scalar_chunks_parallel::<&str, _, _>(ips, &reader, &path, &kwargs, |values| {
                values
            })
            .unwrap();
        assert_eq!(
            parallel_chunks.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![2_048, 2_048, 2_048, 2_048, 8]
        );
        assert!(
            parallel_chunks
                .iter()
                .all(|values| values.len() <= PARALLEL_SCALAR_MAX_CHUNK_ROWS)
        );
        let parallel = parallel_chunks.into_iter().flatten().collect::<Vec<_>>();
        let (unique, rows) = decode_scalar_values::<&str>(ips, &reader, &path, &kwargs).unwrap();
        let deduplicated = rows
            .into_iter()
            .map(|row| row.and_then(|index| unique[index]))
            .collect::<Vec<_>>();

        assert_eq!(parallel, deduplicated);
    }

    #[test]
    fn scalar_tasks_stay_bounded_for_large_batches() {
        let row_count = 1_000_001;
        let values = vec![Some("89.160.20.128"); row_count];
        let input = series(&values);
        let tasks = scalar_input_tasks(input.str().unwrap());
        let lengths = tasks
            .iter()
            .map(|task| {
                task.iter()
                    .copied()
                    .map(ScalarInputSlice::len)
                    .sum::<usize>()
            })
            .collect::<Vec<_>>();

        assert_eq!(
            tasks.len(),
            row_count.div_ceil(PARALLEL_SCALAR_MAX_CHUNK_ROWS)
        );
        assert_eq!(lengths.iter().sum::<usize>(), row_count);
        assert!(
            lengths
                .iter()
                .all(|length| *length <= PARALLEL_SCALAR_MAX_CHUNK_ROWS)
        );
    }

    #[test]
    fn scalar_tasks_coalesce_fragmented_inputs() {
        let mut input = series(&[Some("89.160.20.128")]);
        for _ in 1..PARALLEL_SCALAR_MIN_ROWS {
            input.append(&series(&[Some("89.160.20.128")])).unwrap();
        }
        assert_eq!(input.n_chunks(), PARALLEL_SCALAR_MIN_ROWS);

        let tasks = scalar_input_tasks(input.str().unwrap());
        assert_eq!(
            tasks.len(),
            PARALLEL_SCALAR_MIN_ROWS.div_ceil(PARALLEL_SCALAR_MAX_CHUNK_ROWS)
        );
        assert!(tasks.iter().all(|task| {
            task.iter()
                .copied()
                .map(ScalarInputSlice::len)
                .sum::<usize>()
                == PARALLEL_SCALAR_MAX_CHUNK_ROWS
        }));
    }

    #[test]
    fn scalar_dispatch_respects_row_and_worker_thresholds() {
        let kwargs = kwargs(false);
        let candidates = [
            Some("89.160.20.128"),
            None,
            Some("not-an-ip"),
            Some("203.0.113.1"),
        ];
        let inputs = (0..PARALLEL_SCALAR_MIN_ROWS)
            .map(|index| candidates[index % candidates.len()])
            .collect::<Vec<_>>();
        let input = series(&inputs);

        let parallel =
            lookup_path_series_with_workers(std::slice::from_ref(&input), &kwargs, 3).unwrap();
        let two_workers =
            lookup_path_series_with_workers(std::slice::from_ref(&input), &kwargs, 2).unwrap();
        let below_threshold = lookup_path_series_with_workers(
            &[input.slice(0, PARALLEL_SCALAR_MIN_ROWS - 1)],
            &kwargs,
            3,
        )
        .unwrap();

        assert!(parallel.equals_missing(&two_workers));
        assert_eq!(parallel.n_chunks(), 4);
        assert_eq!(two_workers.n_chunks(), 1);
        assert_eq!(below_threshold.n_chunks(), 1);
    }

    #[test]
    fn unsigned_conversion_preserves_scalar_dispatch_thresholds() {
        let mut kwargs = LookupPathKwargs {
            database: identity("tests/data/test-data/MaxMind-DB-test-decoder.mmdb"),
            path: vec![PathPart::Key("array".to_owned()), PathPart::Index(0)],
            dtype: Some(SchemaSpec::UInt16),
            strict: false,
        };
        let candidates = [
            Some("::1.1.1.0"),
            None,
            Some("not-an-ip"),
            Some("2001:db8::1"),
        ];
        let values = (0..PARALLEL_SCALAR_MIN_ROWS + 1)
            .map(|index| candidates[index % candidates.len()])
            .collect::<Vec<_>>();
        let input = series(&values);
        let parallel =
            lookup_path_series_with_workers(std::slice::from_ref(&input), &kwargs, 3).unwrap();
        let serial =
            lookup_path_series_with_workers(std::slice::from_ref(&input), &kwargs, 2).unwrap();
        let below = lookup_path_series_with_workers(
            &[input.slice(0, PARALLEL_SCALAR_MIN_ROWS - 1)],
            &kwargs,
            3,
        )
        .unwrap();
        assert!(parallel.equals_missing(&serial));
        assert_eq!(parallel.u16().unwrap().get(0), Some(1));
        assert_eq!(
            parallel.n_chunks(),
            (PARALLEL_SCALAR_MIN_ROWS + 1).div_ceil(PARALLEL_SCALAR_MAX_CHUNK_ROWS)
        );
        assert_eq!(serial.n_chunks(), 1);
        assert_eq!(below.n_chunks(), 1);
        kwargs.path = vec![PathPart::Key("uint32".to_owned())];
        for workers in [2, 3] {
            let error =
                lookup_path_series_with_workers(std::slice::from_ref(&input), &kwargs, workers)
                    .unwrap_err();
            assert!(error.to_string().contains("could not decode MMDB"));
        }
    }

    #[test]
    fn parallel_scalar_dispatch_preserves_unequal_input_chunks() {
        let kwargs = kwargs(false);
        let candidates = [
            Some("89.160.20.128"),
            None,
            Some("not-an-ip"),
            Some("203.0.113.1"),
        ];
        let make_values = |length: usize, offset: usize| {
            (0..length)
                .map(|index| candidates[(index + offset) % candidates.len()])
                .collect::<Vec<_>>()
        };
        let first = make_values(4_097, 0);
        let second = make_values(4_111, first.len());
        let mut input = series(&first);
        input.append(&series(&second)).unwrap();
        assert_eq!(input.n_chunks(), 2);

        let parallel =
            lookup_path_series_with_workers(std::slice::from_ref(&input), &kwargs, 3).unwrap();
        let serial =
            lookup_path_series_with_workers(std::slice::from_ref(&input), &kwargs, 1).unwrap();

        assert!(parallel.equals_missing(&serial));
        assert_eq!(
            parallel.n_chunks(),
            input.len().div_ceil(PARALLEL_SCALAR_MAX_CHUNK_ROWS)
        );
    }

    #[test]
    fn parallel_scalar_dispatch_propagates_strict_errors() {
        let mut values = vec![Some("89.160.20.128"); PARALLEL_SCALAR_MIN_ROWS];
        values[PARALLEL_SCALAR_MIN_ROWS / 2] = Some("not-an-ip");
        let input = series(&values);

        let error = lookup_path_series_with_workers(&[input], &kwargs(true), 3).unwrap_err();
        assert!(error.to_string().contains("invalid IP address"));
    }

    fn collect_corrupt_fixtures() -> Vec<std::path::PathBuf> {
        let mut fixtures = Vec::new();
        for directory in [
            "tests/data/bad-data/libmaxminddb",
            "tests/data/bad-data/maxminddb-golang",
            "tests/data/bad-data/maxminddb-python",
            "tests/data/test-data",
        ] {
            for entry in fs::read_dir(directory).unwrap() {
                let path = entry.unwrap().path();
                let file_name = path.file_name().unwrap().to_string_lossy();
                let normalized_name = file_name.to_ascii_lowercase();
                if path
                    .extension()
                    .is_some_and(|extension| extension == "mmdb")
                    && (directory.contains("bad-data")
                        || normalized_name.contains("broken")
                        || normalized_name.contains("invalid"))
                {
                    fixtures.push(path);
                }
            }
        }
        fixtures.sort();
        fixtures
    }

    #[test]
    fn corrupt_fixtures_return_results_without_panicking() {
        let fixtures = collect_corrupt_fixtures();
        let values = vec![Some("1.1.1.1"); PARALLEL_SCALAR_MIN_ROWS];
        let input = series(&values);
        assert_eq!(
            fixtures.len(),
            29,
            "expected the complete corruption corpus"
        );
        for database in fixtures {
            let outcome = catch_unwind(AssertUnwindSafe(|| {
                let kwargs = LookupPathKwargs {
                    database: identity(database.to_str().unwrap()),
                    path: vec![
                        PathPart::Key("country".to_owned()),
                        PathPart::Key("iso_code".to_owned()),
                    ],
                    dtype: Some(SchemaSpec::String),
                    strict: false,
                };
                let _ = lookup_path_series_with_workers(std::slice::from_ref(&input), &kwargs, 3);
            }));
            assert!(outcome.is_ok(), "fixture panicked: {}", database.display());
        }
    }

    #[test]
    fn projected_decoder_enforces_resource_limits() {
        for (fixture, inner, message) in [
            (
                "MaxMind-DB-test-decoder-value-limit-over.mmdb",
                SchemaSpec::UInt16,
                "maximum number of data structure values",
            ),
            (
                "MaxMind-DB-test-decoder-payload-limit-over.mmdb",
                SchemaSpec::Binary,
                "maximum size of data structure string and bytes",
            ),
            (
                "MaxMind-DB-test-payload-amplification-dos.mmdb",
                SchemaSpec::Binary,
                "maximum size of data structure string and bytes",
            ),
        ] {
            let reader = Reader::open_readfile(format!("tests/data/test-data/{fixture}")).unwrap();
            let result = reader.lookup("1.2.3.4".parse().unwrap()).unwrap();
            let schema = SchemaSpec::List {
                inner: Box::new(inner),
            };

            let error = with_projected_schema(&schema, || decode_projected_path(&result, &[]))
                .expect_err("oversized projected values must be rejected");

            assert!(
                matches!(&error, maxminddb::MaxMindDbError::ResourceLimit { .. }),
                "{fixture}: {error}"
            );
            assert!(error.to_string().contains(message), "{fixture}: {error}");
        }
    }

    #[test]
    fn projected_decoder_accepts_resource_limit_boundaries() {
        for (fixture, inner, expected_length, expected_payload) in [
            (
                "MaxMind-DB-test-decoder-value-limit.mmdb",
                SchemaSpec::UInt16,
                (1 << 16) - 1,
                0,
            ),
            (
                "MaxMind-DB-test-decoder-payload-limit.mmdb",
                SchemaSpec::Binary,
                33,
                2 << 20,
            ),
        ] {
            let reader = Reader::open_readfile(format!("tests/data/test-data/{fixture}")).unwrap();
            let result = reader.lookup("1.2.3.4".parse().unwrap()).unwrap();
            let schema = SchemaSpec::List {
                inner: Box::new(inner),
            };

            let value = with_projected_schema(&schema, || decode_projected_path(&result, &[]))
                .unwrap()
                .unwrap();
            let Value::List(values) = value else {
                panic!("expected a list from {fixture}")
            };

            assert_eq!(values.len(), expected_length, "{fixture}");
            let payload: usize = values
                .iter()
                .map(|value| match value {
                    Value::Binary(bytes) => bytes.len(),
                    Value::UInt16(0) => 0,
                    other => panic!("unexpected value in {fixture}: {other:?}"),
                })
                .sum();
            assert_eq!(payload, expected_payload, "{fixture}");
        }
    }

    #[test]
    fn fuzz_discovered_extended_type_overflow_is_an_invalid_database_error() {
        let encoded =
            include_str!("../tests/fuzz-fixtures/projected-value-extended-type-overflow.mmdb.b64");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded.split_whitespace().collect::<String>())
            .unwrap();
        let reader = Reader::from_source(bytes).unwrap();
        let result = reader
            .lookup(IpAddr::V4(Ipv4Addr::new(89, 160, 20, 128)))
            .unwrap();
        let schema = crate::schema::known_schema("GeoIP2-City").unwrap();

        let error = with_projected_schema(&schema, || decode_projected_path(&result, &[]))
            .expect_err("malformed extended types must be rejected");

        assert!(matches!(
            error,
            maxminddb::MaxMindDbError::InvalidDatabase { message, offset: Some(_) }
                if message.contains("unknown data type: 258")
        ));
    }

    fn projected_schema() -> SchemaSpec {
        SchemaSpec::Struct {
            fields: vec![
                crate::schema::SchemaField {
                    name: "label".to_owned(),
                    dtype: SchemaSpec::String,
                },
                crate::schema::SchemaField {
                    name: "nested".to_owned(),
                    dtype: SchemaSpec::Struct {
                        fields: vec![crate::schema::SchemaField {
                            name: "values".to_owned(),
                            dtype: SchemaSpec::List {
                                inner: Box::new(SchemaSpec::UInt32),
                            },
                        }],
                    },
                },
            ],
        }
    }

    fn projected_value(label: &'static str, values: Vec<u32>) -> Value<'static> {
        Value::Map(vec![
            (Cow::Borrowed("label"), Value::String(Cow::Borrowed(label))),
            (
                Cow::Borrowed("nested"),
                Value::Map(vec![(
                    Cow::Borrowed("values"),
                    Value::List(values.into_iter().map(Value::UInt32).collect()),
                )]),
            ),
        ])
    }

    proptest! {
        #[test]
        fn optimized_gather_matches_the_row_wise_reference(
            rows in prop::collection::vec(prop::option::of(0usize..3), 0..512)
        ) {
            let unique_values = vec![
                Some(projected_value("first", vec![1, 2, 3])),
                None,
                Some(projected_value("second", vec![])),
            ];
            let schema = projected_schema();
            let optimized = decoded_values_to_series(
                "record".into(),
                &unique_values,
                &rows,
                &schema,
            ).unwrap();
            let gathered = rows
                .iter()
                .map(|row| row.and_then(|index| unique_values[index].clone()))
                .collect();
            let reference = values_to_series("record".into(), &schema, gathered).unwrap();

            prop_assert!(optimized.equals_missing(&reference));
        }

        #[test]
        fn random_null_and_duplicate_inputs_match_direct_lookups(
            choices in prop::collection::vec(0u8..8, 0..256)
        ) {
            let candidates = [
                Some("89.160.20.128"),
                Some("89.160.20.129"),
                Some("203.0.113.1"),
                Some("2001:db8::1"),
                Some("not-an-ip"),
                Some("999.1.1.1"),
                Some(""),
                None,
            ];
            let inputs = choices
                .iter()
                .map(|choice| candidates[usize::from(*choice)])
                .collect::<Vec<_>>();
            let output = lookup_path_series(&[series(&inputs)], &kwargs(false)).unwrap();
            let reader = reader_for(&kwargs(false).database).unwrap();
            let path = [
                maxminddb::PathElement::Key("country"),
                maxminddb::PathElement::Key("iso_code"),
            ];
            let expected = inputs
                .iter()
                .map(|value| {
                    value
                        .and_then(|value| value.parse::<IpAddr>().ok())
                        .and_then(|ip| reader.lookup(ip).ok())
                        .and_then(|result| result.decode_path::<&str>(&path).ok().flatten())
                        .map(str::to_owned)
                })
                .collect::<Vec<_>>();

            prop_assert_eq!(output.str().unwrap().iter().map(|value| value.map(str::to_owned)).collect::<Vec<_>>(), expected);
        }

        #[test]
        fn arbitrary_path_indexes_never_panic(index in any::<i64>()) {
            let path = [PathPart::Index(index)];
            let _ = to_mmdb_path(&path);
        }
    }
}

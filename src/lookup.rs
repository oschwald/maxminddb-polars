use std::borrow::Cow;
use std::net::IpAddr;

use maxminddb::LookupResult;
use polars::prelude::*;
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
use crate::schema::{PathPart, SchemaSpec, resolve_path_dtype, resolve_record_dtype, to_mmdb_path};
use crate::value::{Value, decode_projected_path, values_to_series, with_projected_schema};

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
        ($ty:ty, $chunked:ty) => {{
            if ips.len() >= PARALLEL_SCALAR_MIN_ROWS && scalar_workers > 2 {
                let chunk_name = name.clone();
                let chunks = decode_scalar_chunks_parallel::<$ty, _, _>(
                    ips,
                    &reader,
                    &path,
                    kwargs,
                    move |values| {
                        <$chunked>::from_iter_options(chunk_name.clone(), values.into_iter())
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
                    .map(|row| row.and_then(|index| unique_values[index]));
                Ok(<$chunked>::from_iter_options(name, values).into_series())
            }
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
        batch
            .unique
            .iter()
            .map(|result| decode_known(result, record))
            .collect::<PolarsResult<Vec<_>>>()
    })?;
    decoded_values_to_series(input.name().clone(), &unique_values, &batch.rows, &dtype)
}

pub(crate) struct LookupBatch<'a> {
    pub(crate) unique: Vec<LookupResult<'a, Vec<u8>>>,
    pub(crate) rows: Vec<Option<usize>>,
}

type DecodedScalarValues<T> = (Vec<Option<T>>, Vec<Option<usize>>);

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
    let chunks = ips
        .downcast_iter()
        .flat_map(|array| {
            (0..array.len())
                .step_by(PARALLEL_SCALAR_MAX_CHUNK_ROWS)
                .map(move |start| {
                    let end = (start + PARALLEL_SCALAR_MAX_CHUNK_ROWS).min(array.len());
                    (array, start, end)
                })
        })
        .collect::<Vec<_>>();
    guard_mmdb_operation(&kwargs.database, || {
        RAYON.install(|| {
            chunks
                .into_par_iter()
                .map(|(array, start, end)| {
                    let mut values = Vec::with_capacity(end - start);
                    for index in start..end {
                        let Some(result) =
                            lookup_one(array.get(index), reader, &kwargs.database, kwargs.strict)?
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
        let serial = lookup_path_series_with_workers(&[input], &kwargs, 1).unwrap();

        assert!(parallel.equals_missing(&serial));
        assert_eq!(parallel.n_chunks(), 6);
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
        assert_eq!(
            fixtures.len(),
            25,
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
                let _ = lookup_path_series(&[series(&[Some("1.1.1.1")])], &kwargs);
            }));
            assert!(outcome.is_ok(), "fixture panicked: {}", database.display());
        }
    }

    #[test]
    fn fuzz_discovered_extended_type_overflow_is_a_decoder_error() {
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

        assert!(
            error.to_string().contains("expected map, got type 258"),
            "{error}"
        );
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

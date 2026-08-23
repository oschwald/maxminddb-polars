use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::sync::Arc;

use maxminddb::{LookupResult, MaxMindDbError, PathElement};
use polars::chunked_array::builder::{AnonymousOwnedListBuilder, ListBuilderTrait};
use polars::prelude::*;
use polars_arrow::bitmap::Bitmap;
use serde::de::{DeserializeSeed, Error as _, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::schema::{SchemaField, SchemaSpec};

#[derive(Clone, Debug, PartialEq)]
pub enum Value<'a> {
    Null,
    Boolean(bool),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    UInt128(u128),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    Int128(i128),
    Float32(f32),
    Float64(f64),
    String(Cow<'a, str>),
    Binary(Cow<'a, [u8]>),
    List(Vec<Value<'a>>),
    Map(Vec<(Cow<'a, str>, Value<'a>)>),
}

thread_local! {
    static PROJECTED_SCHEMAS: RefCell<Vec<Arc<SchemaSpec>>> = const { RefCell::new(Vec::new()) };
}

struct ProjectedSchemaGuard;

impl Drop for ProjectedSchemaGuard {
    fn drop(&mut self) {
        PROJECTED_SCHEMAS.with(|schemas| {
            schemas.borrow_mut().pop();
        });
    }
}

pub fn with_projected_schema<R>(schema: &SchemaSpec, callback: impl FnOnce() -> R) -> R {
    PROJECTED_SCHEMAS.with(|schemas| {
        schemas.borrow_mut().push(Arc::new(schema.clone()));
    });
    let _guard = ProjectedSchemaGuard;
    callback()
}

pub fn decode_projected_path<'a>(
    result: &LookupResult<'a, Vec<u8>>,
    path: &[PathElement<'_>],
) -> Result<Option<Value<'a>>, MaxMindDbError> {
    result
        .decode_path::<ProjectedValue<'a>>(path)
        .map(|value| value.map(|value| value.0))
}

struct ProjectedValue<'a>(Value<'a>);

impl<'de> Deserialize<'de> for ProjectedValue<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let schema = PROJECTED_SCHEMAS
            .with(|schemas| schemas.borrow().last().cloned())
            .ok_or_else(|| D::Error::custom("projected MMDB decoder has no active schema"))?;
        SchemaSeed(&schema)
            .deserialize(deserializer)
            .map(ProjectedValue)
    }
}

#[derive(Clone, Copy)]
struct SchemaSeed<'a>(&'a SchemaSpec);

impl<'de> DeserializeSeed<'de> for SchemaSeed<'_> {
    type Value = Value<'de>;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        match self.0 {
            SchemaSpec::Struct { fields } => {
                deserializer.deserialize_map(ProjectedMapVisitor { fields })
            }
            SchemaSpec::List { inner } => {
                deserializer.deserialize_seq(ProjectedListVisitor { inner })
            }
            schema => {
                let value = Value::deserialize(deserializer)?;
                validate_scalar(&value, schema).map_err(D::Error::custom)?;
                Ok(value)
            }
        }
    }
}

struct ProjectedMapVisitor<'a> {
    fields: &'a [SchemaField],
}

impl<'de> Visitor<'de> for ProjectedMapVisitor<'_> {
    type Value = Value<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an MMDB map matching the requested Struct dtype")
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = vec![None; self.fields.len()];
        while let Some(key) = map.next_key::<String>()? {
            if let Some((index, field)) = self
                .fields
                .iter()
                .enumerate()
                .find(|(_, field)| field.name == key)
            {
                values[index] = Some(map.next_value_seed(SchemaSeed(&field.dtype))?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        Ok(Value::Map(
            self.fields
                .iter()
                .zip(values)
                .map(|(field, value)| {
                    (
                        Cow::Owned(field.name.clone()),
                        value.unwrap_or_else(|| default_value(&field.dtype)),
                    )
                })
                .collect(),
        ))
    }
}

struct ProjectedListVisitor<'a> {
    inner: &'a SchemaSpec,
}

impl<'de> Visitor<'de> for ProjectedListVisitor<'_> {
    type Value = Value<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an MMDB array matching the requested List dtype")
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element_seed(SchemaSeed(self.inner))? {
            values.push(value);
        }
        Ok(Value::List(values))
    }
}

impl Value<'_> {
    fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean(_) => "bool",
            Self::UInt8(_) => "uint8",
            Self::UInt16(_) => "uint16",
            Self::UInt32(_) => "uint32",
            Self::UInt64(_) => "uint64",
            Self::UInt128(_) => "uint128",
            Self::Int8(_) => "int8",
            Self::Int16(_) => "int16",
            Self::Int32(_) => "int32",
            Self::Int64(_) => "int64",
            Self::Int128(_) => "int128",
            Self::Float32(_) => "float32",
            Self::Float64(_) => "float64",
            Self::String(_) => "string",
            Self::Binary(_) => "binary",
            Self::List(_) => "list",
            Self::Map(_) => "map",
        }
    }
}

impl<'de> Deserialize<'de> for Value<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value<'de>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a supported MaxMind DB value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        Deserialize::deserialize(deserializer)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Boolean(value))
    }

    fn visit_u8<E>(self, value: u8) -> Result<Self::Value, E> {
        Ok(Value::UInt8(value))
    }

    fn visit_u16<E>(self, value: u16) -> Result<Self::Value, E> {
        Ok(Value::UInt16(value))
    }

    fn visit_u32<E>(self, value: u32) -> Result<Self::Value, E> {
        Ok(Value::UInt32(value))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::UInt64(value))
    }

    fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E> {
        Ok(Value::UInt128(value))
    }

    fn visit_i8<E>(self, value: i8) -> Result<Self::Value, E> {
        Ok(Value::Int8(value))
    }

    fn visit_i16<E>(self, value: i16) -> Result<Self::Value, E> {
        Ok(Value::Int16(value))
    }

    fn visit_i32<E>(self, value: i32) -> Result<Self::Value, E> {
        Ok(Value::Int32(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Int64(value))
    }

    fn visit_i128<E>(self, value: i128) -> Result<Self::Value, E> {
        Ok(Value::Int128(value))
    }

    fn visit_f32<E>(self, value: f32) -> Result<Self::Value, E> {
        Ok(Value::Float32(value))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E> {
        Ok(Value::Float64(value))
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(Value::String(Cow::Borrowed(value)))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(Cow::Owned(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(Cow::Owned(value)))
    }

    fn visit_borrowed_bytes<E>(self, value: &'de [u8]) -> Result<Self::Value, E> {
        Ok(Value::Binary(Cow::Borrowed(value)))
    }

    fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E> {
        Ok(Value::Binary(Cow::Owned(value.to_vec())))
    }

    fn visit_byte_buf<E>(self, value: Vec<u8>) -> Result<Self::Value, E> {
        Ok(Value::Binary(Cow::Owned(value)))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(Value::List(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((key, value)) = map.next_entry::<Cow<'de, str>, Value<'de>>()? {
            values.push((key, value));
        }
        Ok(Value::Map(values))
    }
}

pub fn default_value(schema: &SchemaSpec) -> Value<'static> {
    match schema {
        SchemaSpec::Struct { fields } => Value::Map(
            fields
                .iter()
                .map(|field| (Cow::Owned(field.name.clone()), default_value(&field.dtype)))
                .collect(),
        ),
        SchemaSpec::List { .. } => Value::List(Vec::new()),
        _ => Value::Null,
    }
}

#[cfg(test)]
fn project_value<'a>(value: &Value<'a>, schema: &SchemaSpec) -> PolarsResult<Value<'a>> {
    match (value, schema) {
        (Value::Map(values), SchemaSpec::Struct { fields }) => Ok(Value::Map(
            fields
                .iter()
                .map(|field| {
                    let projected = values
                        .iter()
                        .find(|(name, _)| name.as_ref() == field.name)
                        .map(|(_, value)| project_value(value, &field.dtype))
                        .unwrap_or_else(|| Ok(default_value(&field.dtype)));
                    projected.map(|value| (Cow::Owned(field.name.clone()), value))
                })
                .collect::<PolarsResult<Vec<_>>>()?,
        )),
        (Value::List(values), SchemaSpec::List { inner }) => Ok(Value::List(
            values
                .iter()
                .map(|value| project_value(value, inner))
                .collect::<PolarsResult<Vec<_>>>()?,
        )),
        (Value::Null, SchemaSpec::Struct { .. } | SchemaSpec::List { .. }) => {
            Ok(default_value(schema))
        }
        (Value::Null, _) => Ok(Value::Null),
        (value, SchemaSpec::Struct { .. }) => {
            polars_bail!(ComputeError: "expected MMDB map, got {}", value.kind())
        }
        (value, SchemaSpec::List { .. }) => {
            polars_bail!(ComputeError: "expected MMDB array, got {}", value.kind())
        }
        (value, _) => {
            validate_scalar(value, schema)?;
            Ok(value.clone())
        }
    }
}

fn validate_scalar(value: &Value<'_>, schema: &SchemaSpec) -> PolarsResult<()> {
    let valid = match schema {
        SchemaSpec::Boolean => matches!(value, Value::Boolean(_)),
        SchemaSpec::UInt8 => unsigned(value).is_some_and(|value| u8::try_from(value).is_ok()),
        SchemaSpec::UInt16 => unsigned(value).is_some_and(|value| u16::try_from(value).is_ok()),
        SchemaSpec::UInt32 => unsigned(value).is_some_and(|value| u32::try_from(value).is_ok()),
        SchemaSpec::UInt64 => unsigned(value).is_some_and(|value| u64::try_from(value).is_ok()),
        SchemaSpec::UInt128 => unsigned(value).is_some(),
        SchemaSpec::Int8 => signed(value).is_some_and(|value| i8::try_from(value).is_ok()),
        SchemaSpec::Int16 => signed(value).is_some_and(|value| i16::try_from(value).is_ok()),
        SchemaSpec::Int32 => signed(value).is_some_and(|value| i32::try_from(value).is_ok()),
        SchemaSpec::Int64 => signed(value).is_some_and(|value| i64::try_from(value).is_ok()),
        SchemaSpec::Int128 => signed(value).is_some(),
        SchemaSpec::Float32 => matches!(value, Value::Float32(_)),
        SchemaSpec::Float64 => matches!(value, Value::Float32(_) | Value::Float64(_)),
        SchemaSpec::String => matches!(value, Value::String(_)),
        SchemaSpec::Binary => matches!(value, Value::Binary(_)),
        SchemaSpec::List { .. } | SchemaSpec::Struct { .. } => unreachable!(),
    };
    if valid {
        Ok(())
    } else {
        polars_bail!(
            ComputeError:
            "MMDB value of type {} does not match requested dtype {}",
            value.kind(),
            schema.to_polars()
        )
    }
}

fn unsigned(value: &Value<'_>) -> Option<u128> {
    match value {
        Value::UInt8(value) => Some((*value).into()),
        Value::UInt16(value) => Some((*value).into()),
        Value::UInt32(value) => Some((*value).into()),
        Value::UInt64(value) => Some((*value).into()),
        Value::UInt128(value) => Some(*value),
        _ => None,
    }
}

fn signed(value: &Value<'_>) -> Option<i128> {
    match value {
        Value::Int8(value) => Some((*value).into()),
        Value::Int16(value) => Some((*value).into()),
        Value::Int32(value) => Some((*value).into()),
        Value::Int64(value) => Some((*value).into()),
        Value::Int128(value) => Some(*value),
        _ => None,
    }
}

pub fn values_to_series(
    name: PlSmallStr,
    schema: &SchemaSpec,
    values: Vec<Option<Value<'_>>>,
) -> PolarsResult<Series> {
    macro_rules! unsigned_series {
        ($ty:ty, $chunked:ty) => {{
            let values = values
                .iter()
                .map(|value| match value {
                    None | Some(Value::Null) => Ok(None),
                    Some(value) => unsigned(value)
                        .and_then(|value| <$ty>::try_from(value).ok())
                        .map(Some)
                        .ok_or_else(|| type_error(value, schema)),
                })
                .collect::<PolarsResult<Vec<Option<$ty>>>>()?;
            Ok(<$chunked>::from_iter_options(name, values.into_iter()).into_series())
        }};
    }

    macro_rules! signed_series {
        ($ty:ty, $chunked:ty) => {{
            let values = values
                .iter()
                .map(|value| match value {
                    None | Some(Value::Null) => Ok(None),
                    Some(value) => signed(value)
                        .and_then(|value| <$ty>::try_from(value).ok())
                        .map(Some)
                        .ok_or_else(|| type_error(value, schema)),
                })
                .collect::<PolarsResult<Vec<Option<$ty>>>>()?;
            Ok(<$chunked>::from_iter_options(name, values.into_iter()).into_series())
        }};
    }

    match schema {
        SchemaSpec::Boolean => {
            let values = values.iter().map(|value| match value {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Boolean(value)) => Ok(Some(*value)),
                Some(value) => Err(type_error(value, schema)),
            });
            Ok(BooleanChunked::from_iter_options(
                name,
                values.collect::<PolarsResult<Vec<_>>>()?.into_iter(),
            )
            .into_series())
        }
        SchemaSpec::UInt8 => unsigned_series!(u8, UInt8Chunked),
        SchemaSpec::UInt16 => unsigned_series!(u16, UInt16Chunked),
        SchemaSpec::UInt32 => unsigned_series!(u32, UInt32Chunked),
        SchemaSpec::UInt64 => unsigned_series!(u64, UInt64Chunked),
        SchemaSpec::UInt128 => unsigned_series!(u128, UInt128Chunked),
        SchemaSpec::Int8 => signed_series!(i8, Int8Chunked),
        SchemaSpec::Int16 => signed_series!(i16, Int16Chunked),
        SchemaSpec::Int32 => signed_series!(i32, Int32Chunked),
        SchemaSpec::Int64 => signed_series!(i64, Int64Chunked),
        SchemaSpec::Int128 => signed_series!(i128, Int128Chunked),
        SchemaSpec::Float32 => {
            let values = values.iter().map(|value| match value {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Float32(value)) => Ok(Some(*value)),
                Some(value) => Err(type_error(value, schema)),
            });
            Ok(Float32Chunked::from_iter_options(
                name,
                values.collect::<PolarsResult<Vec<_>>>()?.into_iter(),
            )
            .into_series())
        }
        SchemaSpec::Float64 => {
            let values = values.iter().map(|value| match value {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Float32(value)) => Ok(Some((*value).into())),
                Some(Value::Float64(value)) => Ok(Some(*value)),
                Some(value) => Err(type_error(value, schema)),
            });
            Ok(Float64Chunked::from_iter_options(
                name,
                values.collect::<PolarsResult<Vec<_>>>()?.into_iter(),
            )
            .into_series())
        }
        SchemaSpec::String => {
            let values = values.iter().map(|value| match value {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(value)) => Ok(Some(value.as_ref())),
                Some(value) => Err(type_error(value, schema)),
            });
            Ok(StringChunked::from_iter_options(
                name,
                values.collect::<PolarsResult<Vec<_>>>()?.into_iter(),
            )
            .into_series())
        }
        SchemaSpec::Binary => {
            let values = values.iter().map(|value| match value {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Binary(value)) => Ok(Some(value.as_ref())),
                Some(value) => Err(type_error(value, schema)),
            });
            Ok(BinaryChunked::from_iter_options(
                name,
                values.collect::<PolarsResult<Vec<_>>>()?.into_iter(),
            )
            .into_series())
        }
        SchemaSpec::Struct { fields } => struct_series(name, fields, values),
        SchemaSpec::List { inner } => list_series(name, inner, values),
    }
}

fn type_error(value: &Value<'_>, schema: &SchemaSpec) -> PolarsError {
    polars_err!(
        ComputeError:
        "MMDB value of type {} does not match requested dtype {}",
        value.kind(),
        schema.to_polars()
    )
}

fn struct_series(
    name: PlSmallStr,
    fields: &[SchemaField],
    values: Vec<Option<Value<'_>>>,
) -> PolarsResult<Series> {
    let validity = Bitmap::from_iter(
        values
            .iter()
            .map(|value| match value {
                None | Some(Value::Null) => Ok(false),
                Some(Value::Map(_)) => Ok(true),
                Some(value) => Err(type_error(
                    value,
                    &SchemaSpec::Struct {
                        fields: fields.to_vec(),
                    },
                )),
            })
            .collect::<PolarsResult<Vec<_>>>()?,
    );

    let mut children = Vec::with_capacity(fields.len());
    for field in fields {
        let child_values = values
            .iter()
            .map(|value| match value {
                Some(Value::Map(values)) => Some(
                    values
                        .iter()
                        .find(|(name, _)| name.as_ref() == field.name)
                        .map(|(_, value)| value.clone())
                        .unwrap_or_else(|| default_value(&field.dtype)),
                ),
                None | Some(Value::Null) => None,
                Some(_) => unreachable!("validated above"),
            })
            .collect();
        children.push(values_to_series(
            field.name.clone().into(),
            &field.dtype,
            child_values,
        )?);
    }

    Ok(
        StructChunked::from_series(name, values.len(), children.iter())?
            .with_outer_validity(Some(validity))
            .into_series(),
    )
}

fn list_series(
    name: PlSmallStr,
    inner: &SchemaSpec,
    values: Vec<Option<Value<'_>>>,
) -> PolarsResult<Series> {
    let mut builder = AnonymousOwnedListBuilder::new(name, values.len(), Some(inner.to_polars()));
    for value in values {
        match value {
            None | Some(Value::Null) => builder.append_null(),
            Some(Value::List(values)) => {
                let values = values.into_iter().map(Some).collect();
                builder.append_owned_series(values_to_series(PlSmallStr::EMPTY, inner, values)?)?;
            }
            Some(value) => {
                return Err(type_error(
                    &value,
                    &SchemaSpec::List {
                        inner: Box::new(inner.clone()),
                    },
                ));
            }
        }
    }
    Ok(builder.finish().into_series())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, dtype: SchemaSpec) -> SchemaField {
        SchemaField {
            name: name.to_owned(),
            dtype,
        }
    }

    #[test]
    fn projects_declared_shape_and_defaults_missing_containers() {
        let schema = SchemaSpec::Struct {
            fields: vec![
                field(
                    "nested",
                    SchemaSpec::Struct {
                        fields: vec![field("name", SchemaSpec::String)],
                    },
                ),
                field(
                    "items",
                    SchemaSpec::List {
                        inner: Box::new(SchemaSpec::UInt32),
                    },
                ),
            ],
        };
        let projected = project_value(&Value::Map(Vec::new()), &schema).unwrap();
        assert_eq!(projected, default_value(&schema));
    }

    #[test]
    fn builds_nested_series_without_json_or_python_values() {
        let schema = SchemaSpec::Struct {
            fields: vec![field("name", SchemaSpec::String)],
        };
        let values = vec![
            Some(Value::Map(vec![(
                Cow::Borrowed("name"),
                Value::String(Cow::Borrowed("example")),
            )])),
            None,
        ];
        let series = values_to_series("record".into(), &schema, values).unwrap();
        assert_eq!(series.null_count(), 1);
        assert_eq!(
            series
                .struct_()
                .unwrap()
                .field_by_name("name")
                .unwrap()
                .str()
                .unwrap()
                .get(0),
            Some("example")
        );
    }
}

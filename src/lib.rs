mod cache;
mod known;
mod lookup;
mod schema;
mod value;

#[cfg(feature = "fuzzing")]
#[doc(hidden)]
pub mod fuzzing;

use polars::prelude::*;
use pyo3::prelude::*;
use pyo3_polars::derive::polars_expr;

use crate::lookup::{LookupPathKwargs, LookupRecordKwargs};

#[pymodule]
fn _maxminddb_polars(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}

#[polars_expr(output_type=String)]
fn identity(inputs: &[Series]) -> PolarsResult<Series> {
    identity_series(inputs)
}

fn identity_series(inputs: &[Series]) -> PolarsResult<Series> {
    let [input] = inputs else {
        polars_bail!(InvalidOperation: "identity smoke expression expects one input column")
    };
    if input.dtype() != &DataType::String {
        polars_bail!(InvalidOperation: "identity smoke expression expects String input")
    }
    Ok(input.clone())
}

fn lookup_path_output(input_fields: &[Field], kwargs: LookupPathKwargs) -> PolarsResult<Field> {
    let [input] = input_fields else {
        polars_bail!(InvalidOperation: "MMDB lookup expects exactly one input column")
    };
    if input.dtype() != &DataType::String {
        polars_bail!(InvalidOperation: "MMDB lookup input must have String dtype")
    }
    let dtype = lookup::output_dtype(&kwargs)?.to_polars();
    Ok(Field::new(input.name().clone(), dtype))
}

#[polars_expr(output_type_func_with_kwargs=lookup_path_output)]
fn mmdb_lookup_path(inputs: &[Series], kwargs: LookupPathKwargs) -> PolarsResult<Series> {
    lookup::lookup_path_series(inputs, &kwargs)
}

fn lookup_output(input_fields: &[Field], kwargs: LookupRecordKwargs) -> PolarsResult<Field> {
    let [input] = input_fields else {
        polars_bail!(InvalidOperation: "MMDB lookup expects exactly one input column")
    };
    if input.dtype() != &DataType::String {
        polars_bail!(InvalidOperation: "MMDB lookup input must have String dtype")
    }
    let dtype = lookup::record_output_dtype(&kwargs)?.to_polars();
    Ok(Field::new(input.name().clone(), dtype))
}

#[polars_expr(output_type_func_with_kwargs=lookup_output)]
fn mmdb_lookup(inputs: &[Series], kwargs: LookupRecordKwargs) -> PolarsResult<Series> {
    lookup::lookup_record_series(inputs, &kwargs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_returns_its_input() {
        let input = Series::new("value".into(), ["one", "two"]);
        let output = identity_series(&[input]).unwrap();

        assert_eq!(output.name().as_str(), "value");
        assert_eq!(output.str().unwrap().get(0), Some("one"));
        assert_eq!(output.str().unwrap().get(1), Some("two"));
    }
}

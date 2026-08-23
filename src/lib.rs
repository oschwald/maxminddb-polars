use polars::prelude::*;
use pyo3::prelude::*;
use pyo3_polars::derive::polars_expr;

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

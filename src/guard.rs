use std::panic::{AssertUnwindSafe, catch_unwind};

/// Keep parser panics caused by corrupt MMDB input from crossing the plugin boundary.
pub(crate) fn catch_mmdb_unwind<T>(operation: impl FnOnce() -> T) -> Result<T, ()> {
    catch_unwind(AssertUnwindSafe(operation)).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_parser_panics() {
        assert!(catch_mmdb_unwind(|| panic!("corrupt input")).is_err());
        assert_eq!(catch_mmdb_unwind(|| 42), Ok(42));
    }
}

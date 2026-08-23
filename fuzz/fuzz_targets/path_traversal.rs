#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    maxminddb_polars::fuzzing::path_traversal(data);
});

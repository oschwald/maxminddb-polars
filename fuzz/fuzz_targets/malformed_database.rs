#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(
    init: maxminddb_polars::fuzzing::initialize_panic_hook(),
    |data: &[u8]| {
        maxminddb_polars::fuzzing::malformed_database(data);
    }
);

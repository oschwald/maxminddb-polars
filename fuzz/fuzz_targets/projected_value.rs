#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    maxminddb_polars::fuzzing::projected_value(data);
});

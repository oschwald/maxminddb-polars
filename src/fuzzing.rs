//! Entry points used only by the out-of-tree `cargo-fuzz` harnesses.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, Ordering};

use maxminddb::Reader;
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::guard::catch_mmdb_unwind;
use crate::lookup::{LookupPathKwargs, LookupRecordKwargs, decode_known_record};
use crate::schema::{
    KnownRecord, PathPart, SchemaSpec, dtype_at_path, field, known_schema, struct_, to_mmdb_path,
};
use crate::value::{decode_projected_path, with_projected_schema};

const MAX_FUZZ_INPUT_BYTES: usize = 8 * 1024 * 1024;

/// Built once, because the City schema allocates hundreds of strings.
/// `with_projected_schema` still clones each one for every input.
static MALFORMED_DATABASE_SCHEMAS: LazyLock<[SchemaSpec; 2]> = LazyLock::new(|| {
    [
        decoder_test_schema(),
        known_schema("GeoIP2-City").expect("City is a built-in schema"),
    ]
});

/// Let the production unwind guard observe parser panics under libFuzzer.
///
/// `libfuzzer-sys` normally installs a hook that aborts before unwinding. Its
/// outer target boundary still aborts on any panic that escapes this crate, so
/// replacing the hook does not hide unrelated fuzz failures.
pub fn initialize_panic_hook() {
    static REPORTED: AtomicBool = AtomicBool::new(false);

    drop(std::panic::take_hook());
    std::panic::set_hook(Box::new(|panic_info| {
        if !REPORTED.swap(true, Ordering::Relaxed) {
            eprintln!("contained parser panic (further reports suppressed): {panic_info}");
        }
    }));
}

#[derive(Deserialize)]
struct PathInput {
    schema: SchemaSpec,
    path: Vec<PathPart>,
}

pub fn kwargs_deserialization(data: &[u8]) {
    if data.len() > MAX_FUZZ_INPUT_BYTES {
        return;
    }
    let _ = serde_json::from_slice::<LookupPathKwargs>(data);
    let _ = serde_json::from_slice::<LookupRecordKwargs>(data);
}

pub fn path_traversal(data: &[u8]) {
    if data.len() > MAX_FUZZ_INPUT_BYTES {
        return;
    }
    let Ok(input) = serde_json::from_slice::<PathInput>(data) else {
        return;
    };
    let _ = dtype_at_path(input.schema, &input.path);
    let _ = to_mmdb_path(&input.path);
}

pub fn projected_value(data: &[u8]) {
    if data.len() > MAX_FUZZ_INPUT_BYTES {
        return;
    }
    let _ = catch_mmdb_unwind(|| {
        let Ok(reader) = Reader::from_source(data.to_vec()) else {
            return;
        };
        let Ok(result) = reader.lookup(IpAddr::V4(Ipv4Addr::new(89, 160, 20, 128))) else {
            return;
        };
        let schema = known_schema("GeoIP2-City").expect("City is a built-in schema");
        let _ = with_projected_schema(&schema, || decode_projected_path(&result, &[]));
    });
}

pub fn malformed_database(data: &[u8]) {
    if data.len() > MAX_FUZZ_INPUT_BYTES {
        return;
    }
    let _ = catch_mmdb_unwind(|| {
        let Ok(reader) = Reader::from_source(data.to_vec()) else {
            return;
        };
        let results: Vec<_> = [
            IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
            // Records in the City, Country, Enterprise, ISP, Domain,
            // Anonymous IP, ASN, Connection Type, and Density/Income seeds.
            IpAddr::V4(Ipv4Addr::new(81, 2, 69, 160)),
            IpAddr::V4(Ipv4Addr::new(89, 160, 20, 128)),
            IpAddr::V4(Ipv4Addr::new(1, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(5, 83, 124, 1)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ]
        .into_iter()
        .filter_map(|ip| reader.lookup(ip).ok())
        .collect();
        for result in &results {
            let _ = result.decode::<Walk>();
        }
        for schema in MALFORMED_DATABASE_SCHEMAS.iter() {
            with_projected_schema(schema, || {
                for result in &results {
                    let _ = decode_projected_path(result, &[]);
                }
            });
        }
        // Decode whole records as a lookup on this database type does, which
        // retries typed decoding errors with the projected decoder.
        if let Some(record) = KnownRecord::from_database_type(&reader.metadata().database_type) {
            with_projected_schema(&record.schema(), || {
                for result in &results {
                    let _ = decode_known_record(result, record);
                }
            });
        }
    });
}

/// The fields of MaxMind-DB-test-decoder.mmdb.
///
/// Most fields request a dtype other than the stored type. Each one fits the
/// valid record, so it decodes in full and reaches every conversion.
fn decoder_test_schema() -> SchemaSpec {
    fn list(inner: SchemaSpec) -> SchemaSpec {
        SchemaSpec::List {
            inner: Box::new(inner),
        }
    }

    struct_(vec![
        field("array", list(SchemaSpec::UInt8)),
        field("boolean", SchemaSpec::Boolean),
        field("bytes", SchemaSpec::Binary),
        field("double", SchemaSpec::Float64),
        field("float", SchemaSpec::Float64),
        field("int32", SchemaSpec::Int64),
        field(
            "map",
            struct_(vec![field(
                "mapX",
                struct_(vec![
                    field("arrayX", list(SchemaSpec::Int16)),
                    field("utf8_stringX", SchemaSpec::String),
                ]),
            )]),
        ),
        field("uint16", SchemaSpec::UInt32),
        field("uint32", SchemaSpec::Int64),
        field("uint64", SchemaSpec::UInt128),
        field("uint128", SchemaSpec::UInt128),
        field("utf8_string", SchemaSpec::String),
    ])
}

/// Visits every value without keeping it.
///
/// The projected decoder skips keys outside its schema, so this reaches
/// containers under any key.
struct Walk;

impl<'de> Deserialize<'de> for Walk {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(WalkVisitor)
    }
}

struct WalkVisitor;

// serde forwards the narrower integer and float visits, and the borrowed
// string and bytes visits, to the methods below.
impl<'de> Visitor<'de> for WalkVisitor {
    type Value = Walk;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any MMDB value")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_i128<E>(self, _value: i128) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_u128<E>(self, _value: u128) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_str<E>(self, _value: &str) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_bytes<E>(self, _value: &[u8]) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_unit<E>(self) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_none<E>(self) -> Result<Walk, E> {
        Ok(Walk)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Walk, D::Error>
    where
        D: Deserializer<'de>,
    {
        Walk::deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Walk, A::Error>
    where
        A: SeqAccess<'de>,
    {
        while sequence.next_element::<Walk>()?.is_some() {}
        Ok(Walk)
    }

    fn visit_map<A>(self, mut map: A) -> Result<Walk, A::Error>
    where
        A: MapAccess<'de>,
    {
        while map.next_entry::<Walk, Walk>()?.is_some() {}
        Ok(Walk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_test_schema_decodes_the_valid_fixture() {
        let reader =
            Reader::open_readfile("tests/data/test-data/MaxMind-DB-test-decoder.mmdb").unwrap();
        let schema = decoder_test_schema();
        for ip in ["::1.1.1.0", "::0.0.0.0", "::255.255.255.255"] {
            let result = reader.lookup(ip.parse::<IpAddr>().unwrap()).unwrap();
            let value = with_projected_schema(&schema, || decode_projected_path(&result, &[]));
            assert!(value.unwrap().is_some(), "{ip}");
        }
    }

    #[test]
    fn walk_visits_every_valid_fixture_value() {
        for path in [
            "tests/data/test-data/MaxMind-DB-test-decoder.mmdb",
            "tests/data/test-data/GeoIP2-City-Test.mmdb",
        ] {
            let reader = Reader::open_readfile(path).unwrap();
            for network in reader.networks(Default::default()).unwrap() {
                let result = network.unwrap();
                assert!(result.decode::<Walk>().unwrap().is_some(), "{path}");
            }
        }
    }
}

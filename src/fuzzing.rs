//! Entry points used only by the out-of-tree `cargo-fuzz` harnesses.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use maxminddb::Reader;
use serde::Deserialize;

use crate::lookup::{LookupPathKwargs, LookupRecordKwargs};
use crate::schema::{PathPart, SchemaSpec, dtype_at_path, known_schema, to_mmdb_path};
use crate::value::{Value, decode_projected_path, with_projected_schema};

const MAX_FUZZ_INPUT_BYTES: usize = 8 * 1024 * 1024;

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
    let Ok(reader) = Reader::from_source(data.to_vec()) else {
        return;
    };
    let Ok(result) = reader.lookup(IpAddr::V4(Ipv4Addr::new(89, 160, 20, 128))) else {
        return;
    };
    let schema = known_schema("GeoIP2-City").expect("City is a built-in schema");
    let _ = with_projected_schema(&schema, || decode_projected_path(&result, &[]));
}

pub fn malformed_database(data: &[u8]) {
    if data.len() > MAX_FUZZ_INPUT_BYTES {
        return;
    }
    let Ok(reader) = Reader::from_source(data.to_vec()) else {
        return;
    };
    for ip in [
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ] {
        let Ok(result) = reader.lookup(ip) else {
            continue;
        };
        let _ = result.decode::<Value<'_>>();
    }
}

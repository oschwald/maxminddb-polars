use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::UNIX_EPOCH;

use maxminddb::Reader;
use polars::prelude::*;
use serde::{Deserialize, Serialize};

pub type CachedReader = Reader<Vec<u8>>;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct DatabaseIdentity {
    pub canonical_path: String,
    pub size: u64,
    pub modified_ns: u64,
}

static READERS: LazyLock<RwLock<HashMap<DatabaseIdentity, Arc<CachedReader>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

pub fn reader_for(identity: &DatabaseIdentity) -> PolarsResult<Arc<CachedReader>> {
    if let Some(reader) = READERS
        .read()
        .map_err(|_| polars_err!(ComputeError: "MMDB reader cache lock is poisoned"))?
        .get(identity)
    {
        return Ok(Arc::clone(reader));
    }

    // Keep the write lock while opening a new generation. Initial opens are
    // rare, and this coalesces concurrent construction of the same snapshot.
    let mut readers = READERS
        .write()
        .map_err(|_| polars_err!(ComputeError: "MMDB reader cache lock is poisoned"))?;
    if let Some(reader) = readers.get(identity) {
        return Ok(Arc::clone(reader));
    }

    let reader = Arc::new(open_snapshot(identity)?);
    readers.insert(identity.clone(), Arc::clone(&reader));
    Ok(reader)
}

fn open_snapshot(identity: &DatabaseIdentity) -> PolarsResult<CachedReader> {
    let path = Path::new(&identity.canonical_path);
    let canonical_path = path.canonicalize().map_err(|error| {
        polars_err!(
            ComputeError:
            "could not resolve MMDB path {:?}: {error}",
            identity.canonical_path
        )
    })?;
    if canonical_path_string(&canonical_path) != identity.canonical_path {
        polars_bail!(
            ComputeError:
            "MMDB path identity changed from {:?} to {:?}; reconstruct the expression",
            identity.canonical_path,
            canonical_path
        )
    }

    let before = identity_for_path(&canonical_path)?;
    if &before != identity {
        polars_bail!(
            ComputeError:
            "MMDB file {:?} changed after expression construction; reconstruct the expression",
            identity.canonical_path
        )
    }

    let bytes = fs::read(&canonical_path).map_err(|error| {
        polars_err!(
            ComputeError:
            "could not read MMDB {}: {error}",
            canonical_path.display()
        )
    })?;
    let after = identity_for_path(&canonical_path)?;
    if after != before {
        polars_bail!(
            ComputeError:
            "MMDB file {:?} changed while it was being read; reconstruct the expression",
            identity.canonical_path
        )
    }

    Reader::from_source(bytes).map_err(|error| {
        polars_err!(
            ComputeError:
            "could not open MMDB {}: {error}",
            canonical_path.display()
        )
    })
}

fn identity_for_path(path: &Path) -> PolarsResult<DatabaseIdentity> {
    let metadata = fs::metadata(path).map_err(
        |error| polars_err!(ComputeError: "could not stat MMDB {}: {error}", path.display()),
    )?;
    if !metadata.is_file() {
        polars_bail!(ComputeError: "MMDB path {} is not a file", path.display())
    }
    let modified = metadata.modified().map_err(|error| {
        polars_err!(
            ComputeError:
            "could not read modification time for MMDB {}: {error}",
            path.display()
        )
    })?;
    let modified_ns = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            polars_err!(
                ComputeError:
                "MMDB {} has a modification time before the Unix epoch: {error}",
                path.display()
            )
        })?
        .as_nanos()
        .try_into()
        .map_err(|_| {
            polars_err!(
                ComputeError:
                "MMDB {} has a modification time outside the supported range",
                path.display()
            )
        })?;

    Ok(DatabaseIdentity {
        canonical_path: canonical_path_string(path),
        size: metadata.len(),
        modified_ns,
    })
}

fn canonical_path_string(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    {
        if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{path}");
        }
        if let Some(path) = path.strip_prefix(r"\\?\") {
            return path.to_owned();
        }
    }
    path.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CITY_DB: &str = "tests/data/test-data/GeoIP2-City-Test.mmdb";

    fn city_identity() -> DatabaseIdentity {
        let path = Path::new(CITY_DB).canonicalize().unwrap();
        identity_for_path(&path).unwrap()
    }

    #[test]
    fn reuses_reader_for_the_same_generation() {
        let identity = city_identity();
        let first = reader_for(&identity).unwrap();
        let second = reader_for(&identity).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn rejects_an_identity_that_does_not_match_the_file() {
        let mut identity = city_identity();
        identity.size += 1;
        let error = reader_for(&identity).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("changed after expression construction")
        );
    }

    #[test]
    fn keeps_replaced_file_generations_isolated() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("database.mmdb");
        fs::copy(CITY_DB, &database).unwrap();
        let old_identity = identity_for_path(&database).unwrap();
        let old_reader = reader_for(&old_identity).unwrap();

        let replacement = directory.path().join("replacement.mmdb");
        fs::copy("tests/data/test-data/GeoLite2-ASN-Test.mmdb", &replacement).unwrap();
        fs::rename(&replacement, &database).unwrap();
        let new_identity = identity_for_path(&database).unwrap();
        let new_reader = reader_for(&new_identity).unwrap();

        assert!(!Arc::ptr_eq(&old_reader, &new_reader));
        assert_eq!(old_reader.metadata().database_type, "GeoIP2-City");
        assert_eq!(new_reader.metadata().database_type, "GeoLite2-ASN");
    }
}

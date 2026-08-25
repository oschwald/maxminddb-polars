use std::collections::HashMap;
use std::collections::VecDeque;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::UNIX_EPOCH;

use maxminddb::Reader;
use polars::prelude::*;
use serde::{Deserialize, Serialize};

use crate::guard::catch_mmdb_unwind;

pub type CachedReader = Reader<Vec<u8>>;

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct DatabaseIdentity {
    pub canonical_path: String,
    pub size: u64,
    pub modified_ns: u64,
    pub changed_ns: i64,
    pub file_id: u64,
}

const DEFAULT_CACHE_MAX_BYTES: usize = 512 * 1024 * 1024;
const CACHE_MAX_BYTES_ENV: &str = "MAXMINDDB_POLARS_CACHE_MAX_BYTES";

#[derive(Default)]
struct ReaderCache {
    readers: HashMap<DatabaseIdentity, Arc<CachedReader>>,
    insertion_order: VecDeque<DatabaseIdentity>,
    bytes: usize,
}

impl ReaderCache {
    fn insert(&mut self, identity: DatabaseIdentity, reader: Arc<CachedReader>, max_bytes: usize) {
        let reader_bytes = usize::try_from(identity.size).unwrap_or(usize::MAX);
        self.bytes = self.bytes.saturating_add(reader_bytes);
        self.insertion_order.push_back(identity.clone());
        self.readers.insert(identity.clone(), reader);

        while self.bytes > max_bytes && self.readers.len() > 1 {
            let Some(oldest) = self.insertion_order.pop_front() else {
                break;
            };
            if oldest == identity {
                self.insertion_order.push_back(oldest);
                break;
            }
            if self.readers.remove(&oldest).is_some() {
                let oldest_bytes = usize::try_from(oldest.size).unwrap_or(usize::MAX);
                self.bytes = self.bytes.saturating_sub(oldest_bytes);
            }
        }
    }
}

static READERS: LazyLock<RwLock<ReaderCache>> =
    LazyLock::new(|| RwLock::new(ReaderCache::default()));

static CACHE_MAX_BYTES: LazyLock<Result<usize, String>> = LazyLock::new(|| {
    let Some(value) = env::var_os(CACHE_MAX_BYTES_ENV) else {
        return Ok(DEFAULT_CACHE_MAX_BYTES);
    };
    let value = value
        .to_str()
        .ok_or_else(|| format!("{CACHE_MAX_BYTES_ENV} must be valid UTF-8"))?;
    value.parse::<usize>().map_err(|error| {
        format!("{CACHE_MAX_BYTES_ENV} must be a non-negative integer number of bytes: {error}")
    })
});

fn cache_max_bytes() -> PolarsResult<usize> {
    CACHE_MAX_BYTES
        .as_ref()
        .copied()
        .map_err(|error| polars_err!(ComputeError: "{error}"))
}

pub fn reader_for(identity: &DatabaseIdentity) -> PolarsResult<Arc<CachedReader>> {
    if let Some(reader) = READERS
        .read()
        .map_err(|_| polars_err!(ComputeError: "MMDB reader cache lock is poisoned"))?
        .readers
        .get(identity)
    {
        return Ok(Arc::clone(reader));
    }

    // Keep the write lock while opening a new generation. Initial opens are
    // rare, and this coalesces concurrent construction of the same snapshot.
    let mut readers = READERS
        .write()
        .map_err(|_| polars_err!(ComputeError: "MMDB reader cache lock is poisoned"))?;
    if let Some(reader) = readers.readers.get(identity) {
        return Ok(Arc::clone(reader));
    }

    let reader = Arc::new(open_snapshot(identity)?);
    readers.insert(identity.clone(), Arc::clone(&reader), cache_max_bytes()?);
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

    let reader = catch_mmdb_unwind(|| Reader::from_source(bytes)).map_err(|()| {
        polars_err!(
            ComputeError:
            "could not open MMDB {} because the parser panicked; the database may be corrupt",
            canonical_path.display()
        )
    })?;
    reader.map_err(|error| {
        polars_err!(
            ComputeError:
            "could not open MMDB {}: {error}",
            canonical_path.display()
        )
    })
}

pub(crate) fn identity_for_path(path: &Path) -> PolarsResult<DatabaseIdentity> {
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
    let changed_ns = metadata_changed_ns(&metadata, path)?;
    let file_id = metadata_file_id(&metadata, path)?;

    Ok(DatabaseIdentity {
        canonical_path: canonical_path_string(path),
        size: metadata.len(),
        modified_ns,
        changed_ns,
        file_id,
    })
}

#[cfg(unix)]
fn metadata_file_id(metadata: &fs::Metadata, _path: &Path) -> PolarsResult<u64> {
    use std::os::unix::fs::MetadataExt;

    Ok(metadata.ino())
}

#[cfg(not(unix))]
fn metadata_file_id(_metadata: &fs::Metadata, _path: &Path) -> PolarsResult<u64> {
    Ok(0)
}

#[cfg(unix)]
fn metadata_changed_ns(metadata: &fs::Metadata, path: &Path) -> PolarsResult<i64> {
    use std::os::unix::fs::MetadataExt;

    let changed_ns = i128::from(metadata.ctime())
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(i128::from(metadata.ctime_nsec())))
        .and_then(|value| i64::try_from(value).ok());
    changed_ns.ok_or_else(|| {
        polars_err!(
            ComputeError:
            "MMDB {} has a metadata change time outside the supported range",
            path.display()
        )
    })
}

#[cfg(not(unix))]
fn metadata_changed_ns(metadata: &fs::Metadata, path: &Path) -> PolarsResult<i64> {
    let changed = metadata.created().map_err(|error| {
        polars_err!(
            ComputeError:
            "could not read creation time for MMDB {}: {error}",
            path.display()
        )
    })?;
    changed
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            polars_err!(
                ComputeError:
                "MMDB {} has a creation time before the Unix epoch: {error}",
                path.display()
            )
        })?
        .as_nanos()
        .try_into()
        .map_err(|_| {
            polars_err!(
                ComputeError:
                "MMDB {} has a creation time outside the supported range",
                path.display()
            )
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
    use std::fs::FileTimes;

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

    #[test]
    fn distinguishes_same_size_and_mtime_replacements() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("database.mmdb");
        fs::copy(CITY_DB, &database).unwrap();
        let old_identity = identity_for_path(&database).unwrap();
        let old_reader = reader_for(&old_identity).unwrap();
        let old_modified = fs::metadata(&database).unwrap().modified().unwrap();

        let replacement = directory.path().join("replacement.mmdb");
        fs::write(&replacement, vec![0; old_identity.size as usize]).unwrap();
        fs::File::open(&replacement)
            .unwrap()
            .set_times(FileTimes::new().set_modified(old_modified))
            .unwrap();
        fs::rename(&replacement, &database).unwrap();

        let new_identity = identity_for_path(&database).unwrap();
        assert_eq!(old_identity.size, new_identity.size);
        assert_eq!(old_identity.modified_ns, new_identity.modified_ns);
        assert_ne!(old_identity.file_id, new_identity.file_id);
        assert!(reader_for(&new_identity).is_err());
        assert_eq!(old_reader.metadata().database_type, "GeoIP2-City");
    }

    #[test]
    fn bounds_cached_reader_bytes_without_dropping_the_newest_reader() {
        let city_identity = city_identity();
        let city = Arc::new(open_snapshot(&city_identity).unwrap());
        let asn_path = Path::new("tests/data/test-data/GeoLite2-ASN-Test.mmdb")
            .canonicalize()
            .unwrap();
        let asn_identity = identity_for_path(&asn_path).unwrap();
        let asn = Arc::new(open_snapshot(&asn_identity).unwrap());
        let mut cache = ReaderCache::default();

        cache.insert(city_identity.clone(), city, 0);
        assert!(cache.readers.contains_key(&city_identity));

        cache.insert(asn_identity.clone(), asn, asn_identity.size as usize);
        assert!(!cache.readers.contains_key(&city_identity));
        assert!(cache.readers.contains_key(&asn_identity));
        assert_eq!(cache.bytes, asn_identity.size as usize);
    }
}

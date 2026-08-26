use std::collections::HashMap;
use std::collections::VecDeque;
use std::env;
use std::ffi::OsStr;
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
    pub volume_id: i64,
    pub file_id: i64,
    pub file_id_high: i64,
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

static CACHE_MAX_BYTES: LazyLock<Result<usize, String>> =
    LazyLock::new(|| parse_cache_max_bytes(env::var_os(CACHE_MAX_BYTES_ENV).as_deref()));

fn parse_cache_max_bytes(value: Option<&OsStr>) -> Result<usize, String> {
    let Some(value) = value else {
        return Ok(DEFAULT_CACHE_MAX_BYTES);
    };
    let value = value
        .to_str()
        .ok_or_else(|| format!("{CACHE_MAX_BYTES_ENV} must be valid UTF-8"))?;
    value.parse::<usize>().map_err(|error| {
        format!("{CACHE_MAX_BYTES_ENV} must be a non-negative integer number of bytes: {error}")
    })
}

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
    let file = fs::File::open(path).map_err(
        |error| polars_err!(ComputeError: "could not open MMDB {}: {error}", path.display()),
    )?;
    let metadata = file.metadata().map_err(
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
    let (volume_id, file_id, file_id_high) = metadata_file_identity(&file, &metadata, path)?;

    Ok(DatabaseIdentity {
        canonical_path: canonical_path_string(path),
        size: metadata.len(),
        modified_ns,
        changed_ns,
        volume_id,
        file_id,
        file_id_high,
    })
}

#[cfg(unix)]
fn metadata_file_identity(
    _file: &fs::File,
    metadata: &fs::Metadata,
    _path: &Path,
) -> PolarsResult<(i64, i64, i64)> {
    use std::os::unix::fs::MetadataExt;

    Ok((signed_bits(metadata.dev()), signed_bits(metadata.ino()), 0))
}

#[cfg(windows)]
fn metadata_file_identity(
    file: &fs::File,
    _metadata: &fs::Metadata,
    path: &Path,
) -> PolarsResult<(i64, i64, i64)> {
    use std::mem::{MaybeUninit, size_of};
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };

    let mut information = MaybeUninit::<FILE_ID_INFO>::uninit();
    // SAFETY: `file` owns a valid handle for the duration of the call, and the
    // output pointer and byte count describe writable `FILE_ID_INFO` storage.
    let succeeded = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            information.as_mut_ptr().cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if succeeded == 0 {
        let error = std::io::Error::last_os_error();
        polars_bail!(
            ComputeError:
            "could not read the Windows file identity for MMDB {}: {error}",
            path.display()
        )
    }
    // SAFETY: a successful call initialized the entire `FILE_ID_INFO` value.
    let information = unsafe { information.assume_init() };
    let [low, high] = information.FileId.Identifier.as_chunks::<8>().0 else {
        unreachable!("Windows file identifiers are 128 bits")
    };
    Ok((
        signed_bits(information.VolumeSerialNumber),
        signed_bits(u64::from_ne_bytes(*low)),
        signed_bits(u64::from_ne_bytes(*high)),
    ))
}

#[cfg(all(not(unix), not(windows)))]
fn metadata_file_identity(
    _file: &fs::File,
    _metadata: &fs::Metadata,
    _path: &Path,
) -> PolarsResult<(i64, i64, i64)> {
    Ok((0, 0, 0))
}

fn signed_bits(value: u64) -> i64 {
    i64::from_ne_bytes(value.to_ne_bytes())
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

    fn test_identity(path: &Path) -> DatabaseIdentity {
        let path = path.canonicalize().unwrap();
        identity_for_path(&path).unwrap()
    }

    fn city_identity() -> DatabaseIdentity {
        test_identity(Path::new(CITY_DB))
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
        let old_identity = test_identity(&database);
        let old_reader = reader_for(&old_identity).unwrap();

        let replacement = directory.path().join("replacement.mmdb");
        fs::copy("tests/data/test-data/GeoLite2-ASN-Test.mmdb", &replacement).unwrap();
        fs::rename(&replacement, &database).unwrap();
        let new_identity = test_identity(&database);
        let new_reader = reader_for(&new_identity).unwrap();

        assert!(!Arc::ptr_eq(&old_reader, &new_reader));
        assert_eq!(old_reader.metadata().database_type, "GeoIP2-City");
        assert_eq!(new_reader.metadata().database_type, "GeoLite2-ASN");
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn distinguishes_same_size_and_mtime_replacements() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("database.mmdb");
        fs::copy(CITY_DB, &database).unwrap();
        let old_identity = test_identity(&database);
        let old_reader = reader_for(&old_identity).unwrap();
        let old_modified = fs::metadata(&database).unwrap().modified().unwrap();

        let replacement = directory.path().join("replacement.mmdb");
        fs::write(&replacement, vec![0; old_identity.size as usize]).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(FileTimes::new().set_modified(old_modified))
            .unwrap();
        fs::rename(&replacement, &database).unwrap();

        let new_identity = test_identity(&database);
        assert_eq!(old_identity.size, new_identity.size);
        assert_eq!(old_identity.modified_ns, new_identity.modified_ns);
        assert_ne!(
            (
                old_identity.volume_id,
                old_identity.file_id,
                old_identity.file_id_high,
            ),
            (
                new_identity.volume_id,
                new_identity.file_id,
                new_identity.file_id_high,
            )
        );
        assert_ne!(old_identity, new_identity);
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
        let asn_identity = test_identity(&asn_path);
        let asn = Arc::new(open_snapshot(&asn_identity).unwrap());
        let mut cache = ReaderCache::default();

        cache.insert(city_identity.clone(), city, 0);
        assert!(cache.readers.contains_key(&city_identity));

        cache.insert(asn_identity.clone(), asn, asn_identity.size as usize);
        assert!(!cache.readers.contains_key(&city_identity));
        assert!(cache.readers.contains_key(&asn_identity));
        assert_eq!(cache.bytes, asn_identity.size as usize);
    }

    #[test]
    fn parses_the_cache_limit_contract() {
        assert_eq!(
            parse_cache_max_bytes(None).unwrap(),
            DEFAULT_CACHE_MAX_BYTES
        );
        assert_eq!(parse_cache_max_bytes(Some(OsStr::new("0"))).unwrap(), 0);
        assert_eq!(
            parse_cache_max_bytes(Some(OsStr::new("1048576"))).unwrap(),
            1_048_576
        );
        for value in ["-1", "not-a-number"] {
            assert!(
                parse_cache_max_bytes(Some(OsStr::new(value)))
                    .unwrap_err()
                    .contains("must be a non-negative integer number of bytes")
            );
        }
    }
}

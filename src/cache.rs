use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, PoisonError, RwLock, Weak};
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

struct CacheEntry {
    reader: Arc<CachedReader>,
    // Updated under the shared lock so cache hits do not need the write lock.
    last_used: AtomicU64,
}

#[derive(Default)]
struct ReaderCache {
    readers: HashMap<DatabaseIdentity, CacheEntry>,
    // Readers evicted while a running evaluation may still hold them. Reusing
    // a live one avoids reading the same file again while it is in memory.
    evicted: HashMap<DatabaseIdentity, Weak<CachedReader>>,
    bytes: usize,
    clock: AtomicU64,
}

impl ReaderCache {
    fn tick(&self) -> u64 {
        self.clock.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Returns a cached reader, or an evicted reader that a running evaluation
    /// still holds. The flag is true for an evicted reader.
    fn get(&self, identity: &DatabaseIdentity) -> Option<(Arc<CachedReader>, bool)> {
        if let Some(entry) = self.readers.get(identity) {
            entry.last_used.fetch_max(self.tick(), Ordering::Relaxed);
            return Some((Arc::clone(&entry.reader), false));
        }
        let reader = self.evicted.get(identity)?.upgrade()?;
        Some((reader, true))
    }

    /// Caches an evicted reader again, unless another lookup already did.
    fn revive(&mut self, identity: &DatabaseIdentity, reader: Arc<CachedReader>, max_bytes: usize) {
        if !self.readers.contains_key(identity) {
            self.insert(identity.clone(), reader, max_bytes);
        }
    }

    fn insert(&mut self, identity: DatabaseIdentity, reader: Arc<CachedReader>, max_bytes: usize) {
        self.evicted.remove(&identity);
        self.evicted.retain(|_, reader| reader.strong_count() > 0);

        let entry = CacheEntry {
            reader,
            last_used: AtomicU64::new(self.tick()),
        };
        if self.readers.insert(identity.clone(), entry).is_none() {
            self.bytes = self.bytes.saturating_add(identity_bytes(&identity));
        }

        if self.bytes <= max_bytes {
            return;
        }
        // Evict the least recently used readers, but always keep this one.
        let mut candidates = self
            .readers
            .iter()
            .filter(|(candidate, _)| **candidate != identity)
            .map(|(candidate, entry)| (entry.last_used.load(Ordering::Relaxed), candidate.clone()))
            .collect::<Vec<_>>();
        candidates.sort_unstable_by_key(|(last_used, _)| *last_used);
        for (_, oldest) in candidates {
            if self.bytes <= max_bytes {
                break;
            }
            if let Some(entry) = self.readers.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(identity_bytes(&oldest));
                if Arc::strong_count(&entry.reader) > 1 {
                    self.evicted.insert(oldest, Arc::downgrade(&entry.reader));
                }
            }
        }
    }
}

fn identity_bytes(identity: &DatabaseIdentity) -> usize {
    usize::try_from(identity.size).unwrap_or(usize::MAX)
}

/// One lock per snapshot being opened, so concurrent lookups share a single
/// read of the file without blocking lookups against other databases.
#[derive(Default)]
struct OpenLocks {
    locks: Mutex<HashMap<DatabaseIdentity, Arc<Mutex<()>>>>,
}

impl OpenLocks {
    fn acquire(&self, identity: &DatabaseIdentity) -> OpenLockHandle<'_> {
        let lock = Arc::clone(
            self.locks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .entry(identity.clone())
                .or_default(),
        );
        OpenLockHandle {
            locks: self,
            identity: identity.clone(),
            lock: Some(lock),
        }
    }
}

struct OpenLockHandle<'a> {
    locks: &'a OpenLocks,
    identity: DatabaseIdentity,
    // Only `Drop` takes this, so it is always set while the handle is in use.
    lock: Option<Arc<Mutex<()>>>,
}

impl OpenLockHandle<'_> {
    fn lock(&self) -> MutexGuard<'_, ()> {
        self.lock
            .as_deref()
            .expect("open lock is set until the handle drops")
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for OpenLockHandle<'_> {
    fn drop(&mut self) {
        let mut locks = self
            .locks
            .locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        // Release this handle's reference under the registry lock, so that
        // handles dropping at the same time cannot each see the other's.
        drop(self.lock.take());
        // Only the map holds a reference, so no other caller is waiting to
        // open this snapshot.
        if locks
            .get(&self.identity)
            .is_some_and(|lock| Arc::strong_count(lock) == 1)
        {
            locks.remove(&self.identity);
        }
    }
}

static READERS: LazyLock<RwLock<ReaderCache>> =
    LazyLock::new(|| RwLock::new(ReaderCache::default()));

static OPEN_LOCKS: LazyLock<OpenLocks> = LazyLock::new(OpenLocks::default);

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
    reader_from(
        &READERS,
        &OPEN_LOCKS,
        identity,
        cache_max_bytes,
        open_snapshot,
    )
}

fn reader_from(
    cache: &RwLock<ReaderCache>,
    open_locks: &OpenLocks,
    identity: &DatabaseIdentity,
    max_bytes: impl FnOnce() -> PolarsResult<usize>,
    open: impl FnOnce(&DatabaseIdentity) -> PolarsResult<CachedReader>,
) -> PolarsResult<Arc<CachedReader>> {
    let poisoned = || polars_err!(ComputeError: "MMDB reader cache lock is poisoned");
    let max_bytes = max_bytes()?;
    let found = cache.read().map_err(|_| poisoned())?.get(identity);
    if let Some((reader, evicted)) = found {
        if evicted {
            // Cache the reader again if that does not mean waiting for the
            // lock. Either way, the lookup uses the reader already in memory.
            if let Ok(mut cache) = cache.try_write() {
                cache.revive(identity, Arc::clone(&reader), max_bytes);
            }
        }
        return Ok(reader);
    }

    // Only one caller opens a snapshot. Others wait here, then find it cached.
    let handle = open_locks.acquire(identity);
    let _open_guard = handle.lock();
    {
        let mut cache = cache.write().map_err(|_| poisoned())?;
        if let Some((reader, evicted)) = cache.get(identity) {
            if evicted {
                cache.revive(identity, Arc::clone(&reader), max_bytes);
            }
            return Ok(reader);
        }
    }

    // Read and parse the file without the cache lock, so lookups against
    // other databases do not wait.
    let reader = Arc::new(open(identity)?);
    cache
        .write()
        .map_err(|_| poisoned())?
        .insert(identity.clone(), Arc::clone(&reader), max_bytes);
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

    const ASN_DB: &str = "tests/data/test-data/GeoLite2-ASN-Test.mmdb";
    const COUNTRY_DB: &str = "tests/data/test-data/GeoIP2-Country-Test.mmdb";

    fn opened(identity: &DatabaseIdentity) -> Arc<CachedReader> {
        Arc::new(open_snapshot(identity).unwrap())
    }

    fn bytes(identity: &DatabaseIdentity) -> usize {
        identity_bytes(identity)
    }

    #[test]
    fn evicts_the_least_recently_used_reader() {
        let city = city_identity();
        let asn = test_identity(Path::new(ASN_DB));
        let country = test_identity(Path::new(COUNTRY_DB));
        let max_bytes = bytes(&city) + bytes(&asn).max(bytes(&country));
        let mut cache = ReaderCache::default();

        cache.insert(city.clone(), opened(&city), max_bytes);
        cache.insert(asn.clone(), opened(&asn), max_bytes);
        // City was inserted first but used last, so ASN is evicted.
        assert!(cache.get(&city).is_some());
        cache.insert(country.clone(), opened(&country), max_bytes);

        assert!(cache.readers.contains_key(&city));
        assert!(!cache.readers.contains_key(&asn));
        assert!(cache.readers.contains_key(&country));
        assert_eq!(cache.bytes, bytes(&city) + bytes(&country));
    }

    #[test]
    fn revives_an_evicted_reader_while_it_is_still_held() {
        let city = city_identity();
        let asn = test_identity(Path::new(ASN_DB));
        let mut cache = ReaderCache::default();

        let held_city = opened(&city);
        cache.insert(city.clone(), Arc::clone(&held_city), 0);
        cache.insert(asn.clone(), opened(&asn), 0);
        assert!(!cache.readers.contains_key(&city));

        let (revived, evicted) = cache.get(&city).unwrap();
        assert!(evicted);
        assert!(Arc::ptr_eq(&held_city, &revived));
        cache.revive(&city, revived, 0);
        assert!(cache.readers.contains_key(&city));
        assert!(!cache.readers.contains_key(&asn));
        assert_eq!(cache.bytes, bytes(&city));

        // ASN was not held, so it cannot be revived.
        assert!(cache.get(&asn).is_none());
        assert!(!cache.evicted.contains_key(&asn));
    }

    #[test]
    fn forgets_evicted_readers_once_they_are_dropped() {
        let city = city_identity();
        let asn = test_identity(Path::new(ASN_DB));
        let country = test_identity(Path::new(COUNTRY_DB));
        let mut cache = ReaderCache::default();

        let held_city = opened(&city);
        cache.insert(city.clone(), Arc::clone(&held_city), 0);
        cache.insert(asn.clone(), opened(&asn), 0);
        assert!(cache.evicted.contains_key(&city));

        drop(held_city);
        cache.insert(country.clone(), opened(&country), 0);
        assert!(cache.evicted.is_empty());
        assert!(cache.get(&city).is_none());
    }

    fn counting_open(
        opens: &AtomicU64,
    ) -> impl FnOnce(&DatabaseIdentity) -> PolarsResult<CachedReader> + '_ {
        move |identity| {
            opens.fetch_add(1, Ordering::SeqCst);
            open_snapshot(identity)
        }
    }

    #[test]
    fn alternating_lookups_over_the_limit_reuse_held_readers() {
        let cache = RwLock::new(ReaderCache::default());
        let open_locks = OpenLocks::default();
        let city = city_identity();
        let asn = test_identity(Path::new(ASN_DB));
        let city_opens = AtomicU64::new(0);
        let asn_opens = AtomicU64::new(0);

        // Each lookup evicts the other database, as when two databases in one
        // query exceed the limit. Overlapping batches still hold both readers.
        let mut held = Vec::new();
        for _ in 0..5 {
            held.push(
                reader_from(
                    &cache,
                    &open_locks,
                    &city,
                    || Ok(1),
                    counting_open(&city_opens),
                )
                .unwrap(),
            );
            held.push(
                reader_from(
                    &cache,
                    &open_locks,
                    &asn,
                    || Ok(1),
                    counting_open(&asn_opens),
                )
                .unwrap(),
            );
        }

        assert_eq!(city_opens.load(Ordering::SeqCst), 1);
        assert_eq!(asn_opens.load(Ordering::SeqCst), 1);
        assert_eq!(cache.read().unwrap().readers.len(), 1);
        assert!(open_locks.locks.lock().unwrap().is_empty());
    }

    #[test]
    fn coalesces_concurrent_opens_of_the_same_snapshot() {
        let cache = RwLock::new(ReaderCache::default());
        let open_locks = OpenLocks::default();
        let city = city_identity();
        let opens = AtomicU64::new(0);
        let barrier = std::sync::Barrier::new(4);

        let readers = std::thread::scope(|scope| {
            let threads = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        barrier.wait();
                        reader_from(
                            &cache,
                            &open_locks,
                            &city,
                            || Ok(usize::MAX),
                            |identity| {
                                opens.fetch_add(1, Ordering::SeqCst);
                                std::thread::sleep(std::time::Duration::from_millis(50));
                                open_snapshot(identity)
                            },
                        )
                        .unwrap()
                    })
                })
                .collect::<Vec<_>>();
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });

        assert_eq!(opens.load(Ordering::SeqCst), 1);
        assert!(
            readers
                .iter()
                .all(|reader| Arc::ptr_eq(reader, &readers[0]))
        );
        assert!(open_locks.locks.lock().unwrap().is_empty());
    }

    #[test]
    fn opens_files_without_blocking_other_cached_databases() {
        let cache = RwLock::new(ReaderCache::default());
        let open_locks = OpenLocks::default();
        let city = city_identity();
        let asn = test_identity(Path::new(ASN_DB));
        let cached_asn =
            reader_from(&cache, &open_locks, &asn, || Ok(usize::MAX), open_snapshot).unwrap();
        let (opening_sender, opening) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();

        let (cache, open_locks, city) = (&cache, &open_locks, &city);
        std::thread::scope(|scope| {
            let city_thread = scope.spawn(move || {
                reader_from(
                    cache,
                    open_locks,
                    city,
                    || Ok(usize::MAX),
                    |identity| {
                        opening_sender.send(()).unwrap();
                        released.recv().unwrap();
                        open_snapshot(identity)
                    },
                )
                .unwrap()
            });
            opening.recv().unwrap();

            // The city file is still being opened. A cache hit for another
            // database must not wait for it.
            let (done_sender, done) = std::sync::mpsc::channel();
            let asn = &asn;
            scope.spawn(move || {
                let reader = reader_from(
                    cache,
                    open_locks,
                    asn,
                    || Ok(usize::MAX),
                    |_| panic!("ASN is cached"),
                )
                .unwrap();
                done_sender.send(reader).unwrap();
            });
            let reader = done.recv_timeout(std::time::Duration::from_secs(10));
            release.send(()).unwrap();
            assert!(Arc::ptr_eq(&reader.unwrap(), &cached_asn));
            city_thread.join().unwrap();
        });
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

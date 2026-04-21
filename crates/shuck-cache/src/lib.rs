#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! File-oriented cache keys and persistent package caches for Shuck.
//!
//! The types in this crate power the `shuck` CLI cache, but are generic enough to reuse in other
//! Rust tooling that wants SHA-256-based cache partitioning and serialized per-file entries.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};
use tempfile::NamedTempFile;

/// Legacy per-project cache directory name used by older shuck releases.
pub const CACHE_DIR_NAME: &str = ".shuck_cache";

const MAX_LAST_SEEN_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Returns the legacy cache directory that lives under a project root.
pub fn legacy_cache_dir(project_root: &Path) -> PathBuf {
    project_root.join(CACHE_DIR_NAME)
}

/// Reads the cached project root marker stored in a legacy cache file.
pub fn read_project_root_from_cache_file(path: &Path) -> io::Result<Option<PathBuf>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };

    let mut reader = BufReader::new(file);
    match bincode::serde::decode_from_std_read(&mut reader, bincode::config::standard()) {
        Ok(project_root) => Ok(Some(project_root)),
        Err(_) => Ok(None),
    }
}

/// Trait for values that can contribute to a deterministic package cache key.
#[allow(missing_docs)]
pub trait CacheKey {
    fn cache_key(&self, state: &mut CacheKeyHasher);
}

/// Incremental hasher used to build structured cache keys.
pub struct CacheKeyHasher {
    hasher: Sha256,
}

#[allow(missing_docs)]
impl CacheKeyHasher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            hasher: Sha256::new(),
        }
    }

    pub fn write_tag(&mut self, tag: &[u8]) {
        self.write_bytes(tag);
    }

    pub fn write_bool(&mut self, value: bool) {
        self.hasher.update([u8::from(value)]);
    }

    pub fn write_u8(&mut self, value: u8) {
        self.hasher.update([value]);
    }

    pub fn write_u32(&mut self, value: u32) {
        self.hasher.update(value.to_le_bytes());
    }

    pub fn write_u64(&mut self, value: u64) {
        self.hasher.update(value.to_le_bytes());
    }

    pub fn write_u128(&mut self, value: u128) {
        self.hasher.update(value.to_le_bytes());
    }

    pub fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    pub fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }

    pub fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_u64(bytes.len() as u64);
        self.hasher.update(bytes);
    }

    #[must_use]
    pub fn finish_hex(self) -> String {
        let digest = self.hasher.finalize();
        let mut out = String::with_capacity(digest.len() * 2);
        for byte in digest {
            use std::fmt::Write as _;
            let _ = write!(&mut out, "{byte:02x}");
        }
        out
    }
}

impl Default for CacheKeyHasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Returns the hex-encoded cache key for a value.
#[must_use]
pub fn cache_key_hex<T: CacheKey>(value: &T) -> String {
    let mut hasher = CacheKeyHasher::new();
    value.cache_key(&mut hasher);
    hasher.finish_hex()
}

impl CacheKey for bool {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_bool(*self);
    }
}

impl CacheKey for u8 {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_u8(*self);
    }
}

impl CacheKey for u32 {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_u32(*self);
    }
}

impl CacheKey for u64 {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_u64(*self);
    }
}

impl CacheKey for u128 {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_u128(*self);
    }
}

impl CacheKey for usize {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_usize(*self);
    }
}

impl CacheKey for str {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_str(self);
    }
}

impl CacheKey for String {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.as_str().cache_key(state);
    }
}

impl<T: CacheKey + ?Sized> CacheKey for &T {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        (**self).cache_key(state);
    }
}

impl<T: CacheKey> CacheKey for Option<T> {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        match self {
            Some(value) => {
                state.write_u8(1);
                value.cache_key(state);
            }
            None => state.write_u8(0),
        }
    }
}

impl<T: CacheKey> CacheKey for [T] {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_usize(self.len());
        for value in self {
            value.cache_key(state);
        }
    }
}

impl<T: CacheKey> CacheKey for Vec<T> {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.as_slice().cache_key(state);
    }
}

impl CacheKey for Path {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_str(&self.to_string_lossy());
    }
}

impl CacheKey for PathBuf {
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        self.as_path().cache_key(state);
    }
}

impl<K, V> CacheKey for BTreeMap<K, V>
where
    K: CacheKey + Ord,
    V: CacheKey,
{
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_usize(self.len());
        for (key, value) in self {
            key.cache_key(state);
            value.cache_key(state);
        }
    }
}

/// File metadata used to validate cached entries against a filesystem path.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FileCacheKey {
    pub file_last_modified_ms: u128,
    pub file_permissions_mode: u32,
    pub file_size_bytes: u64,
}

#[allow(missing_docs)]
impl FileCacheKey {
    pub fn from_path(path: &Path) -> io::Result<Self> {
        let metadata = path.metadata()?;
        let file_last_modified_ms = metadata
            .modified()
            .and_then(|modified| {
                modified
                    .duration_since(UNIX_EPOCH)
                    .map_err(io::Error::other)
            })?
            .as_millis();

        #[cfg(unix)]
        let file_permissions_mode = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode()
        };

        #[cfg(windows)]
        let file_permissions_mode: u32 = u32::from(metadata.permissions().readonly());

        Ok(Self {
            file_last_modified_ms,
            file_permissions_mode,
            file_size_bytes: metadata.len(),
        })
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CachedFile<T> {
    key: FileCacheKey,
    last_seen_ms: u64,
    data: T,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoredPackage<T> {
    project_root: PathBuf,
    files: BTreeMap<PathBuf, CachedFile<T>>,
}

#[derive(Debug, Clone)]
struct Change<T> {
    key: FileCacheKey,
    data: T,
}

/// On-disk cache for file-scoped analysis results within a package.
#[derive(Debug, Clone)]
pub struct PackageCache<T> {
    path: PathBuf,
    package: StoredPackage<T>,
    seen_paths: BTreeSet<PathBuf>,
    changes: BTreeMap<PathBuf, Change<T>>,
    last_seen_ms: u64,
}

#[allow(missing_docs)]
impl<T> PackageCache<T>
where
    T: Clone + Serialize + DeserializeOwned,
{
    pub fn open(
        cache_root: &Path,
        canonical_root: PathBuf,
        tool_version: &str,
        package_key: &impl CacheKey,
    ) -> io::Result<Self> {
        let key = cache_key_hex(package_key);
        let path = cache_root.join(tool_version).join(format!("{key}.bin"));

        let file = match File::open(&path) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::empty(path, canonical_root));
            }
            Err(err) => return Err(err),
        };

        let mut reader = BufReader::new(file);
        let package: StoredPackage<T> =
            match bincode::serde::decode_from_std_read(&mut reader, bincode::config::standard()) {
                Ok(package) => package,
                Err(_) => return Ok(Self::empty(path, canonical_root)),
            };

        if package.project_root != canonical_root {
            return Ok(Self::empty(path, canonical_root));
        }

        Ok(Self {
            path,
            package,
            seen_paths: BTreeSet::new(),
            changes: BTreeMap::new(),
            last_seen_ms: current_time_ms(),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&mut self, relative_path: &Path, key: &FileCacheKey) -> Option<T> {
        let file = self.package.files.get(relative_path)?;
        if &file.key != key {
            return None;
        }

        self.seen_paths.insert(relative_path.to_path_buf());
        Some(file.data.clone())
    }

    pub fn insert(&mut self, relative_path: PathBuf, key: FileCacheKey, data: T) {
        self.seen_paths.insert(relative_path.clone());
        self.changes.insert(relative_path, Change { key, data });
    }

    pub fn persist(mut self) -> io::Result<()> {
        if !self.save() {
            return Ok(());
        }

        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::other("cache path has no parent directory"))?;
        fs::create_dir_all(parent)?;

        let mut temp_file = NamedTempFile::new_in(parent)?;
        let encoded = bincode::serde::encode_to_vec(&self.package, bincode::config::standard())
            .map_err(io::Error::other)?;
        temp_file.write_all(&encoded)?;

        match temp_file.persist(&self.path) {
            Ok(_) => Ok(()),
            Err(err) => Err(err.error),
        }
    }

    fn empty(path: PathBuf, canonical_root: PathBuf) -> Self {
        Self {
            path,
            package: StoredPackage {
                project_root: canonical_root,
                files: BTreeMap::new(),
            },
            seen_paths: BTreeSet::new(),
            changes: BTreeMap::new(),
            last_seen_ms: current_time_ms(),
        }
    }

    fn save(&mut self) -> bool {
        if self.seen_paths.is_empty() && self.changes.is_empty() {
            return false;
        }

        let max_age_ms = MAX_LAST_SEEN_AGE.as_millis() as u64;
        let now = self.last_seen_ms;

        self.package
            .files
            .retain(|_, file| now.saturating_sub(file.last_seen_ms) <= max_age_ms);

        for path in &self.seen_paths {
            if let Some(change) = self.changes.remove(path) {
                self.package.files.insert(
                    path.clone(),
                    CachedFile {
                        key: change.key,
                        last_seen_ms: now,
                        data: change.data,
                    },
                );
            } else if let Some(existing) = self.package.files.get_mut(path) {
                existing.last_seen_ms = now;
            }
        }

        for (path, change) in std::mem::take(&mut self.changes) {
            self.package.files.insert(
                path,
                CachedFile {
                    key: change.key,
                    last_seen_ms: now,
                    data: change.data,
                },
            );
        }

        true
    }
}

fn current_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestSettings {
        strict: bool,
        label: String,
    }

    impl CacheKey for TestSettings {
        fn cache_key(&self, state: &mut CacheKeyHasher) {
            state.write_tag(b"test-settings");
            self.strict.cache_key(state);
            self.label.cache_key(state);
        }
    }

    #[test]
    fn cache_key_hashing_is_deterministic() {
        let settings = TestSettings {
            strict: true,
            label: "alpha".to_string(),
        };

        let first = cache_key_hex(&settings);
        let second = cache_key_hex(&settings);

        assert_eq!(first, second);
    }

    #[test]
    fn cache_key_changes_when_settings_change() {
        let first = TestSettings {
            strict: true,
            label: "alpha".to_string(),
        };
        let second = TestSettings {
            strict: false,
            label: "alpha".to_string(),
        };

        assert_ne!(cache_key_hex(&first), cache_key_hex(&second));
    }

    #[test]
    fn package_cache_persists_and_reloads() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_root = tempdir.path().join("cache");
        let storage_root = tempdir.path().join("project");
        fs::create_dir_all(&storage_root).unwrap();
        let canonical_root = fs::canonicalize(&storage_root).unwrap();

        let settings = TestSettings {
            strict: true,
            label: "alpha".to_string(),
        };

        let mut cache =
            PackageCache::<String>::open(&cache_root, canonical_root.clone(), "0.1.0", &settings)
                .unwrap();
        cache.insert(
            PathBuf::from("script.sh"),
            FileCacheKey {
                file_last_modified_ms: 1,
                file_permissions_mode: 0o644,
                file_size_bytes: 2,
            },
            "ok".to_string(),
        );
        let cache_path = cache.path().to_path_buf();
        cache.persist().unwrap();

        assert!(cache_path.is_file());

        let mut reopened =
            PackageCache::<String>::open(&cache_root, canonical_root, "0.1.0", &settings).unwrap();
        let value = reopened.get(
            Path::new("script.sh"),
            &FileCacheKey {
                file_last_modified_ms: 1,
                file_permissions_mode: 0o644,
                file_size_bytes: 2,
            },
        );

        assert_eq!(value.as_deref(), Some("ok"));
    }

    #[test]
    fn persist_prunes_stale_entries() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_root = tempdir.path().join("cache");
        let storage_root = tempdir.path().join("project");
        fs::create_dir_all(&storage_root).unwrap();
        let canonical_root = fs::canonicalize(&storage_root).unwrap();
        let settings = TestSettings {
            strict: true,
            label: "alpha".to_string(),
        };

        let mut cache =
            PackageCache::<String>::open(&cache_root, canonical_root.clone(), "0.1.0", &settings)
                .unwrap();
        cache.insert(
            PathBuf::from("stale.sh"),
            FileCacheKey {
                file_last_modified_ms: 1,
                file_permissions_mode: 0o644,
                file_size_bytes: 5,
            },
            "stale".to_string(),
        );
        let cache_path = cache.path().to_path_buf();
        cache.persist().unwrap();

        let mut stored: StoredPackage<String> = {
            let mut reader = BufReader::new(File::open(&cache_path).unwrap());
            bincode::serde::decode_from_std_read(&mut reader, bincode::config::standard()).unwrap()
        };
        stored
            .files
            .get_mut(Path::new("stale.sh"))
            .unwrap()
            .last_seen_ms = 0;
        let encoded = bincode::serde::encode_to_vec(&stored, bincode::config::standard()).unwrap();
        fs::write(&cache_path, encoded).unwrap();

        let mut reopened =
            PackageCache::<String>::open(&cache_root, canonical_root, "0.1.0", &settings).unwrap();
        reopened.insert(
            PathBuf::from("fresh.sh"),
            FileCacheKey {
                file_last_modified_ms: 2,
                file_permissions_mode: 0o644,
                file_size_bytes: 5,
            },
            "fresh".to_string(),
        );
        reopened.persist().unwrap();

        let mut reader = BufReader::new(File::open(&cache_path).unwrap());
        let stored: StoredPackage<String> =
            bincode::serde::decode_from_std_read(&mut reader, bincode::config::standard()).unwrap();

        assert!(!stored.files.contains_key(Path::new("stale.sh")));
        assert!(stored.files.contains_key(Path::new("fresh.sh")));
    }

    #[test]
    fn cache_key_miss_when_only_file_size_changes() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_root = tempdir.path().join("cache");
        let storage_root = tempdir.path().join("project");
        fs::create_dir_all(&storage_root).unwrap();
        let canonical_root = fs::canonicalize(&storage_root).unwrap();
        let settings = TestSettings {
            strict: true,
            label: "alpha".to_string(),
        };

        let mut cache =
            PackageCache::<String>::open(&cache_root, canonical_root.clone(), "0.1.0", &settings)
                .unwrap();
        cache.insert(
            PathBuf::from("script.sh"),
            FileCacheKey {
                file_last_modified_ms: 1,
                file_permissions_mode: 0o644,
                file_size_bytes: 2,
            },
            "ok".to_string(),
        );
        cache.persist().unwrap();

        let mut reopened =
            PackageCache::<String>::open(&cache_root, canonical_root, "0.1.0", &settings).unwrap();
        let value = reopened.get(
            Path::new("script.sh"),
            &FileCacheKey {
                file_last_modified_ms: 1,
                file_permissions_mode: 0o644,
                file_size_bytes: 3,
            },
        );

        assert!(value.is_none());
    }

    #[test]
    fn reads_project_root_from_cache_file_without_knowing_payload_type() {
        let tempdir = tempfile::tempdir().unwrap();
        let cache_root = tempdir.path().join("cache");
        let storage_root = tempdir.path().join("project");
        fs::create_dir_all(&storage_root).unwrap();
        let canonical_root = fs::canonicalize(&storage_root).unwrap();
        let settings = TestSettings {
            strict: true,
            label: "alpha".to_string(),
        };

        let mut cache =
            PackageCache::<String>::open(&cache_root, canonical_root.clone(), "0.1.0", &settings)
                .unwrap();
        cache.insert(
            PathBuf::from("script.sh"),
            FileCacheKey {
                file_last_modified_ms: 1,
                file_permissions_mode: 0o644,
                file_size_bytes: 2,
            },
            "ok".to_string(),
        );
        let cache_path = cache.path().to_path_buf();
        cache.persist().unwrap();

        let project_root = read_project_root_from_cache_file(&cache_path).unwrap();
        assert_eq!(project_root, Some(canonical_root));
    }
}

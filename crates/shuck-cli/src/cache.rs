use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use etcetera::BaseStrategy;

use crate::discover::normalize_path;

pub(crate) fn resolve_cache_root(cwd: &Path, override_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = override_path {
        let resolved = if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        };
        return Ok(normalize_path(&resolved));
    }

    default_cache_root()
}

#[cfg(not(target_arch = "wasm32"))]
fn default_cache_root() -> Result<PathBuf> {
    let strategy = etcetera::base_strategy::choose_base_strategy()
        .context("resolve the OS cache directory for shuck")?;
    Ok(strategy.cache_dir().join("shuck"))
}

#[cfg(target_arch = "wasm32")]
fn default_cache_root() -> Result<PathBuf> {
    anyhow::bail!("cache directory discovery is not supported on wasm32")
}

/// Directory under the cache root that holds this release's cache files.
pub(crate) const CACHE_VERSION_DIR: &str = env!("CARGO_PKG_VERSION");

/// Bump whenever a cached data layout changes, so entries written by an older
/// layout are never decoded as the new one.
const CACHE_SCHEMA_VERSION: u32 = 1;

/// Identifies the running build for cache keys.
///
/// The package version alone is not enough: development builds and builds
/// installed from git share a version while carrying different rule logic and
/// cache layouts. Mixing in the executable's size and modification time gives
/// every distinct binary its own cache entries, at the cost of one cold run
/// after an upgrade or rebuild.
pub(crate) fn build_fingerprint() -> &'static str {
    static FINGERPRINT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    FINGERPRINT.get_or_init(|| {
        let executable = std::env::current_exe()
            .and_then(|path| path.metadata())
            .ok()
            .map(|metadata| {
                let modified_ns = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |duration| duration.as_nanos());
                format!("{}:{modified_ns}", metadata.len())
            })
            .unwrap_or_else(|| "unknown".to_owned());
        format!(
            "version={};schema={CACHE_SCHEMA_VERSION};exe={executable}",
            env!("CARGO_PKG_VERSION")
        )
    })
}

/// Returns whether a directory name under the cache root looks like a shuck
/// version directory (`MAJOR.MINOR.PATCH` with an optional suffix).
pub(crate) fn is_cache_version_dir_name(name: &str) -> bool {
    let core = name.split(['-', '+']).next().unwrap_or_default();
    let parts = core.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Returns whether a file name looks like a shuck package cache file
/// (a SHA-256 hex digest with a `.bin` extension).
pub(crate) fn is_cache_file_name(name: &str) -> bool {
    name.strip_suffix(".bin").is_some_and(|stem| {
        stem.len() == 64
            && stem
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_fingerprint_names_version_and_schema() {
        let fingerprint = build_fingerprint();
        assert!(fingerprint.contains(env!("CARGO_PKG_VERSION")));
        assert!(fingerprint.contains(&format!("schema={CACHE_SCHEMA_VERSION}")));
        assert_eq!(fingerprint, build_fingerprint());
    }

    #[test]
    fn recognizes_only_cache_shaped_names() {
        assert!(is_cache_version_dir_name("0.2.5"));
        assert!(is_cache_version_dir_name("1.10.0-rc.1"));
        assert!(!is_cache_version_dir_name("photos"));
        assert!(!is_cache_version_dir_name("1.2"));
        assert!(is_cache_file_name(&format!("{}.bin", "a".repeat(64))));
        assert!(!is_cache_file_name(&format!("{}.bin", "A".repeat(64))));
        assert!(!is_cache_file_name("notes.bin"));
        assert!(!is_cache_file_name(&"a".repeat(64)));
    }
}

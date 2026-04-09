use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Serialize, de::DeserializeOwned};
use shuck_cache::{CacheKey, CacheKeyHasher, FileCacheKey, PackageCache};

use crate::discover::{DiscoveredFile, DiscoveryOptions, ProjectRoot, discover_files};

pub(crate) struct ProjectRun<T, S>
where
    T: Clone + Serialize + DeserializeOwned,
{
    #[allow(dead_code)]
    pub project_root: ProjectRoot,
    pub files: Vec<DiscoveredFile>,
    pub settings: S,
    pub cache: Option<PackageCache<T>>,
}

#[derive(Debug, Clone)]
pub(crate) struct PendingProjectFile {
    pub file: DiscoveredFile,
    pub file_key: FileCacheKey,
}

#[derive(Debug, Clone)]
struct ProjectCacheKey<S> {
    cache_tag: &'static [u8],
    canonical_project_root: PathBuf,
    settings: S,
}

impl<S> CacheKey for ProjectCacheKey<S>
where
    S: CacheKey,
{
    fn cache_key(&self, state: &mut CacheKeyHasher) {
        state.write_tag(self.cache_tag);
        self.canonical_project_root.cache_key(state);
        self.settings.cache_key(state);
    }
}

pub(crate) fn prepare_project_runs<T, S, F>(
    inputs: &[PathBuf],
    cwd: &Path,
    discovery_options: &DiscoveryOptions,
    cache_root: &Path,
    no_cache: bool,
    cache_tag: &'static [u8],
    mut resolve_settings: F,
) -> Result<Vec<ProjectRun<T, S>>>
where
    T: Clone + Serialize + DeserializeOwned,
    S: CacheKey + Clone,
    F: FnMut(&ProjectRoot) -> Result<S>,
{
    let files = discover_files(inputs, cwd, discovery_options)?;
    let mut groups: BTreeMap<ProjectRoot, Vec<DiscoveredFile>> = BTreeMap::new();
    for file in files {
        groups
            .entry(file.project_root.clone())
            .or_default()
            .push(file);
    }

    let mut runs = Vec::new();
    for (project_root, files) in groups {
        let settings = resolve_settings(&project_root)?;
        let cache = if no_cache {
            None
        } else {
            Some(PackageCache::<T>::open(
                cache_root,
                project_root.canonical_root.clone(),
                env!("CARGO_PKG_VERSION"),
                &ProjectCacheKey {
                    cache_tag,
                    canonical_project_root: project_root.canonical_root.clone(),
                    settings: settings.clone(),
                },
            )?)
        };

        runs.push(ProjectRun {
            project_root,
            files,
            settings,
            cache,
        });
    }

    Ok(runs)
}

impl<T, S> ProjectRun<T, S>
where
    T: Clone + Serialize + DeserializeOwned,
{
    pub(crate) fn take_pending_files<F>(&mut self, mut on_hit: F) -> Result<Vec<PendingProjectFile>>
    where
        F: FnMut(DiscoveredFile, T) -> Result<()>,
    {
        let mut pending = Vec::new();

        for file in self.files.drain(..) {
            let file_key = FileCacheKey::from_path(&file.absolute_path)?;
            if let Some(cache) = self.cache.as_mut()
                && let Some(cached) = cache.get(&file.relative_path, &file_key)
            {
                on_hit(file, cached)?;
                continue;
            }

            pending.push(PendingProjectFile { file, file_key });
        }

        Ok(pending)
    }

    pub(crate) fn persist_cache(self) -> Result<()> {
        if let Some(cache) = self.cache {
            cache.persist()?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde::{Deserialize, Serialize};
    use tempfile::tempdir;

    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct DummyCacheData {
        name: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct DummySettings;

    impl CacheKey for DummySettings {
        fn cache_key(&self, state: &mut CacheKeyHasher) {
            state.write_tag(b"dummy-settings");
        }
    }

    fn cache_root(root: &Path) -> PathBuf {
        root.join("cache")
    }

    fn discovery_options(cache_root: &Path) -> DiscoveryOptions {
        DiscoveryOptions {
            parallel: false,
            cache_root: Some(cache_root.to_path_buf()),
            ..DiscoveryOptions::default()
        }
    }

    #[test]
    fn groups_discovered_files_by_project_root() {
        let tempdir = tempdir().unwrap();
        let nested = tempdir.path().join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(tempdir.path().join("shuck.toml"), "[format]\n").unwrap();
        fs::write(nested.join("shuck.toml"), "[format]\n").unwrap();
        fs::write(tempdir.path().join("root.sh"), "#!/bin/bash\necho root\n").unwrap();
        fs::write(nested.join("nested.sh"), "#!/bin/bash\necho nested\n").unwrap();

        let runs = prepare_project_runs::<DummyCacheData, DummySettings, _>(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &discovery_options(&cache_root(tempdir.path())),
            &cache_root(tempdir.path()),
            true,
            b"dummy-cache",
            |_| Ok(DummySettings),
        )
        .unwrap();

        assert_eq!(runs.len(), 2);
        let mut display_paths = runs
            .iter()
            .map(|run| run.files[0].display_path.clone())
            .collect::<Vec<_>>();
        display_paths.sort();
        assert_eq!(
            display_paths,
            vec![PathBuf::from("nested/nested.sh"), PathBuf::from("root.sh")]
        );
    }

    #[test]
    fn cached_hits_are_reported_and_uncached_files_stay_pending() {
        let tempdir = tempdir().unwrap();
        fs::write(tempdir.path().join("one.sh"), "#!/bin/bash\necho one\n").unwrap();
        fs::write(tempdir.path().join("two.sh"), "#!/bin/bash\necho two\n").unwrap();

        let cache_root = cache_root(tempdir.path());
        let mut runs = prepare_project_runs::<DummyCacheData, DummySettings, _>(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &discovery_options(&cache_root),
            &cache_root,
            false,
            b"dummy-cache",
            |_| Ok(DummySettings),
        )
        .unwrap();
        let mut run = runs.pop().unwrap();
        let mut pending = run.take_pending_files(|_, _| unreachable!()).unwrap();
        assert_eq!(pending.len(), 2);

        let cached = pending.remove(0);
        run.cache.as_mut().unwrap().insert(
            cached.file.relative_path.clone(),
            cached.file_key.clone(),
            DummyCacheData {
                name: cached.file.display_path.display().to_string(),
            },
        );
        run.persist_cache().unwrap();

        let mut runs = prepare_project_runs::<DummyCacheData, DummySettings, _>(
            &[tempdir.path().to_path_buf()],
            tempdir.path(),
            &discovery_options(&cache_root),
            &cache_root,
            false,
            b"dummy-cache",
            |_| Ok(DummySettings),
        )
        .unwrap();
        let mut run = runs.pop().unwrap();

        let mut cached_hits = Vec::new();
        let pending = run
            .take_pending_files(|file, cached| {
                cached_hits.push((file.display_path, cached.name));
                Ok(())
            })
            .unwrap();

        assert_eq!(cached_hits.len(), 1);
        assert_eq!(pending.len(), 1);
    }
}

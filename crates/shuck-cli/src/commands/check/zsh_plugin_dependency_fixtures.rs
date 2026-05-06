use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;
use shuck_config::ConfigArguments;
use tempfile::tempdir;

use super::run::run_check_with_cwd;
use super::test_support::{cache_root, check_args};
use super::watch::collect_watch_targets;
use crate::discover::normalize_path;

#[derive(Debug, Deserialize)]
struct Fixture {
    name: String,
    path: String,
    #[serde(default)]
    files: Vec<FixtureFile>,
    #[serde(default)]
    after_files: Vec<FixtureFile>,
}

#[derive(Debug, Deserialize)]
struct FixtureFile {
    path: String,
    #[serde(default)]
    contents: String,
}

#[test]
fn zsh_plugin_dependency_fixtures_record_missing_paths_and_invalidate_cache() -> Result<()> {
    for fixture_path in fixture_paths()? {
        let fixture = load_fixture(&fixture_path)?;
        let tempdir = tempdir().context("failed to create temp dir")?;
        materialize_files(tempdir.path(), &fixture.files)?;

        let expected_dependencies = fixture_dependency_paths(tempdir.path(), &fixture);
        let mut args = check_args(false);
        args.paths = vec![PathBuf::from(&fixture.path)];

        let first = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .with_context(|| format!("fixture `{}` initial run failed", fixture.name))?;
        assert_eq!(
            first.cache_hits, 0,
            "fixture `{}` should miss cache on first run",
            fixture.name
        );
        assert_eq!(
            first.cache_misses, 1,
            "fixture `{}` should analyze its input on first run",
            fixture.name
        );
        assert_dependency_subset(
            &fixture.name,
            "initial run",
            &first.dependency_paths,
            &expected_dependencies,
        );

        let second = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .with_context(|| format!("fixture `{}` cached rerun failed", fixture.name))?;
        assert_eq!(
            second.cache_hits, 1,
            "fixture `{}` should hit cache before dependencies change",
            fixture.name
        );
        assert_eq!(
            second.cache_misses, 0,
            "fixture `{}` should not reanalyze before dependencies change",
            fixture.name
        );
        assert_dependency_subset(
            &fixture.name,
            "cached rerun",
            &second.dependency_paths,
            &expected_dependencies,
        );

        materialize_files(tempdir.path(), &fixture.after_files)?;

        let third = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .with_context(|| {
            format!(
                "fixture `{}` rerun after dependency creation failed",
                fixture.name
            )
        })?;
        assert_eq!(
            third.cache_hits, 0,
            "fixture `{}` should invalidate its cache entry when a dependency appears",
            fixture.name
        );
        assert_eq!(
            third.cache_misses, 1,
            "fixture `{}` should reanalyze after dependency creation",
            fixture.name
        );
        assert_dependency_subset(
            &fixture.name,
            "rerun after dependency creation",
            &third.dependency_paths,
            &expected_dependencies,
        );
    }

    Ok(())
}

#[test]
fn zsh_plugin_dependency_fixtures_build_watch_targets_for_missing_dependencies() -> Result<()> {
    for fixture_path in fixture_paths()? {
        let fixture = load_fixture(&fixture_path)?;
        let tempdir = tempdir().context("failed to create temp dir")?;
        materialize_files(tempdir.path(), &fixture.files)?;

        let expected_dependencies = fixture_dependency_paths(tempdir.path(), &fixture);
        let mut args = check_args(false);
        args.paths = vec![PathBuf::from(&fixture.path)];

        let report = run_check_with_cwd(
            &args,
            &ConfigArguments::default(),
            tempdir.path(),
            &cache_root(tempdir.path()),
        )
        .with_context(|| format!("fixture `{}` initial run failed", fixture.name))?;
        let targets = collect_watch_targets(
            &args.paths,
            &ConfigArguments::default(),
            tempdir.path(),
            &report.dependency_paths,
        )
        .with_context(|| format!("fixture `{}` failed to collect watch targets", fixture.name))?;

        for dependency in expected_dependencies {
            let target = targets
                .iter()
                .find(|target| target.match_paths.contains(&dependency))
                .unwrap_or_else(|| {
                    panic!(
                        "fixture `{}` did not build a watch target for missing dependency {}",
                        fixture.name,
                        dependency.display()
                    )
                });

            let parent_exists = dependency.parent().is_some_and(Path::exists);
            assert_eq!(
                target.recursive,
                !parent_exists,
                "fixture `{}` should use recursive watch mode when dependency parent {} is missing",
                fixture.name,
                dependency.parent().unwrap_or(&dependency).display()
            );
        }
    }

    Ok(())
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("testdata")
        .join("zsh-plugin-dependencies")
}

fn fixture_paths() -> Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(fixture_dir())
        .with_context(|| format!("failed to read {}", fixture_dir().display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .context("failed to enumerate zsh plugin dependency fixtures")?;
    paths.retain(|path| path.extension().and_then(|ext| ext.to_str()) == Some("toml"));
    paths.sort();
    Ok(paths)
}

fn load_fixture(path: &Path) -> Result<Fixture> {
    let text =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))
}

fn materialize_files(root: &Path, files: &[FixtureFile]) -> Result<()> {
    for file in files {
        let path = root.join(&file.path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&path, &file.contents)
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

fn fixture_dependency_paths(root: &Path, fixture: &Fixture) -> Vec<PathBuf> {
    let root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    fixture
        .after_files
        .iter()
        .map(|file| normalize_path(&root.join(&file.path)))
        .collect()
}

fn assert_dependency_subset(
    fixture_name: &str,
    stage: &str,
    actual: &[PathBuf],
    expected: &[PathBuf],
) {
    for dependency in expected {
        assert!(
            actual.contains(dependency),
            "fixture `{fixture_name}` {stage} did not record dependency {}\nactual: {:?}",
            dependency.display(),
            actual
        );
    }
}

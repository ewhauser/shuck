use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use super::*;
use crate::environment::current_platform;
use crate::managed::install_with_environment;
use crate::metadata::parse_script_metadata;
use crate::registry::{available_shells, load_registry};
use crate::resolve::resolve_with_environment;
use crate::system::{find_on_path_in, resolve_system_at_path};

fn test_environment(root: &Path, registry_url: String) -> Environment {
    Environment {
        shells_root: root.join("shells"),
        registry_url,
    }
}

#[derive(Clone)]
struct RegistryEntry {
    shell: Shell,
    version: String,
    platform: String,
    url: String,
    sha256: String,
}

fn registry_entry(
    shell: Shell,
    version: &str,
    platform: &str,
    archive: &Path,
    sha256: &str,
) -> RegistryEntry {
    RegistryEntry {
        shell,
        version: version.to_owned(),
        platform: platform.to_owned(),
        url: Url::from_file_path(archive).unwrap().to_string(),
        sha256: sha256.to_owned(),
    }
}

fn write_registry_document(root: &Path, relative_path: &str, document: &Value) -> PathBuf {
    let path = root.join("registry").join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let contents = format!("{}\n", serde_json::to_string_pretty(document).unwrap());
    fs::write(&path, contents).unwrap();
    path
}

fn write_registry_site(root: &Path, entries: &[RegistryEntry]) -> PathBuf {
    let mut grouped = BTreeMap::<String, BTreeMap<String, BTreeMap<String, RegistryEntry>>>::new();
    for entry in entries {
        grouped
            .entry(entry.shell.as_str().to_owned())
            .or_default()
            .entry(entry.version.clone())
            .or_default()
            .insert(entry.platform.clone(), entry.clone());
    }

    let mut root_shells = Map::new();
    for (shell, versions) in &grouped {
        root_shells.insert(
            shell.clone(),
            json!({
                "versions_url": format!("shells/{shell}/index.json"),
            }),
        );

        let mut version_names = versions.keys().cloned().collect::<Vec<_>>();
        version_names.sort_by(|left, right| {
            Version::parse(right)
                .unwrap()
                .cmp(&Version::parse(left).unwrap())
        });

        let mut shell_versions = Map::new();
        for version in version_names {
            shell_versions.insert(
                version.clone(),
                json!({
                    "manifest_url": format!("{version}.json"),
                }),
            );

            let mut platforms = Map::new();
            for (platform, entry) in versions.get(&version).unwrap() {
                platforms.insert(
                    platform.clone(),
                    json!({
                        "url": entry.url,
                        "sha256": entry.sha256,
                    }),
                );
            }

            write_registry_document(
                root,
                &format!("shells/{shell}/{version}.json"),
                &json!({
                    "version": 2,
                    "kind": "shuck.shells.release",
                    "shell": shell,
                    "release": version,
                    "platforms": platforms,
                }),
            );
        }

        write_registry_document(
            root,
            &format!("shells/{shell}/index.json"),
            &json!({
                "version": 2,
                "kind": "shuck.shells.versions",
                "shell": shell,
                "versions": shell_versions,
            }),
        );
    }

    write_registry_document(
        root,
        "index.json",
        &json!({
            "version": 2,
            "kind": "shuck.shells.index",
            "shells": root_shells,
        }),
    )
}

fn make_shell_archive(root: &Path, shell: Shell, version: &str) -> (PathBuf, String) {
    let archive_root = root.join(format!("{}-{version}", shell.as_str()));
    let bin_dir = archive_root.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let shell_path = bin_dir.join(shell.as_str());
    let script = match shell {
        Shell::Bash | Shell::Gbash | Shell::Bashkit | Shell::Zsh => format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf '{} {}\\n'\n  exit 0\nfi\nprintf '%s\\n' \"${{SHUCK_SHELL_VERSION}}\"\n",
            shell.as_str(),
            version
        ),
        Shell::Dash => format!(
            "#!/bin/sh\nif [ \"$1\" = \"-V\" ] || [ \"$1\" = \"--version\" ]; then\n  printf '{} {}\\n' 1>&2\n  exit 0\nfi\nprintf '%s\\n' \"${{SHUCK_SHELL_VERSION}}\"\n",
            shell.as_str(),
            version
        ),
        Shell::Mksh => format!(
            "#!/bin/sh\nif [ \"$1\" = \"-c\" ]; then\n  printf '@(#)MIRBSD KSH R{}\\n'\n  exit 0\nfi\nif [ \"$1\" = \"-V\" ]; then\n  printf '@(#)MIRBSD KSH R{}\\n'\n  exit 0\nfi\nprintf '%s\\n' \"${{SHUCK_SHELL_VERSION}}\"\n",
            version, version
        ),
        Shell::Busybox => format!(
            "#!/bin/sh\nif [ \"$1\" = \"--help\" ]; then\n  printf 'BusyBox v{} () multi-call binary.\\n'\n  exit 0\nfi\nprintf '%s\\n' \"${{SHUCK_SHELL_VERSION}}\"\n",
            version
        ),
    };
    fs::write(&shell_path, script).unwrap();
    let mut permissions = fs::metadata(&shell_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shell_path, permissions).unwrap();

    let archive_path = root.join(format!("{}-{version}.tar.gz", shell.as_str()));
    let status = Command::new("/usr/bin/tar")
        .current_dir(&archive_root)
        .arg("-czf")
        .arg(&archive_path)
        .arg("bin")
        .status()
        .unwrap();
    assert!(status.success());

    let digest = Sha256::digest(fs::read(&archive_path).unwrap());
    let sha256 = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    (archive_path, sha256)
}

fn registry_for_archive(
    root: &Path,
    shell: Shell,
    version: &str,
    archive: &Path,
    sha256: &str,
) -> PathBuf {
    let platform = current_platform().unwrap();
    write_registry_site(
        root,
        &[registry_entry(shell, version, &platform, archive, sha256)],
    )
}

#[test]
fn parses_exact_and_range_constraints() {
    assert_eq!(
        Version::parse("59C").unwrap(),
        Version::parse("59c").unwrap()
    );
    assert!(Version::parse("5..2").is_err());
    assert!(Version::parse(".5.2").is_err());
    assert!(Version::parse("5.2.").is_err());
    assert!(Version::parse("18446744073709551616").is_err());
    assert!(matches!(
        VersionConstraint::parse("latest").unwrap(),
        VersionConstraint::Latest
    ));
    assert!(matches!(
        VersionConstraint::parse("5.2").unwrap(),
        VersionConstraint::ExactPrefix(_)
    ));
    assert!(matches!(
        VersionConstraint::parse("5.2.21").unwrap(),
        VersionConstraint::Exact(_)
    ));
    assert!(matches!(
        VersionConstraint::parse(">=5.1,<6").unwrap(),
        VersionConstraint::Range(_)
    ));
    assert!(VersionConstraint::parse(">=5.1 <6").is_err());
    assert!(VersionConstraint::parse("5.2)").is_err());
    assert!(VersionConstraint::parse(">=18446744073709551616.1").is_err());
}

#[test]
fn resolves_cli_shell_and_config_version() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let registry_path =
        registry_for_archive(tempdir.path(), Shell::Bash, "5.2.21", &archive, &sha256);
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );

    let config = RunConfig {
        shell: None,
        shell_version: None,
        shells: BTreeMap::from([(String::from("bash"), String::from("5.2"))]),
    };
    let resolved = resolve_with_environment(
        &environment,
        ResolveOptions {
            shell: Some(Shell::Bash),
            version: None,
            system: false,
            implicit_system_fallback: false,
            script: None,
            config: Some(&config),
            verbose: false,
            refresh_registry: false,
        },
    )
    .unwrap();

    assert_eq!(resolved.shell, Shell::Bash);
    assert_eq!(resolved.version.as_str(), "5.2.21");
    assert_eq!(resolved.source, ResolutionSource::Managed);
    assert!(resolved.path.ends_with("bin/bash"));
}

#[test]
fn metadata_overrides_project_defaults() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Zsh, "5.9");
    let registry_path = registry_for_archive(tempdir.path(), Shell::Zsh, "5.9", &archive, &sha256);
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );
    let script_path = tempdir.path().join("deploy.sh");
    fs::write(
        &script_path,
        "# /// shuck\n# shell = \"zsh\"\n# version = \"5.9\"\n# ///\nprint hello\n",
    )
    .unwrap();
    let config = RunConfig {
        shell: Some(String::from("bash")),
        shell_version: Some(String::from("5.2")),
        shells: BTreeMap::new(),
    };

    let resolved = resolve_with_environment(
        &environment,
        ResolveOptions {
            shell: None,
            version: None,
            system: false,
            implicit_system_fallback: false,
            script: Some(&script_path),
            config: Some(&config),
            verbose: false,
            refresh_registry: false,
        },
    )
    .unwrap();

    assert_eq!(resolved.shell, Shell::Zsh);
    assert_eq!(resolved.version.as_str(), "5.9");
}

#[test]
fn defaults_to_bash_when_only_a_version_constraint_is_provided() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive_a, sha_a) = make_shell_archive(tempdir.path(), Shell::Bash, "5.1.16");
    let (archive_b, sha_b) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let platform = current_platform().unwrap();
    let registry_path = write_registry_site(
        tempdir.path(),
        &[
            registry_entry(Shell::Bash, "5.1.16", &platform, &archive_a, &sha_a),
            registry_entry(Shell::Bash, "5.2.21", &platform, &archive_b, &sha_b),
        ],
    );
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );

    let resolved = resolve_with_environment(
        &environment,
        ResolveOptions {
            shell: None,
            version: Some(VersionConstraint::parse("5.2").unwrap()),
            system: false,
            implicit_system_fallback: false,
            script: None,
            config: None,
            verbose: false,
            refresh_registry: false,
        },
    )
    .unwrap();

    assert_eq!(resolved.shell, Shell::Bash);
    assert_eq!(resolved.version.as_str(), "5.2.21");
    assert_eq!(resolved.source, ResolutionSource::Managed);
}

#[test]
fn cli_shell_override_ignores_mismatched_metadata_version() {
    let tempdir = tempfile::tempdir().unwrap();
    let (bash_archive, bash_sha) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let (zsh_archive, zsh_sha) = make_shell_archive(tempdir.path(), Shell::Zsh, "5.9");
    let platform = current_platform().unwrap();
    let registry_path = write_registry_site(
        tempdir.path(),
        &[
            registry_entry(Shell::Bash, "5.2.21", &platform, &bash_archive, &bash_sha),
            registry_entry(Shell::Zsh, "5.9", &platform, &zsh_archive, &zsh_sha),
        ],
    );
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );
    let script_path = tempdir.path().join("deploy.sh");
    fs::write(
        &script_path,
        "# /// shuck\n# shell = \"zsh\"\n# version = \"5.9\"\n# ///\necho hi\n",
    )
    .unwrap();

    let resolved = resolve_with_environment(
        &environment,
        ResolveOptions {
            shell: Some(Shell::Bash),
            version: None,
            system: false,
            implicit_system_fallback: false,
            script: Some(&script_path),
            config: None,
            verbose: false,
            refresh_registry: false,
        },
    )
    .unwrap();

    assert_eq!(resolved.shell, Shell::Bash);
    assert_eq!(resolved.version.as_str(), "5.2.21");
    assert_eq!(resolved.source, ResolutionSource::Managed);
}

#[test]
fn shell_specific_config_pin_overrides_generic_shell_version() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive_a, sha_a) = make_shell_archive(tempdir.path(), Shell::Bash, "5.1.16");
    let (archive_b, sha_b) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let platform = current_platform().unwrap();
    let registry_path = write_registry_site(
        tempdir.path(),
        &[
            registry_entry(Shell::Bash, "5.1.16", &platform, &archive_a, &sha_a),
            registry_entry(Shell::Bash, "5.2.21", &platform, &archive_b, &sha_b),
        ],
    );
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );
    let script_path = tempdir.path().join("deploy.sh");
    fs::write(&script_path, "#!/usr/bin/env bash\necho hi\n").unwrap();
    let config = RunConfig {
        shell: None,
        shell_version: Some(String::from("5.1")),
        shells: BTreeMap::from([(String::from("bash"), String::from("5.2"))]),
    };

    let resolved = resolve_with_environment(
        &environment,
        ResolveOptions {
            shell: None,
            version: None,
            system: false,
            implicit_system_fallback: false,
            script: Some(&script_path),
            config: Some(&config),
            verbose: false,
            refresh_registry: false,
        },
    )
    .unwrap();

    assert_eq!(resolved.shell, Shell::Bash);
    assert_eq!(resolved.version.as_str(), "5.2.21");
}

#[test]
fn shebang_without_other_constraints_uses_latest_available_version() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive_a, sha_a) = make_shell_archive(tempdir.path(), Shell::Bash, "5.1.16");
    let (archive_b, sha_b) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let platform = current_platform().unwrap();
    let registry_path = write_registry_site(
        tempdir.path(),
        &[
            registry_entry(Shell::Bash, "5.1.16", &platform, &archive_a, &sha_a),
            registry_entry(Shell::Bash, "5.2.21", &platform, &archive_b, &sha_b),
        ],
    );
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );
    let script_path = tempdir.path().join("deploy.sh");
    fs::write(&script_path, "#!/usr/bin/env bash\necho hi\n").unwrap();

    let resolved = resolve_with_environment(
        &environment,
        ResolveOptions {
            shell: None,
            version: None,
            system: false,
            implicit_system_fallback: false,
            script: Some(&script_path),
            config: None,
            verbose: false,
            refresh_registry: false,
        },
    )
    .unwrap();

    assert_eq!(resolved.shell, Shell::Bash);
    assert_eq!(resolved.version.as_str(), "5.2.21");
}

#[test]
fn checksum_mismatch_aborts_install() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, _sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let registry_path = registry_for_archive(
        tempdir.path(),
        Shell::Bash,
        "5.2.21",
        &archive,
        "0000000000000000000000000000000000000000000000000000000000000000",
    );
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );

    let err = install_with_environment(
        &environment,
        Shell::Bash,
        &VersionConstraint::parse("5.2").unwrap(),
        false,
        false,
    )
    .unwrap_err();

    assert!(format!("{err:#}").contains("Checksum mismatch"));
}

#[test]
fn partial_install_directory_is_replaced() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let registry_path =
        registry_for_archive(tempdir.path(), Shell::Bash, "5.2.21", &archive, &sha256);
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );
    let platform = current_platform().unwrap();
    let install_dir = environment
        .shells_root
        .join("bash")
        .join("5.2.21")
        .join(&platform);
    fs::create_dir_all(&install_dir).unwrap();
    fs::write(install_dir.join("partial.txt"), "incomplete").unwrap();

    let resolved = install_with_environment(
        &environment,
        Shell::Bash,
        &VersionConstraint::parse("5.2").unwrap(),
        false,
        false,
    )
    .unwrap();

    assert_eq!(resolved.source, ResolutionSource::Managed);
    assert_eq!(resolved.version.as_str(), "5.2.21");
    assert!(resolved.path.exists());
    assert!(!install_dir.join("partial.txt").exists());
}

#[test]
fn invalid_cached_install_is_replaced() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let registry_path =
        registry_for_archive(tempdir.path(), Shell::Bash, "5.2.21", &archive, &sha256);
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );
    let platform = current_platform().unwrap();
    let install_dir = environment
        .shells_root
        .join("bash")
        .join("5.2.21")
        .join(&platform)
        .join("bin");
    fs::create_dir_all(&install_dir).unwrap();
    let binary_path = install_dir.join("bash");
    fs::write(
        &binary_path,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'bash 4.4.0\\n'\n  exit 0\nfi\nexit 0\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&binary_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&binary_path, permissions).unwrap();

    let resolved = install_with_environment(
        &environment,
        Shell::Bash,
        &VersionConstraint::parse("5.2").unwrap(),
        false,
        false,
    )
    .unwrap();

    assert_eq!(resolved.version.as_str(), "5.2.21");
    assert_eq!(resolved.source, ResolutionSource::Managed);
    assert!(
        fs::read_to_string(&resolved.path)
            .unwrap()
            .contains("5.2.21")
    );
}

#[test]
fn install_picks_latest_version_with_current_platform_artifact() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive_old, sha_old) = make_shell_archive(tempdir.path(), Shell::Bash, "5.1.16");
    let (archive_new, sha_new) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let platform = current_platform().unwrap();
    let registry_path = write_registry_site(
        tempdir.path(),
        &[
            registry_entry(Shell::Bash, "5.1.16", &platform, &archive_old, &sha_old),
            registry_entry(
                Shell::Bash,
                "5.2.21",
                "other-platform",
                &archive_new,
                &sha_new,
            ),
        ],
    );
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );

    let resolved = install_with_environment(
        &environment,
        Shell::Bash,
        &VersionConstraint::Latest,
        false,
        false,
    )
    .unwrap();

    assert_eq!(resolved.version.as_str(), "5.1.16");
    assert_eq!(resolved.source, ResolutionSource::Managed);
}

#[test]
fn failed_refresh_keeps_last_good_registry() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let good_registry_path =
        registry_for_archive(tempdir.path(), Shell::Bash, "5.2.21", &archive, &sha256);
    let bad_registry_path = tempdir.path().join("bad-registry.json");
    fs::write(&bad_registry_path, "{not json").unwrap();

    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(&good_registry_path)
            .unwrap()
            .to_string(),
    );
    let loaded = load_registry(&environment, false, false).unwrap();
    assert!(loaded.shells.contains_key("bash"));

    let failing_environment = test_environment(
        tempdir.path(),
        Url::from_file_path(&bad_registry_path).unwrap().to_string(),
    );
    let refresh_err = load_registry(&failing_environment, true, false).unwrap_err();
    assert!(format!("{refresh_err:#}").contains("parse"));

    let recovered = load_registry(&failing_environment, false, false).unwrap();
    assert!(recovered.shells.contains_key("bash"));
}

#[test]
fn registry_site_root_url_resolves_root_index_document() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let registry_path =
        registry_for_archive(tempdir.path(), Shell::Bash, "5.2.21", &archive, &sha256);
    let registry_root = registry_path.parent().unwrap();

    let environment = test_environment(
        tempdir.path(),
        Url::from_directory_path(registry_root).unwrap().to_string(),
    );
    let loaded = load_registry(&environment, false, false).unwrap();

    assert!(loaded.shells.contains_key("bash"));
}

#[test]
fn explicit_registry_endpoint_url_is_fetched_as_given() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let platform = current_platform().unwrap();
    let endpoint_path = tempdir.path().join("registry-endpoint");

    fs::create_dir_all(tempdir.path().join("shells/bash")).unwrap();
    fs::write(
        &endpoint_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "version": 2,
                "kind": "shuck.shells.index",
                "shells": {
                    "bash": {
                        "versions_url": "shells/bash/index.json",
                    }
                },
            }))
            .unwrap()
        ),
    )
    .unwrap();
    fs::write(
        tempdir.path().join("shells/bash/index.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "version": 2,
                "kind": "shuck.shells.versions",
                "shell": "bash",
                "versions": {
                    "5.2.21": {
                        "manifest_url": "5.2.21.json",
                    }
                },
            }))
            .unwrap()
        ),
    )
    .unwrap();
    fs::write(
        tempdir.path().join("shells/bash/5.2.21.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "version": 2,
                "kind": "shuck.shells.release",
                "shell": "bash",
                "release": "5.2.21",
                "platforms": {
                    platform: {
                        "url": Url::from_file_path(archive).unwrap().to_string(),
                        "sha256": sha256,
                    }
                },
            }))
            .unwrap()
        ),
    )
    .unwrap();

    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(endpoint_path).unwrap().to_string(),
    );
    let loaded = load_registry(&environment, false, false).unwrap();

    assert!(loaded.shells.contains_key("bash"));
}

#[test]
fn unresolved_shell_uses_managed_bash_without_implicit_system_fallback() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let registry_path =
        registry_for_archive(tempdir.path(), Shell::Bash, "5.2.21", &archive, &sha256);
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );

    let resolved = resolve_with_environment(
        &environment,
        ResolveOptions {
            shell: None,
            version: None,
            system: false,
            implicit_system_fallback: false,
            script: None,
            config: None,
            verbose: false,
            refresh_registry: false,
        },
    )
    .unwrap();

    assert_eq!(resolved.shell, Shell::Bash);
    assert_eq!(resolved.version.as_str(), "5.2.21");
    assert_eq!(resolved.source, ResolutionSource::Managed);
}

#[test]
fn non_utf8_script_still_resolves_with_explicit_shell_and_version() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let registry_path =
        registry_for_archive(tempdir.path(), Shell::Bash, "5.2.21", &archive, &sha256);
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );
    let script_path = tempdir.path().join("legacy.sh");
    fs::write(&script_path, b"#!/bin/sh\nprintf '\xff'\n").unwrap();

    let resolved = resolve_with_environment(
        &environment,
        ResolveOptions {
            shell: Some(Shell::Bash),
            version: Some(VersionConstraint::parse("5.2").unwrap()),
            system: false,
            implicit_system_fallback: false,
            script: Some(&script_path),
            config: None,
            verbose: false,
            refresh_registry: false,
        },
    )
    .unwrap();

    assert_eq!(resolved.shell, Shell::Bash);
    assert_eq!(resolved.version.as_str(), "5.2.21");
    assert_eq!(resolved.source, ResolutionSource::Managed);
}

#[test]
fn parses_script_metadata_before_non_comment_lines() {
    let metadata = parse_script_metadata(
            "# /// shuck\n# shell = \"bash\"\n# version = \">=5.1\"\n# [metadata]\n# description = \"demo\"\n# ///\necho hi\n",
        )
        .unwrap()
        .unwrap();
    assert_eq!(metadata.shell, Shell::Bash);
    assert!(matches!(
        metadata.version.unwrap(),
        VersionConstraint::Range(_)
    ));

    let err =
        parse_script_metadata("echo hi\n# /// shuck\n# shell = \"bash\"\n# ///\n").unwrap_err();
    assert!(err.to_string().contains("before the script body"));

    let err = parse_script_metadata(
        "# /// shuck\n# shell = \"bash\"\n# ///\necho hi\n# /// shuck\n# shell = \"zsh\"\n# ///\n",
    )
    .unwrap_err();
    assert!(err.to_string().contains("multiple `# /// shuck` blocks"));

    let metadata = parse_script_metadata(
        "# /// shuck\n# shell = \"bash\"\n# ///\ncat <<'EOF'\n# /// shuck\nshell = \"zsh\"\n# ///\nEOF\n",
    )
    .unwrap()
    .unwrap();
    assert_eq!(metadata.shell, Shell::Bash);

    let metadata =
        parse_script_metadata("# /// shuck notes\n# shell = \"bash\"\necho hi\n").unwrap();
    assert!(metadata.is_none());
}

#[test]
fn rejects_unknown_metadata_keys() {
    assert!(
        parse_script_metadata("# /// shuck\n# shell = \"bash\"\n# foo = \"bar\"\n# ///\n").is_err()
    );
}

#[test]
fn lists_available_versions() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive_a, sha_a) = make_shell_archive(tempdir.path(), Shell::Bash, "5.1.16");
    let (archive_b, sha_b) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let platform = current_platform().unwrap();
    let registry_path = write_registry_site(
        tempdir.path(),
        &[
            registry_entry(Shell::Bash, "5.1.16", &platform, &archive_a, &sha_a),
            registry_entry(Shell::Bash, "5.2.21", &platform, &archive_b, &sha_b),
        ],
    );
    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(registry_path).unwrap().to_string(),
    );

    let available = available_shells(
        &load_registry(&environment, false, false).unwrap(),
        Some(Shell::Bash),
    );
    assert_eq!(available.len(), 1);
    assert_eq!(available[0].versions[0].as_str(), "5.2.21");
    assert_eq!(available[0].versions[1].as_str(), "5.1.16");
}

#[test]
fn busybox_system_resolution_detects_version() {
    let tempdir = tempfile::tempdir().unwrap();
    let path_dir = tempdir.path().join("bin");
    fs::create_dir_all(&path_dir).unwrap();
    let shell_path = path_dir.join("busybox");
    fs::write(
        &shell_path,
        "#!/bin/sh\nif [ \"$1\" = \"--help\" ]; then\n  printf 'BusyBox v1.36.1 () multi-call binary.\\n'\n  exit 0\nfi\nexit 0\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&shell_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shell_path, permissions).unwrap();

    let resolved = resolve_system_at_path(
        Shell::Busybox,
        &shell_path,
        &VersionConstraint::parse(">=1.36,<2").unwrap(),
    )
    .unwrap();

    assert_eq!(resolved.version.as_str(), "1.36.1");
    assert_eq!(resolved.source, ResolutionSource::System);
}

#[test]
fn busybox_resolution_is_linux_only() {
    let tempdir = tempfile::tempdir().unwrap();
    let script_path = tempdir.path().join("deploy.sh");
    fs::write(&script_path, "#!/bin/busybox sh\necho hi\n").unwrap();

    if cfg!(target_os = "linux") {
        let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Busybox, "1.36.1");
        let registry_path =
            registry_for_archive(tempdir.path(), Shell::Busybox, "1.36.1", &archive, &sha256);
        let environment = test_environment(
            tempdir.path(),
            Url::from_file_path(registry_path).unwrap().to_string(),
        );

        let resolved = resolve_with_environment(
            &environment,
            ResolveOptions {
                shell: None,
                version: None,
                system: false,
                implicit_system_fallback: false,
                script: Some(&script_path),
                config: None,
                verbose: false,
                refresh_registry: false,
            },
        )
        .unwrap();

        assert_eq!(resolved.shell, Shell::Busybox);
        assert_eq!(resolved.version.as_str(), "1.36.1");
        assert_eq!(resolved.source, ResolutionSource::Managed);
        assert!(resolved.path.ends_with("bin/busybox"));
        return;
    }

    let environment = test_environment(
        tempdir.path(),
        Url::from_file_path(tempdir.path().join("registry.json"))
            .unwrap()
            .to_string(),
    );
    let err = resolve_with_environment(
        &environment,
        ResolveOptions {
            shell: None,
            version: None,
            system: false,
            implicit_system_fallback: false,
            script: Some(&script_path),
            config: None,
            verbose: false,
            refresh_registry: false,
        },
    )
    .unwrap_err();

    assert!(err.to_string().contains("only supported on Linux"));
}

#[test]
fn system_resolution_checks_version_constraints() {
    let tempdir = tempfile::tempdir().unwrap();
    let path_dir = tempdir.path().join("bin");
    fs::create_dir_all(&path_dir).unwrap();
    let shell_path = path_dir.join("bash");
    fs::write(
        &shell_path,
        "#!/bin/sh\nprintf 'GNU bash, version 5.2.21(1)-release\\n'\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&shell_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shell_path, permissions).unwrap();

    let resolved = resolve_system_at_path(
        Shell::Bash,
        &shell_path,
        &VersionConstraint::parse(">=5.1,<6").unwrap(),
    )
    .unwrap();
    assert_eq!(resolved.version.as_str(), "5.2.21");
    let err = resolve_system_at_path(
        Shell::Bash,
        &shell_path,
        &VersionConstraint::parse(">=6").unwrap(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("System bash is 5.2.21"));
}

#[test]
fn failed_version_probe_output_does_not_count_as_a_version() {
    let tempdir = tempfile::tempdir().unwrap();
    let path_dir = tempdir.path().join("bin");
    fs::create_dir_all(&path_dir).unwrap();
    let shell_path = path_dir.join("dash");
    fs::write(
        &shell_path,
        "#!/bin/sh\nif [ \"$1\" = \"-V\" ]; then\n  printf 'dash: 0: Illegal option -V\\n' 1>&2\n  exit 2\nfi\nif [ \"$1\" = \"--version\" ]; then\n  printf 'dash 0.5.12\\n' 1>&2\n  exit 0\nfi\nexit 0\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&shell_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shell_path, permissions).unwrap();

    let resolved = resolve_system_at_path(
        Shell::Dash,
        &shell_path,
        &VersionConstraint::parse(">=0.5").unwrap(),
    )
    .unwrap();

    assert_eq!(resolved.version.as_str(), "0.5.12");
    assert_eq!(resolved.source, ResolutionSource::System);
}

#[test]
fn gbash_version_probe_falls_back_to_version_subcommand() {
    let tempdir = tempfile::tempdir().unwrap();
    let path_dir = tempdir.path().join("bin");
    fs::create_dir_all(&path_dir).unwrap();
    let shell_path = path_dir.join("gbash");
    fs::write(
        &shell_path,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf 'gbash: unknown option --version\\n' 1>&2\n  exit 2\nfi\nif [ \"$1\" = \"version\" ]; then\n  printf 'gbash 0.0.32\\ncommit: abc123\\n'\n  exit 0\nfi\nexit 1\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&shell_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shell_path, permissions).unwrap();

    let resolved = resolve_system_at_path(
        Shell::Gbash,
        &shell_path,
        &VersionConstraint::parse(">=0.0.30").unwrap(),
    )
    .unwrap();

    assert_eq!(resolved.version.as_str(), "0.0.32");
    assert_eq!(resolved.source, ResolutionSource::System);
}

#[test]
fn path_lookup_skips_non_executable_entries() {
    let tempdir = tempfile::tempdir().unwrap();
    let shadow_dir = tempdir.path().join("shadow");
    let real_dir = tempdir.path().join("real");
    fs::create_dir_all(&shadow_dir).unwrap();
    fs::create_dir_all(&real_dir).unwrap();

    let shadow_bash = shadow_dir.join("bash");
    fs::write(&shadow_bash, "#!/bin/sh\nexit 0\n").unwrap();
    let mut shadow_permissions = fs::metadata(&shadow_bash).unwrap().permissions();
    shadow_permissions.set_mode(0o644);
    fs::set_permissions(&shadow_bash, shadow_permissions).unwrap();

    let real_bash = real_dir.join("bash");
    fs::write(&real_bash, "#!/bin/sh\nexit 0\n").unwrap();
    let mut real_permissions = fs::metadata(&real_bash).unwrap().permissions();
    real_permissions.set_mode(0o755);
    fs::set_permissions(&real_bash, real_permissions).unwrap();

    let path_var = std::env::join_paths([shadow_dir.as_path(), real_dir.as_path()]).unwrap();
    let resolved = find_on_path_in(Some(path_var.as_os_str()), "bash").unwrap();

    assert_eq!(resolved, real_bash);
}

#[cfg(not(unix))]
#[test]
fn path_lookup_uses_pathext_variants() {
    let tempdir = tempfile::tempdir().unwrap();
    let path_dir = tempdir.path().join("bin");
    fs::create_dir_all(&path_dir).unwrap();

    let bash_exe = path_dir.join("bash.exe");
    fs::write(&bash_exe, "@echo off\r\n").unwrap();
    let path_var = std::env::join_paths([path_dir.as_path()]).unwrap();

    let previous_pathext = std::env::var_os("PATHEXT");
    std::env::set_var("PATHEXT", ".EXE;.CMD");

    let resolved = find_on_path_in(Some(path_var.as_os_str()), "bash").unwrap();

    if let Some(previous_pathext) = previous_pathext {
        std::env::set_var("PATHEXT", previous_pathext);
    } else {
        std::env::remove_var("PATHEXT");
    }

    assert_eq!(resolved, bash_exe);
}

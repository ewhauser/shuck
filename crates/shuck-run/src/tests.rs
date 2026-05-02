use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

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

fn write_registry(root: &Path, body: &str) -> PathBuf {
    let registry_path = root.join("registry.json");
    fs::write(&registry_path, body).unwrap();
    registry_path
}

fn make_shell_archive(root: &Path, shell: Shell, version: &str) -> (PathBuf, String) {
    let archive_root = root.join(format!("{}-{version}", shell.as_str()));
    let bin_dir = archive_root.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let shell_path = bin_dir.join(shell.as_str());
    let script = match shell {
        Shell::Bash | Shell::Zsh => format!(
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

fn registry_for_archive(shell: Shell, version: &str, archive: &Path, sha256: &str) -> String {
    let platform = current_platform().unwrap();
    format!(
        r#"{{
  "version": 1,
  "shells": {{
    "{shell}": {{
      "versions": {{
        "{version}": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url}",
              "sha256": "{sha256}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
        shell = shell.as_str(),
        version = version,
        platform = platform,
        url = Url::from_file_path(archive).unwrap(),
        sha256 = sha256
    )
}

#[test]
fn parses_exact_and_range_constraints() {
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
}

#[test]
fn resolves_cli_shell_and_config_version() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let registry = registry_for_archive(Shell::Bash, "5.2.21", &archive, &sha256);
    let registry_path = write_registry(tempdir.path(), &registry);
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
    let registry = registry_for_archive(Shell::Zsh, "5.9", &archive, &sha256);
    let registry_path = write_registry(tempdir.path(), &registry);
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
    let registry = format!(
        r#"{{
  "version": 1,
  "shells": {{
    "bash": {{
      "versions": {{
        "5.1.16": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_a}",
              "sha256": "{sha_a}"
            }}
          }}
        }},
        "5.2.21": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_b}",
              "sha256": "{sha_b}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
        platform = platform,
        url_a = Url::from_file_path(archive_a).unwrap(),
        sha_a = sha_a,
        url_b = Url::from_file_path(archive_b).unwrap(),
        sha_b = sha_b
    );
    let registry_path = write_registry(tempdir.path(), &registry);
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
    let registry = format!(
        r#"{{
  "version": 1,
  "shells": {{
    "bash": {{
      "versions": {{
        "5.2.21": {{
          "platforms": {{
            "{platform}": {{
              "url": "{bash_url}",
              "sha256": "{bash_sha}"
            }}
          }}
        }}
      }}
    }},
    "zsh": {{
      "versions": {{
        "5.9": {{
          "platforms": {{
            "{platform}": {{
              "url": "{zsh_url}",
              "sha256": "{zsh_sha}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
        platform = platform,
        bash_url = Url::from_file_path(bash_archive).unwrap(),
        bash_sha = bash_sha,
        zsh_url = Url::from_file_path(zsh_archive).unwrap(),
        zsh_sha = zsh_sha
    );
    let registry_path = write_registry(tempdir.path(), &registry);
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
    let registry = format!(
        r#"{{
  "version": 1,
  "shells": {{
    "bash": {{
      "versions": {{
        "5.1.16": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_a}",
              "sha256": "{sha_a}"
            }}
          }}
        }},
        "5.2.21": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_b}",
              "sha256": "{sha_b}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
        platform = platform,
        url_a = Url::from_file_path(archive_a).unwrap(),
        sha_a = sha_a,
        url_b = Url::from_file_path(archive_b).unwrap(),
        sha_b = sha_b
    );
    let registry_path = write_registry(tempdir.path(), &registry);
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
    let registry = format!(
        r#"{{
  "version": 1,
  "shells": {{
    "bash": {{
      "versions": {{
        "5.1.16": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_a}",
              "sha256": "{sha_a}"
            }}
          }}
        }},
        "5.2.21": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_b}",
              "sha256": "{sha_b}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
        platform = platform,
        url_a = Url::from_file_path(archive_a).unwrap(),
        sha_a = sha_a,
        url_b = Url::from_file_path(archive_b).unwrap(),
        sha_b = sha_b
    );
    let registry_path = write_registry(tempdir.path(), &registry);
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
    let registry = registry_for_archive(
        Shell::Bash,
        "5.2.21",
        &archive,
        "0000000000000000000000000000000000000000000000000000000000000000",
    );
    let registry_path = write_registry(tempdir.path(), &registry);
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
    let registry = registry_for_archive(Shell::Bash, "5.2.21", &archive, &sha256);
    let registry_path = write_registry(tempdir.path(), &registry);
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
fn unresolved_shell_uses_managed_bash_without_implicit_system_fallback() {
    let tempdir = tempfile::tempdir().unwrap();
    let (archive, sha256) = make_shell_archive(tempdir.path(), Shell::Bash, "5.2.21");
    let registry = registry_for_archive(Shell::Bash, "5.2.21", &archive, &sha256);
    let registry_path = write_registry(tempdir.path(), &registry);
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
    let registry = format!(
        r#"{{
  "version": 1,
  "shells": {{
    "bash": {{
      "versions": {{
        "5.1.16": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_a}",
              "sha256": "{sha_a}"
            }}
          }}
        }},
        "5.2.21": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url_b}",
              "sha256": "{sha_b}"
            }}
          }}
        }}
      }}
    }}
  }}
}}"#,
        platform = platform,
        url_a = Url::from_file_path(archive_a).unwrap(),
        sha_a = sha_a,
        url_b = Url::from_file_path(archive_b).unwrap(),
        sha_b = sha_b
    );
    let registry_path = write_registry(tempdir.path(), &registry);
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

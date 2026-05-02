#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use url::Url;

fn configure_runtime_env(cmd: &mut Command, root: &Path, registry_path: &Path) {
    cmd.env("SHUCK_SHELLS_DIR", root.join("shells"));
    cmd.env(
        "SHUCK_RUN_REGISTRY_URL",
        Url::from_file_path(registry_path).unwrap().to_string(),
    );
}

fn registry_path(root: &Path, body: &str) -> PathBuf {
    let path = root.join("registry.json");
    fs::write(&path, body).unwrap();
    path
}

fn fake_shell_archive(root: &Path, shell: &str, version: &str) -> (PathBuf, String) {
    let archive_root = root.join(format!("{shell}-{version}"));
    let bin_dir = archive_root.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let shell_path = bin_dir.join(shell);
    let probe = match shell {
        "bash" | "zsh" => format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n  printf '{} {}\\n'\n  exit 0\nfi\nexec /bin/sh \"$@\"\n",
            shell, version
        ),
        "dash" => format!(
            "#!/bin/sh\nif [ \"$1\" = \"-V\" ] || [ \"$1\" = \"--version\" ]; then\n  printf '{} {}\\n' 1>&2\n  exit 0\nfi\nexec /bin/sh \"$@\"\n",
            shell, version
        ),
        other => panic!("unsupported fake shell {other}"),
    };
    fs::write(&shell_path, probe).unwrap();
    let mut permissions = fs::metadata(&shell_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&shell_path, permissions).unwrap();

    let archive_path = root.join(format!("{shell}-{version}.tar.gz"));
    let status = ProcessCommand::new("tar")
        .current_dir(&archive_root)
        .args(["-czf"])
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

fn registry_body(shell: &str, versions: &[(&str, &Path, &str)]) -> String {
    let platform = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-linux",
        ("linux", "aarch64") => "aarch64-linux",
        ("macos", "x86_64") => "x86_64-darwin",
        ("macos", "aarch64") => "aarch64-darwin",
        (os, arch) => panic!("unsupported test platform {arch}-{os}"),
    };
    let versions = versions
        .iter()
        .map(|(version, archive, sha256)| {
            format!(
                r#"
        "{version}": {{
          "platforms": {{
            "{platform}": {{
              "url": "{url}",
              "sha256": "{sha256}"
            }}
          }}
        }}"#,
                url = Url::from_file_path(archive).unwrap(),
            )
        })
        .collect::<Vec<_>>()
        .join(",");

    format!(
        r#"{{
  "version": 1,
  "shells": {{
    "{shell}": {{
      "versions": {{{versions}
      }}
    }}
  }}
}}"#
    )
}

fn fake_system_shell(path: &Path, name: &str, version: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let contents = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ] || [ \"$1\" = \"-V\" ]; then\n  printf '{} {}\\n'\n  exit 0\nfi\nexec /bin/sh \"$@\"\n",
        name, version
    );
    fs::write(path, contents).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
fn top_level_help_includes_runtime_commands() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("run"))
        .stdout(predicate::str::contains("install"))
        .stdout(predicate::str::contains("shell"));
}

#[test]
fn run_dry_run_uses_project_config_and_registry() {
    let tempdir = tempdir().unwrap();
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "bash", "5.2.21");
    let registry = registry_body("bash", &[("5.2.21", &archive, &sha256)]);
    let registry = registry_path(tempdir.path(), &registry);
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[run.shells]\nbash = '5.2'\n",
    )
    .unwrap();
    fs::write(
        tempdir.path().join("deploy.sh"),
        "#!/usr/bin/env bash\necho hi\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path())
        .args(["run", "--dry-run", "deploy.sh"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("bash 5.2.21"))
        .stdout(predicate::str::contains("bin/bash"));
}

#[test]
fn run_command_string_uses_managed_shell_and_sets_env() {
    let tempdir = tempdir().unwrap();
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "bash", "5.2.21");
    let registry = registry_body("bash", &[("5.2.21", &archive, &sha256)]);
    let registry = registry_path(tempdir.path(), &registry);

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path()).args([
        "run",
        "--shell-version",
        "5.2",
        "-c",
        "printf '%s|%s\\n' \"$SHUCK_SHELL\" \"$SHUCK_SHELL_VERSION\"",
    ]);
    cmd.assert().success().stdout("bash|5.2.21\n");
}

#[test]
fn run_stdin_defaults_to_system_bash() {
    let tempdir = tempdir().unwrap();
    let bin_dir = tempdir.path().join("bin");
    fake_system_shell(&bin_dir.join("bash"), "bash", "5.2.21");

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .env("PATH", bin_dir.as_os_str())
        .arg("run")
        .write_stdin("printf '%s\\n' \"$SHUCK_SHELL\"\n");
    cmd.assert().success().stdout("bash\n");
}

#[test]
fn run_stdin_with_shell_executes_managed_interpreter() {
    let tempdir = tempdir().unwrap();
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "bash", "5.2.21");
    let registry = registry_body("bash", &[("5.2.21", &archive, &sha256)]);
    let registry = registry_path(tempdir.path(), &registry);

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path())
        .args(["run", "--shell", "bash", "-"])
        .write_stdin("printf '%s\\n' \"$SHUCK_SHELL\"\n");
    cmd.assert().success().stdout("bash\n");
}

#[test]
fn install_list_shows_newest_version_first() {
    let tempdir = tempdir().unwrap();
    let (archive_a, sha_a) = fake_shell_archive(tempdir.path(), "bash", "5.1.16");
    let (archive_b, sha_b) = fake_shell_archive(tempdir.path(), "bash", "5.2.21");
    let registry = registry_body(
        "bash",
        &[
            ("5.1.16", &archive_a, &sha_a),
            ("5.2.21", &archive_b, &sha_b),
        ],
    );
    let registry = registry_path(tempdir.path(), &registry);

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path())
        .args(["install", "--list", "bash"]);
    cmd.assert().success().stdout("5.2.21\n5.1.16\n");
}

#[test]
fn system_run_reports_version_mismatch() {
    let tempdir = tempdir().unwrap();
    let bin_dir = tempdir.path().join("bin");
    fake_system_shell(&bin_dir.join("bash"), "bash", "3.2.57");

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .env("PATH", bin_dir.as_os_str())
        .args([
            "run",
            "--system",
            "--shell-version",
            ">=5.1",
            "-c",
            "echo hi",
        ]);
    cmd.assert()
        .code(2)
        .stderr(predicate::str::contains("System bash is 3.2.57"));
}

#[test]
fn shell_subcommand_uses_system_shell() {
    let tempdir = tempdir().unwrap();
    let bin_dir = tempdir.path().join("bin");
    fake_system_shell(&bin_dir.join("bash"), "bash", "5.2.21");

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .env("PATH", bin_dir.as_os_str())
        .args(["shell", "--system"])
        .write_stdin("");
    cmd.assert().success();
}

#[test]
fn shell_subcommand_defaults_to_managed_bash() {
    let tempdir = tempdir().unwrap();
    let bin_dir = tempdir.path().join("bin");
    fake_system_shell(&bin_dir.join("bash"), "bash", "3.2.57");
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "bash", "5.2.21");
    let registry = registry_body("bash", &[("5.2.21", &archive, &sha256)]);
    let registry = registry_path(tempdir.path(), &registry);
    let path = std::env::join_paths(
        std::iter::once(bin_dir.clone()).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path())
        .env("PATH", path)
        .arg("shell")
        .write_stdin("printf '%s|%s\\n' \"$SHUCK_SHELL\" \"$SHUCK_SHELL_VERSION\"\n");
    cmd.assert().success().stdout("bash|5.2.21\n");
}

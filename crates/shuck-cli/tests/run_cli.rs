#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::{Map, json};
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

fn fake_shell_archive(root: &Path, shell: &str, version: &str) -> (PathBuf, String) {
    let archive_root = root.join(format!("{shell}-{version}"));
    let bin_dir = archive_root.join("bin");
    fs::create_dir_all(&bin_dir).unwrap();
    let shell_path = bin_dir.join(shell);
    let probe = match shell {
        "bash" | "gbash" | "bashkit" | "zsh" => format!(
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

fn registry_platform() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") if cfg!(target_env = "gnu") => "x86_64-linux-gnu",
        ("linux", "x86_64") if cfg!(target_env = "musl") => "x86_64-linux-musl",
        ("linux", "aarch64") if cfg!(target_env = "gnu") => "aarch64-linux-gnu",
        ("linux", "aarch64") if cfg!(target_env = "musl") => "aarch64-linux-musl",
        ("macos", "x86_64") => "x86_64-darwin",
        ("macos", "aarch64") => "aarch64-darwin",
        (os, arch) => panic!("unsupported test platform {arch}-{os}"),
    }
}

fn registry_path(root: &Path, shell: &str, versions: &[(&str, &Path, &str)]) -> PathBuf {
    let root_dir = root.join("registry");
    let shell_dir = root_dir.join("shells").join(shell);
    fs::create_dir_all(&shell_dir).unwrap();

    let platform = registry_platform();
    let mut shell_versions = Map::new();
    for (version, archive, sha256) in versions {
        shell_versions.insert(
            (*version).to_owned(),
            json!({
                "manifest_url": format!("{version}.json"),
            }),
        );
        fs::write(
            shell_dir.join(format!("{version}.json")),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&json!({
                    "version": 2,
                    "kind": "shuck.shells.release",
                    "shell": shell,
                    "release": version,
                    "platforms": {
                        platform: {
                            "url": Url::from_file_path(archive).unwrap().to_string(),
                            "sha256": sha256,
                        }
                    }
                }))
                .unwrap()
            ),
        )
        .unwrap();
    }

    fs::write(
        shell_dir.join("index.json"),
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "version": 2,
                "kind": "shuck.shells.versions",
                "shell": shell,
                "versions": shell_versions,
            }))
            .unwrap()
        ),
    )
    .unwrap();

    let path = root_dir.join("index.json");
    fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&json!({
                "version": 2,
                "kind": "shuck.shells.index",
                "shells": {
                    shell: {
                        "versions_url": format!("shells/{shell}/index.json"),
                    }
                }
            }))
            .unwrap()
        ),
    )
    .unwrap();
    path
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
    let registry = registry_path(tempdir.path(), "bash", &[("5.2.21", &archive, &sha256)]);
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
    let registry = registry_path(tempdir.path(), "bash", &[("5.2.21", &archive, &sha256)]);

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
fn run_command_string_supports_gbash_shell_and_sets_env() {
    let tempdir = tempdir().unwrap();
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "gbash", "0.0.32");
    let registry = registry_path(tempdir.path(), "gbash", &[("0.0.32", &archive, &sha256)]);

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path()).args([
        "run",
        "--shell",
        "gbash",
        "--shell-version",
        "0.0",
        "-c",
        "printf '%s|%s\\n' \"$SHUCK_SHELL\" \"$SHUCK_SHELL_VERSION\"",
    ]);
    cmd.assert().success().stdout("gbash|0.0.32\n");
}

#[test]
fn run_command_string_supports_bashkit_shell_and_sets_env() {
    let tempdir = tempdir().unwrap();
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "bashkit", "0.2.1");
    let registry = registry_path(tempdir.path(), "bashkit", &[("0.2.1", &archive, &sha256)]);

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path()).args([
        "run",
        "--shell",
        "bashkit",
        "--shell-version",
        "0.2",
        "-c",
        "printf '%s|%s\\n' \"$SHUCK_SHELL\" \"$SHUCK_SHELL_VERSION\"",
    ]);
    cmd.assert().success().stdout("bashkit|0.2.1\n");
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
    let registry = registry_path(tempdir.path(), "bash", &[("5.2.21", &archive, &sha256)]);

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path())
        .args(["run", "--shell", "bash", "-"])
        .write_stdin("printf '%s\\n' \"$SHUCK_SHELL\"\n");
    cmd.assert().success().stdout("bash\n");
}

#[test]
fn run_dry_run_supports_bashkit_shebang_and_config() {
    let tempdir = tempdir().unwrap();
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "bashkit", "0.2.1");
    let registry = registry_path(tempdir.path(), "bashkit", &[("0.2.1", &archive, &sha256)]);
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[run.shells]\nbashkit = '0.2'\n",
    )
    .unwrap();
    fs::write(
        tempdir.path().join("sandbox.sh"),
        "#!/usr/bin/env bashkit\necho hi\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path())
        .args(["run", "--dry-run", "sandbox.sh"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("bashkit 0.2.1"))
        .stdout(predicate::str::contains("bin/bashkit"));
}

#[test]
fn run_bashkit_script_preserves_script_args() {
    let tempdir = tempdir().unwrap();
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "bashkit", "0.2.1");
    let registry = registry_path(tempdir.path(), "bashkit", &[("0.2.1", &archive, &sha256)]);
    let script_path = tempdir.path().join("args.sh");
    let canonical_script_path = fs::canonicalize(tempdir.path()).unwrap().join("args.sh");
    fs::write(
        &script_path,
        "#!/usr/bin/env bashkit\nprintf '%s|%s|%s\\n' \"$0\" \"$1\" \"$2\"\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path())
        .args(["run", "--shell", "bashkit", "args.sh", "--", "one", "two"]);
    cmd.assert()
        .success()
        .stdout(format!("{}|one|two\n", canonical_script_path.display()));
}

#[test]
fn run_bashkit_stdin_preserves_script_args() {
    let tempdir = tempdir().unwrap();
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "bashkit", "0.2.1");
    let registry = registry_path(tempdir.path(), "bashkit", &[("0.2.1", &archive, &sha256)]);

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path())
        .args(["run", "--shell", "bashkit", "-", "--", "one", "two"])
        .write_stdin("printf '%s|%s\\n' \"$1\" \"$2\"\n");
    cmd.assert().success().stdout("one|two\n");
}

#[test]
fn run_bashkit_large_stdin_script_avoids_argv_limits() {
    let tempdir = tempdir().unwrap();
    let (archive, sha256) = fake_shell_archive(tempdir.path(), "bashkit", "0.2.1");
    let registry = registry_path(tempdir.path(), "bashkit", &[("0.2.1", &archive, &sha256)]);

    let mut source = String::from("#");
    source.push_str(&"x".repeat(3_000_000));
    source.push_str("\nprintf '%s|%s\\n' \"$1\" \"$2\"\n");

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_runtime_env(&mut cmd, tempdir.path(), &registry);
    cmd.current_dir(tempdir.path())
        .args(["run", "--shell", "bashkit", "-", "--", "one", "two"])
        .write_stdin(source);
    cmd.assert().success().stdout("one|two\n");
}

#[test]
fn install_list_shows_newest_version_first() {
    let tempdir = tempdir().unwrap();
    let (archive_a, sha_a) = fake_shell_archive(tempdir.path(), "bash", "5.1.16");
    let (archive_b, sha_b) = fake_shell_archive(tempdir.path(), "bash", "5.2.21");
    let registry = registry_path(
        tempdir.path(),
        "bash",
        &[
            ("5.1.16", &archive_a, &sha_a),
            ("5.2.21", &archive_b, &sha_b),
        ],
    );

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
    let registry = registry_path(tempdir.path(), "bash", &[("5.2.21", &archive, &sha256)]);
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

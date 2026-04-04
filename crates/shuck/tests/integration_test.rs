use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn help_shows_commands() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("check"))
        .stdout(predicate::str::contains("clean"));
}

#[test]
fn check_good_file_succeeds() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert().success().stdout("");
}

#[test]
fn check_broken_file_reports_parse_error() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("broken.sh:2:"))
        .stdout(predicate::str::contains("parse error"));
}

#[test]
fn check_skips_ignored_directories_when_defaulting_to_current_directory() {
    let tempdir = tempdir().unwrap();
    fs::create_dir_all(tempdir.path().join(".git")).unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();
    fs::write(
        tempdir.path().join(".git").join("broken.sh"),
        "#!/bin/bash\nif true\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert().success().stdout("");
}

#[test]
fn check_no_cache_does_not_write_cache_tree() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .args(["check", "--no-cache"]);
    cmd.assert().success();

    assert!(!tempdir.path().join(".shuck_cache").exists());
}

#[test]
fn check_writes_versioned_bin_cache_file() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert().success();

    let version_dir = tempdir
        .path()
        .join(".shuck_cache")
        .join(env!("CARGO_PKG_VERSION"));
    assert!(version_dir.is_dir());

    let entries = fs::read_dir(&version_dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].path().extension().and_then(|ext| ext.to_str()),
        Some("bin")
    );
}

#[test]
fn clean_removes_existing_cache_tree() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut check = Command::cargo_bin("shuck").unwrap();
    check.current_dir(tempdir.path()).arg("check");
    check.assert().success();
    assert!(tempdir.path().join(".shuck_cache").exists());

    let mut clean = Command::cargo_bin("shuck").unwrap();
    clean.current_dir(tempdir.path()).arg("clean");
    clean
        .assert()
        .success()
        .stdout(predicate::str::contains("cache cleared"));

    assert!(!tempdir.path().join(".shuck_cache").exists());
}

#[test]
fn clean_succeeds_when_cache_tree_is_absent() {
    let tempdir = tempdir().unwrap();

    let mut clean = Command::cargo_bin("shuck").unwrap();
    clean.current_dir(tempdir.path()).arg("clean");
    clean
        .assert()
        .success()
        .stdout(predicate::str::contains("cache cleared"));
}

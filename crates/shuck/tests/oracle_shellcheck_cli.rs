use std::collections::BTreeSet;
use std::fs;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::tempdir;

fn shellcheck_available() -> bool {
    Command::new("shellcheck")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn run_shellcheck(args: &[&str], cwd: &std::path::Path) -> Output {
    Command::new("shellcheck")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

fn run_compat(args: &[&str], cwd: &std::path::Path) -> Output {
    Command::new(assert_cmd::cargo::cargo_bin("shuck"))
        .env("SHUCK_SHELLCHECK_COMPAT", "1")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

fn json1_codes(output: &Output) -> BTreeSet<u64> {
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    value["comments"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|entry| entry["code"].as_u64())
        .collect()
}

#[test]
#[ignore]
fn oracle_help_and_version_keep_shellcheck_shape() {
    if !shellcheck_available() {
        return;
    }

    let cwd = tempdir().unwrap();
    let help_sc = run_shellcheck(&["--help"], cwd.path());
    let help_compat = run_compat(&["--help"], cwd.path());
    assert_eq!(help_sc.status.code(), help_compat.status.code());
    let help_stdout = String::from_utf8_lossy(&help_compat.stdout);
    assert!(help_stdout.contains("Usage: shellcheck"));
    assert!(help_stdout.contains("--check-sourced"));
    assert!(help_stdout.contains("--list-optional"));

    let version_sc = run_shellcheck(&["--version"], cwd.path());
    let version_compat = run_compat(&["--version"], cwd.path());
    assert_eq!(version_sc.status.code(), version_compat.status.code());
    let version_stdout = String::from_utf8_lossy(&version_compat.stdout);
    assert!(version_stdout.to_ascii_lowercase().contains("shellcheck"));
    assert!(version_stdout.to_ascii_lowercase().contains("version"));
}

#[test]
#[ignore]
fn oracle_option_acceptance_and_rejection_match_exit_codes() {
    if !shellcheck_available() {
        return;
    }

    let cwd = tempdir().unwrap();
    let unknown_sc = run_shellcheck(&["--wat"], cwd.path());
    let unknown_compat = run_compat(&["--wat"], cwd.path());
    assert_eq!(unknown_sc.status.code(), unknown_compat.status.code());

    let severity_sc = run_shellcheck(&["--severity=banana"], cwd.path());
    let severity_compat = run_compat(&["--severity=banana"], cwd.path());
    assert_eq!(severity_sc.status.code(), severity_compat.status.code());
}

#[test]
#[ignore]
fn oracle_config_precedence_matches_for_simple_disable_cases() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join(".shellcheckrc"), "disable=SC2086\n").unwrap();
    fs::write(tempdir.path().join("alt.rc"), "disable=SC2154\n").unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let searched_sc = run_shellcheck(&["--norc", "-f", "json1", "x.sh"], tempdir.path());
    let searched_compat = run_compat(&["--norc", "-f", "json1", "x.sh"], tempdir.path());
    assert_eq!(json1_codes(&searched_sc), json1_codes(&searched_compat));

    let rcfile_sc = run_shellcheck(
        &["--rcfile", "alt.rc", "-f", "json1", "x.sh"],
        tempdir.path(),
    );
    let rcfile_compat = run_compat(
        &["--rcfile", "alt.rc", "-f", "json1", "x.sh"],
        tempdir.path(),
    );
    assert_eq!(json1_codes(&rcfile_sc), json1_codes(&rcfile_compat));
}

#[test]
#[ignore]
fn oracle_output_formats_stay_structurally_valid() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let json_sc = run_shellcheck(&["--norc", "-f", "json", "x.sh"], tempdir.path());
    let json_compat = run_compat(&["--norc", "-f", "json", "x.sh"], tempdir.path());
    assert_eq!(json_sc.status.code(), json_compat.status.code());
    serde_json::from_slice::<Value>(&json_compat.stdout).unwrap();

    let json1_sc = run_shellcheck(&["--norc", "-f", "json1", "x.sh"], tempdir.path());
    let json1_compat = run_compat(&["--norc", "-f", "json1", "x.sh"], tempdir.path());
    assert_eq!(json1_sc.status.code(), json1_compat.status.code());
    serde_json::from_slice::<Value>(&json1_compat.stdout).unwrap();

    let checkstyle_sc = run_shellcheck(&["--norc", "-f", "checkstyle", "x.sh"], tempdir.path());
    let checkstyle_compat = run_compat(&["--norc", "-f", "checkstyle", "x.sh"], tempdir.path());
    assert_eq!(checkstyle_sc.status.code(), checkstyle_compat.status.code());
    let checkstyle_stdout = String::from_utf8_lossy(&checkstyle_compat.stdout);
    assert!(checkstyle_stdout.starts_with("<?xml"));
    assert!(checkstyle_stdout.contains("<checkstyle"));

    let quiet_sc = run_shellcheck(&["--norc", "-f", "quiet", "x.sh"], tempdir.path());
    let quiet_compat = run_compat(&["--norc", "-f", "quiet", "x.sh"], tempdir.path());
    assert_eq!(quiet_sc.status.code(), quiet_compat.status.code());
    assert!(quiet_compat.stdout.is_empty());

    let tty_sc = run_shellcheck(&["--norc", "-f", "tty", "x.sh"], tempdir.path());
    let tty_compat = run_compat(&["--norc", "-f", "tty", "x.sh"], tempdir.path());
    assert_eq!(tty_sc.status.code(), tty_compat.status.code());
    let tty_stdout = String::from_utf8_lossy(&tty_compat.stdout);
    assert!(tty_stdout.contains("SC2154"));
    assert!(tty_stdout.contains("For more information"));
}

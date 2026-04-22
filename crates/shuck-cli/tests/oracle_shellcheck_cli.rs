use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

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

fn run_with_stdin(mut command: Command, stdin: &str) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn run_shellcheck_stdin(args: &[&str], cwd: &Path, stdin: &str) -> Output {
    let mut command = Command::new("shellcheck");
    command.args(args).current_dir(cwd);
    run_with_stdin(command, stdin)
}

fn run_compat_stdin(args: &[&str], cwd: &Path, stdin: &str) -> Output {
    let mut command = Command::new(assert_cmd::cargo::cargo_bin("shuck"));
    command
        .env("SHUCK_SHELLCHECK_COMPAT", "1")
        .args(args)
        .current_dir(cwd);
    run_with_stdin(command, stdin)
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

fn json1_comments(output: &Output) -> Vec<Value> {
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["comments"]
        .as_array()
        .unwrap()
        .clone()
}

fn json1_comment_shapes(output: &Output) -> Vec<Value> {
    json1_comments(output)
        .into_iter()
        .map(|mut comment| {
            comment.as_object_mut().unwrap().remove("message");
            comment
        })
        .collect()
}

fn comment_by_code(comments: &[Value], code: u64) -> &Value {
    comments
        .iter()
        .find(|comment| comment["code"].as_u64() == Some(code))
        .unwrap()
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
fn oracle_unknown_sc_selectors_match_shellcheck_behavior() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let include_args = vec!["--norc", "--include=SC9999", "-f", "json1", "x.sh"];
    let include_sc = run_shellcheck(&include_args, tempdir.path());
    let include_compat = run_compat(&include_args, tempdir.path());
    assert_eq!(include_sc.status.code(), include_compat.status.code());
    assert_eq!(
        json1_comment_shapes(&include_sc),
        json1_comment_shapes(&include_compat)
    );

    let exclude_args = vec!["--norc", "--exclude=SC9999", "-f", "json1", "x.sh"];
    let exclude_sc = run_shellcheck(&exclude_args, tempdir.path());
    let exclude_compat = run_compat(&exclude_args, tempdir.path());
    assert_eq!(exclude_sc.status.code(), exclude_compat.status.code());
    assert_eq!(
        json1_comment_shapes(&exclude_sc),
        json1_comment_shapes(&exclude_compat)
    );
}

#[test]
#[ignore]
fn oracle_enable_checks_match_shellcheck_exit_codes() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let enable_all_args = vec!["--norc", "--enable=all", "-f", "json1", "x.sh"];
    let shellcheck_enable_all = run_shellcheck(&enable_all_args, tempdir.path());
    let compat_enable_all = run_compat(&enable_all_args, tempdir.path());
    assert_eq!(
        shellcheck_enable_all.status.code(),
        compat_enable_all.status.code(),
        "{enable_all_args:?}"
    );

    for args in [
        vec!["--norc", "--enable=add-default-case", "-f", "json1", "x.sh"],
        vec![
            "--norc",
            "--enable=useless-use-of-cat",
            "-f",
            "json1",
            "x.sh",
        ],
    ] {
        let shellcheck = run_shellcheck(&args, tempdir.path());
        let compat = run_compat(&args, tempdir.path());
        assert_eq!(shellcheck.status.code(), compat.status.code(), "{args:?}");
        assert_eq!(
            json1_comment_shapes(&shellcheck),
            json1_comment_shapes(&compat),
            "{args:?}"
        );
    }
}

#[test]
#[ignore]
fn oracle_check_unassigned_uppercase_matches_shellcheck() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $VAR\n").unwrap();

    let args = vec![
        "--norc",
        "--enable=check-unassigned-uppercase",
        "-f",
        "json1",
        "x.sh",
    ];
    let shellcheck = run_shellcheck(&args, tempdir.path());
    let compat = run_compat(&args, tempdir.path());
    assert_eq!(shellcheck.status.code(), compat.status.code());
    assert_eq!(
        json1_comment_shapes(&shellcheck),
        json1_comment_shapes(&compat)
    );
}

#[test]
#[ignore]
fn oracle_uppercase_optional_is_disabled_by_default() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $VAR\n").unwrap();

    let args = vec!["--norc", "-f", "json1", "x.sh"];
    let shellcheck = run_shellcheck(&args, tempdir.path());
    let compat = run_compat(&args, tempdir.path());
    assert_eq!(shellcheck.status.code(), compat.status.code());
    assert_eq!(
        json1_comment_shapes(&shellcheck),
        json1_comment_shapes(&compat)
    );
}

#[test]
#[ignore]
fn oracle_include_sc2248_does_not_select_bare_slash_marker() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\n*/\n").unwrap();

    let args = vec!["--norc", "--include=SC2248", "-f", "json1", "x.sh"];
    let shellcheck = run_shellcheck(&args, tempdir.path());
    let compat = run_compat(&args, tempdir.path());
    assert_eq!(shellcheck.status.code(), compat.status.code());
    assert_eq!(
        json1_comment_shapes(&shellcheck),
        json1_comment_shapes(&compat)
    );
}

#[test]
#[ignore]
fn oracle_include_sc2335_does_not_select_unquoted_path_in_mkdir() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("x.sh"),
        "#!/bin/bash\nmkdir -p -m 750 $PKG/var/lib/app\n",
    )
    .unwrap();

    let args = vec!["--norc", "--include=SC2335", "-f", "json1", "x.sh"];
    let shellcheck = run_shellcheck(&args, tempdir.path());
    let compat = run_compat(&args, tempdir.path());
    assert_eq!(shellcheck.status.code(), compat.status.code());
    assert_eq!(
        json1_comment_shapes(&shellcheck),
        json1_comment_shapes(&compat)
    );
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

#[test]
#[ignore]
fn oracle_json1_order_matches_shellcheck_for_shared_spans() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo $bar\n").unwrap();

    let shellcheck = run_shellcheck(&["--norc", "-f", "json1", "x.sh"], tempdir.path());
    let compat = run_compat(&["--norc", "-f", "json1", "x.sh"], tempdir.path());
    assert_eq!(shellcheck.status.code(), compat.status.code());

    let shellcheck_order = json1_comments(&shellcheck)
        .into_iter()
        .map(|comment| {
            (
                comment["code"].as_u64().unwrap(),
                comment["level"].as_str().unwrap().to_owned(),
                comment["line"].as_u64().unwrap(),
                comment["column"].as_u64().unwrap(),
                comment["endColumn"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let compat_order = json1_comments(&compat)
        .into_iter()
        .map(|comment| {
            (
                comment["code"].as_u64().unwrap(),
                comment["level"].as_str().unwrap().to_owned(),
                comment["line"].as_u64().unwrap(),
                comment["column"].as_u64().unwrap(),
                comment["endColumn"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(shellcheck_order, compat_order);
}

#[test]
#[ignore]
fn oracle_json1_sc2086_fix_shape_matches_shellcheck() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("x.sh"),
        "#!/bin/sh\nprintf %s prefix${name}suffix\n",
    )
    .unwrap();

    let shellcheck = run_shellcheck(&["--norc", "-f", "json1", "x.sh"], tempdir.path());
    let compat = run_compat(&["--norc", "-f", "json1", "x.sh"], tempdir.path());
    assert_eq!(shellcheck.status.code(), compat.status.code());

    let shellcheck_fix = comment_by_code(&json1_comments(&shellcheck), 2086)["fix"].clone();
    let compat_fix = comment_by_code(&json1_comments(&compat), 2086)["fix"].clone();
    assert_eq!(shellcheck_fix, compat_fix);
}

#[test]
#[ignore]
fn oracle_diff_behavior_matches_shellcheck_for_fixable_and_unfixable_inputs() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("fixable.sh"), "#!/bin/sh\necho $foo\n").unwrap();
    fs::write(
        tempdir.path().join("unfixable.sh"),
        "#!/bin/bash\nprintf '%s\\n' x &;\n",
    )
    .unwrap();

    let shellcheck_fixable =
        run_shellcheck(&["--norc", "-f", "diff", "fixable.sh"], tempdir.path());
    let compat_fixable = run_compat(&["--norc", "-f", "diff", "fixable.sh"], tempdir.path());
    assert_eq!(
        shellcheck_fixable.status.code(),
        compat_fixable.status.code()
    );
    assert_eq!(shellcheck_fixable.stdout, compat_fixable.stdout);

    let shellcheck_unfixable =
        run_shellcheck(&["--norc", "-f", "diff", "unfixable.sh"], tempdir.path());
    let compat_unfixable = run_compat(&["--norc", "-f", "diff", "unfixable.sh"], tempdir.path());
    assert_eq!(
        shellcheck_unfixable.status.code(),
        compat_unfixable.status.code()
    );
    assert_eq!(shellcheck_unfixable.stdout, compat_unfixable.stdout);
}

#[test]
#[ignore]
fn oracle_stdin_json1_shape_matches_shellcheck() {
    if !shellcheck_available() {
        return;
    }

    let tempdir = tempdir().unwrap();
    let stdin = "#!/bin/sh\necho $foo\n";

    let shellcheck = run_shellcheck_stdin(&["--norc", "-f", "json1", "-"], tempdir.path(), stdin);
    let compat = run_compat_stdin(&["--norc", "-f", "json1", "-"], tempdir.path(), stdin);
    assert_eq!(shellcheck.status.code(), compat.status.code());
    assert_eq!(
        json1_comment_shapes(&shellcheck),
        json1_comment_shapes(&compat)
    );
}

use std::fs;
use std::path::Path;
use std::process::Output;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::tempdir;

fn compat_cmd() -> Command {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.env("SHUCK_SHELLCHECK_COMPAT", "1");
    cmd
}

fn run_compat(args: &[&str], cwd: &Path) -> Output {
    compat_cmd().current_dir(cwd).args(args).output().unwrap()
}

fn json1_comments(output: &Output) -> Vec<Value> {
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["comments"]
        .as_array()
        .unwrap()
        .clone()
}

fn comment_by_code(comments: &[Value], code: u64) -> &Value {
    comments
        .iter()
        .find(|comment| comment["code"].as_u64() == Some(code))
        .unwrap()
}

#[test]
fn env_activation_uses_shellcheck_surface() {
    let mut cmd = compat_cmd();
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ShellCheck compatibility mode"))
        .stdout(predicate::str::contains("engine: shuck"));
}

#[cfg(unix)]
#[test]
fn argv0_basename_shellcheck_activates_compat_mode() {
    use std::os::unix::fs::symlink;

    let tempdir = tempdir().unwrap();
    let link = tempdir.path().join("shellcheck");
    symlink(assert_cmd::cargo::cargo_bin("shuck"), &link).unwrap();

    let mut cmd = Command::new(&link);
    cmd.arg("--version");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ShellCheck compatibility mode"));
}

#[test]
fn plain_shuck_help_stays_on_existing_cli() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("Shell checker CLI for shuck"))
        .stdout(predicate::str::contains("ShellCheck compatibility mode").not());
}

#[test]
fn compat_reads_shellcheckrc_from_cwd() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join(".shellcheckrc"), "disable=SC2086\n").unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let mut cmd = compat_cmd();
    cmd.current_dir(tempdir.path())
        .args(["-f", "json1", "x.sh"]);
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("\"code\":2154"))
        .stdout(predicate::str::contains("\"code\":2086").not());
}

#[test]
fn compat_norc_disables_shellcheckrc_search() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join(".shellcheckrc"), "disable=SC2086\n").unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let mut cmd = compat_cmd();
    cmd.current_dir(tempdir.path())
        .args(["--norc", "-f", "json1", "x.sh"]);
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("\"code\":2154"))
        .stdout(predicate::str::contains("\"code\":2086"));
}

#[test]
fn compat_rcfile_overrides_searched_config() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join(".shellcheckrc"), "disable=SC2086\n").unwrap();
    fs::write(tempdir.path().join("alt.rc"), "disable=SC2154\n").unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let mut cmd = compat_cmd();
    cmd.current_dir(tempdir.path())
        .args(["--rcfile", "alt.rc", "-f", "json1", "x.sh"]);
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("\"code\":2086"))
        .stdout(predicate::str::contains("\"code\":2154").not());
}

#[test]
fn compat_include_and_severity_filter_work_on_sc_codes() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let mut include = compat_cmd();
    include
        .current_dir(tempdir.path())
        .args(["-f", "json1", "--include=SC2086", "x.sh"]);
    include
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"code\":2086"))
        .stdout(predicate::str::contains("\"code\":2154").not());

    let mut severity = compat_cmd();
    severity
        .current_dir(tempdir.path())
        .args(["-f", "json1", "--severity=warning", "x.sh"]);
    severity
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"code\":2154"))
        .stdout(predicate::str::contains("\"code\":2086").not());
}

#[test]
fn compat_include_unknown_sc_code_is_accepted_as_empty_selection() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let output = run_compat(
        ["--norc", "-f", "json1", "--include=SC9999", "x.sh"].as_slice(),
        tempdir.path(),
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(json1_comments(&output), Vec::<Value>::new());
}

#[test]
fn compat_exclude_unknown_sc_code_is_ignored() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let output = run_compat(
        ["--norc", "-f", "json1", "--exclude=SC9999", "x.sh"].as_slice(),
        tempdir.path(),
    );
    assert_eq!(output.status.code(), Some(1));

    let ordered_codes = json1_comments(&output)
        .into_iter()
        .map(|comment| comment["code"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ordered_codes, vec![2154, 2086]);
}

#[test]
fn compat_accepts_busybox_shell_alias() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "echo hi\n").unwrap();

    let mut cmd = compat_cmd();
    cmd.current_dir(tempdir.path())
        .args(["-s", "busybox", "-f", "json1", "x.sh"]);
    cmd.assert().success();
}

#[test]
fn compat_list_optional_prints_catalog() {
    let mut cmd = compat_cmd();
    cmd.arg("--list-optional");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("name:    add-default-case"))
        .stdout(predicate::str::contains("name:    useless-use-of-cat"));
}

#[test]
fn compat_check_sourced_reports_resolved_source_diagnostics() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("main.sh"), "#!/bin/sh\n. ./lib.sh\n").unwrap();
    fs::write(tempdir.path().join("lib.sh"), "echo $foo\n").unwrap();

    let mut cmd = compat_cmd();
    cmd.current_dir(tempdir.path())
        .args(["-a", "-f", "json1", "main.sh"]);
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("\"file\":\"lib.sh\""))
        .stdout(predicate::str::contains("\"code\":2086"));
}

#[test]
fn compat_color_flags_do_not_consume_filename() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let mut long = compat_cmd();
    long.current_dir(tempdir.path())
        .args(["--color", "x.sh", "-f", "json1"]);
    long.assert()
        .code(1)
        .stdout(predicate::str::contains("\"code\":2086"));

    let mut short = compat_cmd();
    short
        .current_dir(tempdir.path())
        .args(["-C", "x.sh", "-f", "json1"]);
    short
        .assert()
        .code(1)
        .stdout(predicate::str::contains("\"code\":2086"));
}

#[test]
fn compat_dash_path_reads_from_stdin() {
    let mut cmd = compat_cmd();
    cmd.args(["-f", "json1", "-"])
        .write_stdin("#!/bin/sh\necho $foo\n");
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("\"file\":\"-\""))
        .stdout(predicate::str::contains("\"code\":2086"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn compat_enable_all_is_accepted() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let output = run_compat(
        ["--norc", "--enable=all", "-f", "json1", "x.sh"].as_slice(),
        tempdir.path(),
    );
    assert_eq!(output.status.code(), Some(1));

    let comments = json1_comments(&output);
    assert!(
        comments
            .iter()
            .any(|comment| comment["code"].as_u64() == Some(2086))
    );
    assert!(
        comments
            .iter()
            .any(|comment| comment["code"].as_u64() == Some(2154))
    );
}

#[test]
fn compat_enable_all_includes_check_unassigned_uppercase() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $VAR\n").unwrap();

    let output = run_compat(
        ["--norc", "--enable=all", "-f", "json1", "x.sh"].as_slice(),
        tempdir.path(),
    );
    assert_eq!(output.status.code(), Some(1));

    let ordered_codes = json1_comments(&output)
        .into_iter()
        .map(|comment| comment["code"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ordered_codes, vec![2154, 2086]);
}

#[test]
fn compat_accepts_named_unimplemented_optional_checks_as_noops() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    for check in ["add-default-case", "useless-use-of-cat"] {
        let output = run_compat(
            ["--norc", "--enable", check, "-f", "json1", "x.sh"].as_slice(),
            tempdir.path(),
        );
        assert_eq!(output.status.code(), Some(1), "optional check {check}");

        let comments = json1_comments(&output);
        assert!(
            comments
                .iter()
                .any(|comment| comment["code"].as_u64() == Some(2086))
        );
        assert!(
            comments
                .iter()
                .any(|comment| comment["code"].as_u64() == Some(2154))
        );
    }
}

#[test]
fn compat_check_unassigned_uppercase_enables_sc2154_on_uppercase_names() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $VAR\n").unwrap();

    let output = run_compat(
        [
            "--norc",
            "--enable=check-unassigned-uppercase",
            "-f",
            "json1",
            "x.sh",
        ]
        .as_slice(),
        tempdir.path(),
    );
    assert_eq!(output.status.code(), Some(1));

    let ordered_codes = json1_comments(&output)
        .into_iter()
        .map(|comment| comment["code"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ordered_codes, vec![2154, 2086]);
}

#[test]
fn compat_json1_orders_higher_severity_before_lower_severity_at_same_span() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let output = run_compat(["--norc", "-f", "json1", "x.sh"].as_slice(), tempdir.path());
    assert_eq!(output.status.code(), Some(1));

    let comments = json1_comments(&output);
    let ordered_codes = comments
        .iter()
        .map(|comment| comment["code"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(ordered_codes, vec![2154, 2086]);
}

#[test]
fn compat_json1_emits_sc2086_fix_payload_for_plain_expansions() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let output = run_compat(["--norc", "-f", "json1", "x.sh"].as_slice(), tempdir.path());
    assert_eq!(output.status.code(), Some(1));

    let comments = json1_comments(&output);
    let sc2086 = comment_by_code(&comments, 2086);
    let replacements = sc2086["fix"]["replacements"].as_array().unwrap();
    assert_eq!(replacements.len(), 2);
    assert_eq!(replacements[0]["column"].as_u64(), Some(6));
    assert_eq!(replacements[0]["endColumn"].as_u64(), Some(6));
    assert_eq!(replacements[0]["insertionPoint"].as_str(), Some("afterEnd"));
    assert_eq!(replacements[0]["replacement"].as_str(), Some("\""));
    assert_eq!(replacements[1]["column"].as_u64(), Some(10));
    assert_eq!(replacements[1]["endColumn"].as_u64(), Some(10));
    assert_eq!(
        replacements[1]["insertionPoint"].as_str(),
        Some("beforeStart")
    );
    assert_eq!(replacements[1]["replacement"].as_str(), Some("\""));
}

#[test]
fn compat_json1_emits_sc2086_fix_payload_for_mixed_words() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("x.sh"),
        "#!/bin/sh\nprintf %s prefix${name}suffix\n",
    )
    .unwrap();

    let output = run_compat(["--norc", "-f", "json1", "x.sh"].as_slice(), tempdir.path());
    assert_eq!(output.status.code(), Some(1));

    let comments = json1_comments(&output);
    let sc2086 = comment_by_code(&comments, 2086);
    let replacements = sc2086["fix"]["replacements"].as_array().unwrap();
    assert_eq!(replacements.len(), 2);
    assert_eq!(replacements[0]["column"].as_u64(), Some(17));
    assert_eq!(replacements[0]["endColumn"].as_u64(), Some(17));
    assert_eq!(replacements[0]["insertionPoint"].as_str(), Some("afterEnd"));
    assert_eq!(replacements[1]["column"].as_u64(), Some(24));
    assert_eq!(replacements[1]["endColumn"].as_u64(), Some(24));
    assert_eq!(
        replacements[1]["insertionPoint"].as_str(),
        Some("beforeStart")
    );
}

#[test]
fn compat_diff_reports_when_diagnostics_are_not_auto_fixable() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("x.sh"),
        "#!/bin/bash\nprintf '%s\\n' x &;\n",
    )
    .unwrap();

    let output = run_compat(["--norc", "-f", "diff", "x.sh"].as_slice(), tempdir.path());
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.is_empty());
    assert_eq!(
        stderr,
        "Issues were detected, but none were auto-fixable. Use another format to see them.\n"
    );
    assert!(!stderr.contains("@@ compatibility mode @@"));
    assert!(!stderr.contains("--- x.sh"));
}

#[test]
fn compat_diff_emits_unified_patch_for_fixable_sc2086() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("x.sh"), "#!/bin/sh\necho $foo\n").unwrap();

    let output = run_compat(["--norc", "-f", "diff", "x.sh"].as_slice(), tempdir.path());
    assert_eq!(output.status.code(), Some(1));

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--- a/x.sh"));
    assert!(stdout.contains("+++ b/x.sh"));
    assert!(stdout.contains("-echo $foo"));
    assert!(stdout.contains("+echo \"$foo\""));
}

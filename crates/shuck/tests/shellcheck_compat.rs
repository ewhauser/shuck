use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn compat_cmd() -> Command {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.env("SHUCK_SHELLCHECK_COMPAT", "1");
    cmd
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

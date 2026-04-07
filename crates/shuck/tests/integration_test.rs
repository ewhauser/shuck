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
        .stdout(predicate::str::contains("format"))
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
fn format_good_file_succeeds_and_preserves_contents() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("ok.sh");
    let source = "#!/bin/bash\necho ok\n";
    fs::write(&script, source).unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path()).arg("format");
    cmd.assert().success().stdout("");

    assert_eq!(fs::read_to_string(script).unwrap(), source);
}

#[test]
fn format_check_and_diff_are_clean_for_valid_input() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut check = Command::cargo_bin("shuck").unwrap();
    check
        .current_dir(tempdir.path())
        .args(["format", "--check"]);
    check.assert().success().stdout("");

    let mut diff = Command::cargo_bin("shuck").unwrap();
    diff.current_dir(tempdir.path()).args(["format", "--diff"]);
    diff.assert().success().stdout("");
}

#[test]
fn format_check_and_diff_report_changes_for_noncanonical_input() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("fn.sh"), "foo(){\necho hi\n}\n").unwrap();

    let mut check = Command::cargo_bin("shuck").unwrap();
    check
        .current_dir(tempdir.path())
        .args(["format", "--check", "--function-next-line"]);
    check.assert().code(1).stdout("");

    let mut diff = Command::cargo_bin("shuck").unwrap();
    diff.current_dir(tempdir.path())
        .args(["format", "--diff", "--function-next-line"]);
    diff.assert()
        .code(1)
        .stdout(predicate::str::contains("--- a/fn.sh"))
        .stdout(predicate::str::contains("+++ b/fn.sh"));
}

#[test]
fn format_broken_file_reports_parse_error() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path()).arg("format");
    cmd.assert()
        .code(2)
        .stdout(predicate::str::contains("broken.sh:2:"))
        .stdout(predicate::str::contains("parse error"));
}

#[test]
fn format_stdin_round_trips_valid_input() {
    let source = "#!/bin/bash\necho ok\n";

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.args(["format", "-"]).write_stdin(source);
    cmd.assert().success().stdout(source);
}

#[test]
fn format_stdin_filename_reports_parse_error_with_filename() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.args(["format", "--stdin-filename", "foo.sh"])
        .write_stdin("#!/bin/bash\nif true\n");
    cmd.assert()
        .code(2)
        .stdout(predicate::str::contains("foo.sh:2:"))
        .stdout(predicate::str::contains("parse error"));
}

#[test]
fn format_stdin_uses_current_project_config() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[format]\nfunction-next-line = true\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .args(["format", "-"])
        .write_stdin("foo(){\necho hi\n}\n");
    cmd.assert().success().stdout("foo()\n{\n\techo hi\n}\n");
}

#[test]
fn format_stdin_filename_uses_inferred_posix_dialect() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.args(["format", "--stdin-filename", "script.sh"])
        .write_stdin("[[ foo == bar ]]\n");
    cmd.assert()
        .code(2)
        .stdout(predicate::str::contains("script.sh:1:"))
        .stdout(predicate::str::contains("[[ ]] conditionals"));
}

#[test]
fn format_stdin_filename_infers_remaining_common_shell_extensions() {
    for path in ["script.bash", "script.mksh"] {
        let mut cmd = Command::cargo_bin("shuck").unwrap();
        cmd.args(["format", "--stdin-filename", path])
            .write_stdin("[[ foo == bar ]]\n");
        cmd.assert().success().stdout("[[ foo == bar ]]\n");
    }

    for path in ["script.ksh", "script.dash"] {
        let mut cmd = Command::cargo_bin("shuck").unwrap();
        cmd.args(["format", "--stdin-filename", path])
            .write_stdin("[[ foo == bar ]]\n");
        cmd.assert()
            .code(2)
            .stdout(predicate::str::contains(format!("{path}:1:")))
            .stdout(predicate::str::contains("[[ ]] conditionals"));
    }
}

#[test]
fn format_stdin_filename_infers_zsh_dialect() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.args(["format", "--stdin-filename", "script.zsh"])
        .write_stdin("print ${(m)foo}\n");
    cmd.assert().success().stdout("print ${(m)foo}\n");
}

#[test]
fn format_stdin_uses_configured_zsh_dialect() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[format]\ndialect = \"zsh\"\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .args(["format", "-"])
        .write_stdin("print ${(m)foo}\n");
    cmd.assert().success().stdout("print ${(m)foo}\n");
}

#[test]
fn check_zsh_extension_parses_with_inferred_zsh_dialect() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.zsh"), "foo=bar\nprint ${(m)foo}\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert().success().stdout("");
}

#[test]
fn check_zsh_shebang_parses_with_inferred_zsh_dialect() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("ok"),
        "#!/usr/bin/env zsh\nfoo=bar\nprint ${(m)foo}\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert().success().stdout("");
}

#[test]
fn format_exclude_skips_walked_files_but_not_explicit_files_without_force_exclude() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();
    fs::write(tempdir.path().join("ignored.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut walked = Command::cargo_bin("shuck").unwrap();
    walked
        .current_dir(tempdir.path())
        .args(["format", "--exclude", "ignored.sh"]);
    walked.assert().success().stdout("");

    let mut explicit = Command::cargo_bin("shuck").unwrap();
    explicit
        .current_dir(tempdir.path())
        .args(["format", "--exclude", "ignored.sh", "ignored.sh"]);
    explicit
        .assert()
        .code(2)
        .stdout(predicate::str::contains("ignored.sh:2:"));

    let mut forced = Command::cargo_bin("shuck").unwrap();
    forced.current_dir(tempdir.path()).args([
        "format",
        "--exclude",
        "ignored.sh",
        "--force-exclude",
        "ignored.sh",
    ]);
    forced.assert().success().stdout("");
}

#[test]
fn format_gitignore_and_force_exclude_flags_control_explicit_files() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join(".gitignore"), "ignored.sh\n").unwrap();
    fs::write(tempdir.path().join("ignored.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut default_walk = Command::cargo_bin("shuck").unwrap();
    default_walk.current_dir(tempdir.path()).arg("format");
    default_walk.assert().success().stdout("");

    let mut no_respect = Command::cargo_bin("shuck").unwrap();
    no_respect
        .current_dir(tempdir.path())
        .args(["format", "--no-respect-gitignore"]);
    no_respect
        .assert()
        .code(2)
        .stdout(predicate::str::contains("ignored.sh:2:"));

    let mut explicit = Command::cargo_bin("shuck").unwrap();
    explicit
        .current_dir(tempdir.path())
        .args(["format", "ignored.sh"]);
    explicit
        .assert()
        .code(2)
        .stdout(predicate::str::contains("ignored.sh:2:"));

    let mut forced = Command::cargo_bin("shuck").unwrap();
    forced
        .current_dir(tempdir.path())
        .args(["format", "--force-exclude", "ignored.sh"]);
    forced.assert().success().stdout("");
}

#[test]
fn format_honors_project_config_and_cli_overrides_it() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[format]\nfunction-next-line = false\n",
    )
    .unwrap();
    let script = tempdir.path().join("fn.sh");
    fs::write(&script, "foo(){\necho hi\n}\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .args(["format", "--function-next-line"]);
    cmd.assert().success().stdout("");

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "foo()\n{\n\techo hi\n}\n"
    );
}

#[test]
fn format_prefers_nested_project_config_for_explicit_files() {
    let tempdir = tempdir().unwrap();
    let nested = tempdir.path().join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[format]\nfunction-next-line = false\n",
    )
    .unwrap();
    fs::write(
        nested.join("shuck.toml"),
        "[format]\nfunction-next-line = true\n",
    )
    .unwrap();
    let script = nested.join("fn.sh");
    fs::write(&script, "foo(){\necho hi\n}\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .args(["format", "nested/fn.sh"]);
    cmd.assert().success().stdout("");

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "foo()\n{\n\techo hi\n}\n"
    );
}

#[test]
fn format_cache_invalidates_when_formatter_options_change() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("fn.sh"), "foo(){\necho hi\n}\n").unwrap();

    let mut initial = Command::cargo_bin("shuck").unwrap();
    initial.current_dir(tempdir.path()).arg("format");
    initial.assert().success().stdout("");

    let mut check = Command::cargo_bin("shuck").unwrap();
    check
        .current_dir(tempdir.path())
        .args(["format", "--check", "--function-next-line"]);
    check.assert().code(1).stdout("");
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

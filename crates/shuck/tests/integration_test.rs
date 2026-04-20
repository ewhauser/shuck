use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;
use walkdir::WalkDir;

fn cache_dir(root: &Path) -> PathBuf {
    root.join("shared-cache")
}

fn configure_env_cache(cmd: &mut Command, root: &Path) {
    cmd.env("SHUCK_CACHE_DIR", cache_dir(root));
}

fn configure_default_cache_env(cmd: &mut Command, root: &Path) {
    let home = root.join("home");
    let xdg_cache = root.join("xdg-cache");
    let appdata = root.join("appdata").join("Roaming");
    let local_appdata = root.join("appdata").join("Local");

    cmd.env_remove("SHUCK_CACHE_DIR");
    cmd.env("HOME", &home);
    cmd.env("USERPROFILE", &home);
    cmd.env("XDG_CACHE_HOME", xdg_cache);
    cmd.env("APPDATA", appdata);
    cmd.env("LOCALAPPDATA", local_appdata);
}

fn enable_experimental(cmd: &mut Command) {
    cmd.env("SHUCK_EXPERIMENTAL", "1");
}

#[test]
fn help_shows_commands() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("check"))
        .stdout(predicate::str::contains("Format shell files").not())
        .stdout(predicate::str::contains("clean"));
}

#[test]
fn help_shows_format_when_experimental_enabled() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    enable_experimental(&mut cmd);
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("check"))
        .stdout(predicate::str::contains("Format shell files"))
        .stdout(predicate::str::contains("clean"));
}

#[test]
fn format_subcommand_requires_experimental_env() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.arg("format");
    cmd.assert().code(2).stderr(predicate::str::contains(
        "the `format` subcommand is experimental; set SHUCK_EXPERIMENTAL=1 to enable it",
    ));
}

#[test]
fn check_good_file_succeeds() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert().success().stdout("");
}

#[test]
fn check_unterminated_quote_reports_parse_error() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("broken.sh"),
        "#!/bin/bash\necho \"unterminated\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("error[parse-error]:"))
        .stdout(predicate::str::contains("--> broken.sh:2:6"))
        .stdout(predicate::str::contains("2 | echo \"unterminated"))
        .stdout(predicate::str::contains("^"));
}

#[test]
fn check_missing_then_reports_c064_lint() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("warning[C064]:"))
        .stdout(predicate::str::contains("--> broken.sh:2:1"))
        .stdout(predicate::str::contains("2 | if true"))
        .stdout(predicate::str::contains("| ^"));
}

#[test]
fn check_reports_lint_with_full_snippet_by_default() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("warn.sh"),
        "#!/bin/bash\nunused=1\necho ok\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("warning[C001]:"))
        .stdout(predicate::str::contains("--> warn.sh:2:1"))
        .stdout(predicate::str::contains("2 | unused=1"))
        .stdout(predicate::str::contains("| ^"));
}

#[test]
fn check_concise_output_preserves_legacy_one_line_format() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("warn.sh"),
        "#!/bin/bash\nunused=1\necho ok\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--output-format", "concise"]);
    cmd.assert()
        .code(1)
        .stdout("warn.sh:2:1: warning[C001] variable `unused` is assigned but never used\n");
}

#[test]
fn check_cache_hits_keep_full_snippet_output() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("warn.sh"),
        "#!/bin/bash\nunused=1\necho ok\n",
    )
    .unwrap();

    let mut first = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut first, tempdir.path());
    first.current_dir(tempdir.path()).arg("check");
    first.assert().code(1);

    let mut second = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut second, tempdir.path());
    second.current_dir(tempdir.path()).arg("check");
    second
        .assert()
        .code(1)
        .stdout(predicate::str::contains("--> warn.sh:2:1"))
        .stdout(predicate::str::contains("2 | unused=1"))
        .stdout(predicate::str::contains("| ^"));
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
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert().success().stdout("");
}

#[test]
fn check_no_cache_does_not_write_cache_tree() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--no-cache"]);
    cmd.assert().success();

    assert!(!tempdir.path().join(".shuck_cache").exists());
    assert!(!cache_dir(tempdir.path()).exists());
}

#[test]
fn check_writes_versioned_bin_cache_file_via_env_override() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert().success();

    let version_dir = cache_dir(tempdir.path()).join(env!("CARGO_PKG_VERSION"));
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
fn check_writes_versioned_bin_cache_file_via_cli_arg() {
    let tempdir = tempdir().unwrap();
    let cache_dir = tempdir.path().join("cli-cache");
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("check");
    cmd.assert().success();

    let version_dir = cache_dir.join(env!("CARGO_PKG_VERSION"));
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
fn check_default_cache_uses_os_cache_dir_and_not_local_tree() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_default_cache_env(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert().success();

    assert!(!tempdir.path().join(".shuck_cache").exists());
    let cache_files = WalkDir::new(tempdir.path())
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("bin"))
        .collect::<Vec<_>>();
    assert_eq!(cache_files.len(), 1);
    assert!(
        !cache_files[0]
            .path()
            .starts_with(tempdir.path().join(".shuck_cache"))
    );
}

#[test]
fn format_good_file_succeeds_and_preserves_contents() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("ok.sh");
    let source = "#!/bin/bash\necho ok\n";
    fs::write(&script, source).unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    enable_experimental(&mut cmd);
    cmd.current_dir(tempdir.path()).arg("format");
    cmd.assert().success().stdout("");

    assert_eq!(fs::read_to_string(script).unwrap(), source);
}

#[test]
fn format_check_and_diff_are_clean_for_valid_input() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut check = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut check, tempdir.path());
    enable_experimental(&mut check);
    check
        .current_dir(tempdir.path())
        .args(["format", "--check"]);
    check.assert().success().stdout("");

    let mut diff = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut diff, tempdir.path());
    enable_experimental(&mut diff);
    diff.current_dir(tempdir.path()).args(["format", "--diff"]);
    diff.assert().success().stdout("");
}

#[test]
fn format_check_and_diff_report_changes_for_noncanonical_input() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("fn.sh"), "foo(){\necho hi\n}\n").unwrap();

    let mut check = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut check, tempdir.path());
    enable_experimental(&mut check);
    check
        .current_dir(tempdir.path())
        .args(["format", "--check", "--function-next-line"]);
    check.assert().code(1).stdout("");

    let mut diff = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut diff, tempdir.path());
    enable_experimental(&mut diff);
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
    configure_env_cache(&mut cmd, tempdir.path());
    enable_experimental(&mut cmd);
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
    enable_experimental(&mut cmd);
    cmd.args(["format", "-"]).write_stdin(source);
    cmd.assert().success().stdout(source);
}

#[test]
fn format_stdin_filename_reports_parse_error_with_filename() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    enable_experimental(&mut cmd);
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
    enable_experimental(&mut cmd);
    cmd.current_dir(tempdir.path())
        .args(["format", "-"])
        .write_stdin("foo(){\necho hi\n}\n");
    cmd.assert().success().stdout("foo()\n{\n\techo hi\n}\n");
}

#[test]
fn format_stdin_filename_uses_inferred_posix_dialect() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    enable_experimental(&mut cmd);
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
        enable_experimental(&mut cmd);
        cmd.args(["format", "--stdin-filename", path])
            .write_stdin("[[ foo == bar ]]\n");
        cmd.assert().success().stdout("[[ foo == bar ]]\n");
    }

    for path in ["script.ksh", "script.dash"] {
        let mut cmd = Command::cargo_bin("shuck").unwrap();
        enable_experimental(&mut cmd);
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
    enable_experimental(&mut cmd);
    cmd.args(["format", "--stdin-filename", "script.zsh"])
        .write_stdin("print ${(m)foo}\n");
    cmd.assert().success().stdout("print ${(m)foo}\n");
}

#[test]
fn format_stdin_rejects_configured_dialect() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[format]\ndialect = \"zsh\"\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    enable_experimental(&mut cmd);
    cmd.current_dir(tempdir.path())
        .args(["format", "-"])
        .write_stdin("print ${(m)foo}\n");
    cmd.assert()
        .code(2)
        .stderr(predicate::str::contains("[format].dialect"))
        .stderr(predicate::str::contains("--dialect"));
}

#[test]
fn format_stdin_uses_cli_zsh_dialect_override() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    enable_experimental(&mut cmd);
    cmd.args(["format", "--dialect", "zsh", "-"])
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
fn check_zsh_extension_parses_repeat_and_foreach_with_inferred_zsh_dialect() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("ok.zsh"),
        "repeat 3; do echo hi; done\nforeach x (a b c) { echo $x; }\n",
    )
    .unwrap();

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
    configure_env_cache(&mut walked, tempdir.path());
    enable_experimental(&mut walked);
    walked
        .current_dir(tempdir.path())
        .args(["format", "--exclude", "ignored.sh"]);
    walked.assert().success().stdout("");

    let mut explicit = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut explicit, tempdir.path());
    enable_experimental(&mut explicit);
    explicit
        .current_dir(tempdir.path())
        .args(["format", "--exclude", "ignored.sh", "ignored.sh"]);
    explicit
        .assert()
        .code(2)
        .stdout(predicate::str::contains("ignored.sh:2:"));

    let mut forced = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut forced, tempdir.path());
    enable_experimental(&mut forced);
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
    configure_env_cache(&mut default_walk, tempdir.path());
    enable_experimental(&mut default_walk);
    default_walk.current_dir(tempdir.path()).arg("format");
    default_walk.assert().success().stdout("");

    let mut no_respect = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut no_respect, tempdir.path());
    enable_experimental(&mut no_respect);
    no_respect
        .current_dir(tempdir.path())
        .args(["format", "--no-respect-gitignore"]);
    no_respect
        .assert()
        .code(2)
        .stdout(predicate::str::contains("ignored.sh:2:"));

    let mut explicit = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut explicit, tempdir.path());
    enable_experimental(&mut explicit);
    explicit
        .current_dir(tempdir.path())
        .args(["format", "ignored.sh"]);
    explicit
        .assert()
        .code(2)
        .stdout(predicate::str::contains("ignored.sh:2:"));

    let mut forced = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut forced, tempdir.path());
    enable_experimental(&mut forced);
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
    configure_env_cache(&mut cmd, tempdir.path());
    enable_experimental(&mut cmd);
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
    configure_env_cache(&mut cmd, tempdir.path());
    enable_experimental(&mut cmd);
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
    configure_env_cache(&mut initial, tempdir.path());
    enable_experimental(&mut initial);
    initial.current_dir(tempdir.path()).arg("format");
    initial.assert().success().stdout("");

    let mut check = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut check, tempdir.path());
    enable_experimental(&mut check);
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
    configure_env_cache(&mut check, tempdir.path());
    check.current_dir(tempdir.path()).arg("check");
    check.assert().success();
    assert!(cache_dir(tempdir.path()).exists());

    let mut clean = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut clean, tempdir.path());
    clean.current_dir(tempdir.path()).arg("clean");
    clean
        .assert()
        .success()
        .stdout(predicate::str::contains("cache cleared"));

    assert!(!cache_dir(tempdir.path()).exists());
    assert!(!tempdir.path().join(".shuck_cache").exists());
}

#[test]
fn clean_succeeds_when_cache_tree_is_absent() {
    let tempdir = tempdir().unwrap();

    let mut clean = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut clean, tempdir.path());
    clean.current_dir(tempdir.path()).arg("clean");
    clean
        .assert()
        .success()
        .stdout(predicate::str::contains("cache cleared"));
}

#[test]
fn clean_removes_legacy_local_cache_directory_during_transition() {
    let tempdir = tempdir().unwrap();
    fs::create_dir_all(tempdir.path().join(".shuck_cache").join("stale")).unwrap();

    let mut clean = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut clean, tempdir.path());
    clean.current_dir(tempdir.path()).arg("clean");
    clean.assert().success();

    assert!(!tempdir.path().join(".shuck_cache").exists());
}

#[test]
fn clean_only_removes_selected_project_entries_from_shared_cache() {
    let tempdir = tempdir().unwrap();
    let cache_dir = tempdir.path().join("shared-cache");
    let project_a = tempdir.path().join("project-a");
    let project_b = tempdir.path().join("project-b");
    fs::create_dir_all(&project_a).unwrap();
    fs::create_dir_all(&project_b).unwrap();
    fs::write(project_a.join("a.sh"), "#!/bin/bash\necho a\n").unwrap();
    fs::write(project_b.join("b.sh"), "#!/bin/bash\necho b\n").unwrap();

    let mut check_a = Command::cargo_bin("shuck").unwrap();
    check_a
        .current_dir(&project_a)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("check");
    check_a.assert().success();

    let mut check_b = Command::cargo_bin("shuck").unwrap();
    check_b
        .current_dir(&project_b)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("check");
    check_b.assert().success();

    let version_dir = cache_dir.join(env!("CARGO_PKG_VERSION"));
    let initial_entries = fs::read_dir(&version_dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(initial_entries.len(), 2);

    let mut clean_a = Command::cargo_bin("shuck").unwrap();
    clean_a
        .current_dir(&project_a)
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("clean");
    clean_a.assert().success();

    let remaining_entries = fs::read_dir(&version_dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(remaining_entries.len(), 1);
}

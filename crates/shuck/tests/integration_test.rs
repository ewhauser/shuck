use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;
use wait_timeout::ChildExt;
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

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

#[derive(Default)]
struct CapturedOutput {
    stdout: String,
    stderr: String,
}

fn spawn_output_reader<R>(
    reader: R,
    kind: StreamKind,
    tx: mpsc::Sender<(StreamKind, String)>,
) -> thread::JoinHandle<()>
where
    R: std::io::Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send((kind, line)).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

fn wait_for_output<F>(
    rx: &Receiver<(StreamKind, String)>,
    captured: &mut CapturedOutput,
    timeout: Duration,
    predicate: F,
) -> bool
where
    F: Fn(&CapturedOutput) -> bool,
{
    if predicate(captured) {
        return true;
    }

    let deadline = Instant::now() + timeout;
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return predicate(captured);
        };

        match rx.recv_timeout(remaining) {
            Ok((kind, line)) => match kind {
                StreamKind::Stdout => captured.stdout.push_str(&line),
                StreamKind::Stderr => captured.stderr.push_str(&line),
            },
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                return predicate(captured);
            }
        }

        if predicate(captured) {
            return true;
        }
    }
}

fn stop_child(child: &mut std::process::Child) {
    if child.try_wait().unwrap().is_none() {
        let _ = child.kill();
    }
    if child
        .wait_timeout(Duration::from_secs(5))
        .unwrap()
        .is_none()
    {
        let _ = child.kill();
        let _ = child.wait();
    }
}

#[test]
fn help_shows_commands() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.arg("--help");
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("check"))
        .stdout(predicate::str::contains("--config <CONFIG_OPTION>"))
        .stdout(predicate::str::contains("--isolated"))
        .stdout(predicate::str::contains("--color <WHEN>"))
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
fn clean_help_describes_project_cache_entries() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.args(["clean", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "Remove shuck cache entries for the provided paths' projects",
        ))
        .stdout(predicate::str::contains(
            "Files or directories whose project caches should be removed",
        ));
}

#[test]
fn check_help_shows_file_selection_options() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.args(["check", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("File selection"))
        .stdout(predicate::str::contains("--exclude <FILE_PATTERN>"))
        .stdout(predicate::str::contains("--extend-exclude <FILE_PATTERN>"))
        .stdout(predicate::str::contains("--respect-gitignore"))
        .stdout(predicate::str::contains("--force-exclude"));
}

#[test]
fn format_help_shows_file_selection_options() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    enable_experimental(&mut cmd);
    cmd.args(["format", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("File selection"))
        .stdout(predicate::str::contains("--exclude <FILE_PATTERN>"))
        .stdout(predicate::str::contains("--extend-exclude <FILE_PATTERN>"))
        .stdout(predicate::str::contains("--respect-gitignore"))
        .stdout(predicate::str::contains("--force-exclude"));
}

#[test]
fn check_help_includes_add_ignore_flag() {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.args(["check", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("--add-ignore"))
        .stdout(predicate::str::contains("-w, --watch"))
        .stdout(predicate::str::contains("shuck ignore directives"))
        .stdout(predicate::str::contains("--add-noqa").not());
}

#[test]
fn config_file_and_isolated_conflict() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("shuck.toml"), "[format]\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .arg("--isolated")
        .arg("--config")
        .arg("shuck.toml")
        .arg("check");
    cmd.assert()
        .code(2)
        .stderr(predicate::str::contains("cannot be used with `--isolated`"));
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
fn check_fix_rewrites_safe_s074_and_bypasses_cache() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("warn.sh");
    fs::write(&script, "#!/bin/bash\nprintf '%s\\n' x &;\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).args(["check", "--fix"]);
    cmd.assert().success().stdout("");

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "#!/bin/bash\nprintf '%s\\n' x &\n"
    );
    assert!(!cache_dir(tempdir.path()).exists());
}

#[test]
fn check_without_fix_leaves_s074_file_unchanged() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("warn.sh");
    let source = "#!/bin/bash\nprintf '%s\\n' x &;\n";
    fs::write(&script, source).unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).arg("check");
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("warning[S074]:"));

    assert_eq!(fs::read_to_string(script).unwrap(), source);
}

#[test]
fn check_exit_non_zero_on_fix_returns_failure_when_fix_is_applied() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("warn.sh");
    fs::write(&script, "#!/bin/bash\nprintf '%s\\n' x &;\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--fix", "--exit-non-zero-on-fix"]);
    cmd.assert().code(1).stdout("");

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "#!/bin/bash\nprintf '%s\\n' x &\n"
    );
}

#[test]
fn check_unsafe_fixes_applies_safe_s074_fix() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("warn.sh");
    fs::write(&script, "#!/bin/bash\nprintf '%s\\n' x &;\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--unsafe-fixes"]);
    cmd.assert().success().stdout("");

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "#!/bin/bash\nprintf '%s\\n' x &\n"
    );
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
fn check_add_ignore_writes_inline_shuck_ignore() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("warn.sh");
    fs::write(&script, "#!/bin/bash\necho $foo\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--add-ignore"]);
    cmd.assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("Added 1 shuck ignore directive."));

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "#!/bin/bash\necho $foo  # shuck: ignore=C006\n"
    );
}

#[test]
fn check_rejects_add_noqa_alias() {
    let tempdir = tempdir().unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--add-noqa=legacy"]);
    cmd.assert()
        .code(2)
        .stderr(predicate::str::contains("unexpected argument '--add-noqa'"));
}

#[test]
fn check_add_ignore_merges_existing_ignore_and_preserves_reason() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("warn.sh");
    fs::write(
        &script,
        "#!/bin/bash\necho $foo  # shuck: ignore=S001 # legacy\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--add-ignore"]);
    cmd.assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("Added 1 shuck ignore directive."));

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "#!/bin/bash\necho $foo  # shuck: ignore=C006, S001 # legacy\n"
    );
}

#[test]
fn check_add_ignore_reports_remaining_parse_errors() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("broken.sh");
    fs::write(&script, "#!/bin/bash\necho \"unterminated\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--add-ignore"]);
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("error[parse-error]:"))
        .stderr(predicate::str::is_empty());

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "#!/bin/bash\necho \"unterminated\n"
    );
}

#[test]
fn check_add_ignore_leaves_uneditable_trailing_comment_lines_failing() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("warn.sh");
    let source = "#!/bin/bash\necho $foo # existing comment\n";
    fs::write(&script, source).unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--add-ignore", "--output-format", "concise"]);
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("error[C006]"))
        .stderr(predicate::str::is_empty());

    assert_eq!(fs::read_to_string(script).unwrap(), source);
}

#[test]
fn check_add_ignore_respects_exit_zero_for_warning_leftovers() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("warn.sh");
    let source = "#!/bin/bash\nunused=1 # existing comment\necho ok\n";
    fs::write(&script, source).unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).args([
        "check",
        "--add-ignore",
        "--exit-zero",
        "--output-format",
        "concise",
    ]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("warning[C001]"))
        .stderr(predicate::str::is_empty());

    assert_eq!(fs::read_to_string(script).unwrap(), source);
}

#[test]
fn check_add_ignore_leaves_continuation_lines_failing() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("warn.sh");
    let source = "#!/bin/bash\necho $foo \\\n&& echo ok\n";
    fs::write(&script, source).unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--add-ignore", "--output-format", "concise"]);
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("error[C006]"))
        .stderr(predicate::str::is_empty());

    assert_eq!(fs::read_to_string(script).unwrap(), source);
}

#[test]
fn check_add_ignore_respects_force_exclude_for_explicit_files() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("ignored.sh");
    let source = "#!/bin/bash\necho $foo\n";
    fs::write(&script, source).unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).args([
        "check",
        "--add-ignore",
        "--exclude",
        "ignored.sh",
        "--force-exclude",
        "ignored.sh",
    ]);
    cmd.assert()
        .success()
        .stdout("")
        .stderr(predicate::str::is_empty());

    assert_eq!(fs::read_to_string(script).unwrap(), source);
}

#[test]
fn inline_shuck_ignore_suppresses_only_its_own_line() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("warn.sh"),
        "#!/bin/bash\necho $foo  # shuck: ignore=C006\necho $bar\n",
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--output-format", "concise"]);
    cmd.assert()
        .code(1)
        .stdout("warn.sh:3:6: error[C006] variable `bar` is referenced before assignment\n");
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
fn check_exclude_skips_walked_files_but_not_explicit_files_without_force_exclude() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();
    fs::write(tempdir.path().join("ignored.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut walked = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut walked, tempdir.path());
    walked
        .current_dir(tempdir.path())
        .args(["check", "--exclude", "ignored.sh"]);
    walked.assert().success().stdout("");

    let mut explicit = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut explicit, tempdir.path());
    explicit
        .current_dir(tempdir.path())
        .args(["check", "--exclude", "ignored.sh", "ignored.sh"]);
    explicit
        .assert()
        .code(1)
        .stdout(predicate::str::contains("--> ignored.sh:2:1"));

    let mut forced = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut forced, tempdir.path());
    forced.current_dir(tempdir.path()).args([
        "check",
        "--exclude",
        "ignored.sh",
        "--force-exclude",
        "ignored.sh",
    ]);
    forced.assert().success().stdout("");
}

#[test]
fn check_gitignore_and_force_exclude_flags_control_explicit_files() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join(".gitignore"), "ignored.sh\n").unwrap();
    fs::write(tempdir.path().join("ignored.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut default_walk = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut default_walk, tempdir.path());
    default_walk.current_dir(tempdir.path()).arg("check");
    default_walk.assert().success().stdout("");

    let mut no_respect = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut no_respect, tempdir.path());
    no_respect
        .current_dir(tempdir.path())
        .args(["check", "--no-respect-gitignore"]);
    no_respect
        .assert()
        .code(1)
        .stdout(predicate::str::contains("--> ignored.sh:2:1"));

    let mut explicit = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut explicit, tempdir.path());
    explicit
        .current_dir(tempdir.path())
        .args(["check", "ignored.sh"]);
    explicit
        .assert()
        .code(1)
        .stdout(predicate::str::contains("--> ignored.sh:2:1"));

    let mut forced = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut forced, tempdir.path());
    forced
        .current_dir(tempdir.path())
        .args(["check", "--force-exclude", "ignored.sh"]);
    forced.assert().success().stdout("");
}

#[test]
fn check_extend_exclude_adds_to_exclude_patterns() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("base.sh"), "#!/bin/bash\nif true\n").unwrap();
    fs::write(tempdir.path().join("extra.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path()).args([
        "check",
        "--exclude",
        "base.sh",
        "--extend-exclude",
        "extra.sh",
    ]);
    cmd.assert().success().stdout("");
}

#[test]
fn check_invalid_extend_exclude_pattern_reports_discovery_error() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/bash\necho ok\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    cmd.current_dir(tempdir.path())
        .args(["check", "--extend-exclude", "["]);
    cmd.assert()
        .code(2)
        .stderr(predicate::str::contains("invalid exclude pattern `[`"));
}

#[test]
fn check_watch_reruns_when_files_change() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("watch.sh");
    fs::write(&script, "#!/bin/bash\necho ok\n").unwrap();

    let mut child = ProcessCommand::new(assert_cmd::cargo::cargo_bin("shuck"));
    child
        .env("SHUCK_CACHE_DIR", cache_dir(tempdir.path()))
        .current_dir(tempdir.path())
        .args(["check", "--watch", "--output-format", "concise"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = child.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let stdout_reader = spawn_output_reader(stdout, StreamKind::Stdout, tx.clone());
    let stderr_reader = spawn_output_reader(stderr, StreamKind::Stderr, tx);
    let mut captured = CapturedOutput::default();

    let initial_ready = wait_for_output(&rx, &mut captured, Duration::from_secs(10), |output| {
        output.stderr.contains("Starting linter in watch mode...")
    });
    if !initial_ready {
        stop_child(&mut child);
        let _ = stdout_reader.join();
        let _ = stderr_reader.join();
        panic!(
            "watch mode did not emit its startup banner\nstdout:\n{}\nstderr:\n{}",
            captured.stdout, captured.stderr
        );
    }

    fs::write(&script, "#!/bin/bash\nunused=1\necho ok\n").unwrap();

    let rerun_ready = wait_for_output(&rx, &mut captured, Duration::from_secs(10), |output| {
        output.stderr.contains("File change detected...")
            && output.stdout.contains("watch.sh:2:1: warning[C001]")
    });

    stop_child(&mut child);
    let _ = stdout_reader.join();
    let _ = stderr_reader.join();

    assert!(
        rerun_ready,
        "watch mode did not rerun after file changes\nstdout:\n{}\nstderr:\n{}",
        captured.stdout, captured.stderr
    );
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
fn format_honors_explicit_global_config_file() {
    let tempdir = tempdir().unwrap();
    let config_path = tempdir.path().join("override.toml");
    fs::write(&config_path, "[format]\nfunction-next-line = true\n").unwrap();
    let script = tempdir.path().join("fn.sh");
    fs::write(&script, "foo(){\necho hi\n}\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    enable_experimental(&mut cmd);
    cmd.current_dir(tempdir.path())
        .arg("--config")
        .arg(&config_path)
        .arg("format");
    cmd.assert().success().stdout("");

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "foo()\n{\n\techo hi\n}\n"
    );
}

#[test]
fn format_inline_global_config_override_beats_global_config_file() {
    let tempdir = tempdir().unwrap();
    let config_path = tempdir.path().join("override.toml");
    fs::write(&config_path, "[format]\nfunction-next-line = false\n").unwrap();
    let script = tempdir.path().join("fn.sh");
    fs::write(&script, "foo(){\necho hi\n}\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    enable_experimental(&mut cmd);
    cmd.current_dir(tempdir.path())
        .arg("--config")
        .arg(&config_path)
        .arg("--config")
        .arg("format.function-next-line = true")
        .arg("format");
    cmd.assert().success().stdout("");

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "foo()\n{\n\techo hi\n}\n"
    );
}

#[test]
fn format_isolated_ignores_discovered_project_config() {
    let tempdir = tempdir().unwrap();
    fs::write(
        tempdir.path().join("shuck.toml"),
        "[format]\nfunction-next-line = true\n",
    )
    .unwrap();
    let script = tempdir.path().join("fn.sh");
    fs::write(&script, "foo(){\necho hi\n}\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    configure_env_cache(&mut cmd, tempdir.path());
    enable_experimental(&mut cmd);
    cmd.current_dir(tempdir.path())
        .arg("--isolated")
        .arg("format");
    cmd.assert().success().stdout("");

    assert_eq!(
        fs::read_to_string(script).unwrap(),
        "foo() {\n\techo hi\n}\n"
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

#[test]
fn check_and_clean_share_config_root_mode_for_explicit_config_files() {
    let tempdir = tempdir().unwrap();
    let cache_dir = tempdir.path().join("shared-cache");
    let override_config = tempdir.path().join("override.toml");
    let nested = tempdir.path().join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(&override_config, "[format]\n").unwrap();
    fs::write(
        nested.join("shuck.toml"),
        "[format]\nfunction-next-line = true\n",
    )
    .unwrap();
    fs::write(nested.join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut check = Command::cargo_bin("shuck").unwrap();
    check
        .current_dir(tempdir.path())
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("--config")
        .arg(&override_config)
        .arg("check");
    check.assert().code(1);
    assert!(cache_dir.exists());

    let mut clean = Command::cargo_bin("shuck").unwrap();
    clean
        .current_dir(tempdir.path())
        .arg("--cache-dir")
        .arg(&cache_dir)
        .arg("--config")
        .arg(&override_config)
        .arg("clean");
    clean.assert().success();

    assert!(!cache_dir.exists());
}

#[test]
fn check_color_always_forces_ansi_output() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .arg("--color")
        .arg("always")
        .arg("check");
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("\u{1b}["));
}

#[test]
fn check_color_never_overrides_force_color_env() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("broken.sh"), "#!/bin/bash\nif true\n").unwrap();

    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.current_dir(tempdir.path())
        .env("FORCE_COLOR", "1")
        .arg("--color")
        .arg("never")
        .arg("check");
    cmd.assert()
        .code(1)
        .stdout(predicate::str::contains("\u{1b}[").not());
}

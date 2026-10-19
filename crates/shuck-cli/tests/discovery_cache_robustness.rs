//! CLI regressions for file discovery, per-file I/O failures, write-back, and
//! the shared cache.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::mpsc;
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

fn shuck(root: &Path, cwd: &Path) -> Command {
    let mut cmd = Command::cargo_bin("shuck").unwrap();
    cmd.env("SHUCK_CACHE_DIR", cache_dir(root)).current_dir(cwd);
    cmd
}

fn cache_files(cache_root: &Path) -> Vec<PathBuf> {
    WalkDir::new(cache_root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "bin"))
        .map(|entry| entry.into_path())
        .collect()
}

#[test]
fn ignored_directory_names_above_the_project_do_not_hide_it() {
    let tempdir = tempdir().unwrap();
    for ancestor in ["vendor", ".cache", "node_modules"] {
        let project = tempdir.path().join(ancestor).join("proj");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("broken.sh"), "echo \"$(\n").unwrap();

        shuck(tempdir.path(), &project)
            .args(["check", "--no-cache", "."])
            .assert()
            .code(1)
            .stdout(predicate::str::contains("broken.sh:1:"));

        // Explicitly named files inside an ignored directory are checked too.
        shuck(tempdir.path(), tempdir.path())
            .args(["check", "--no-cache"])
            .arg(Path::new(ancestor).join("proj/broken.sh"))
            .assert()
            .code(1)
            .stdout(predicate::str::contains("broken.sh:1:"));
    }
}

#[test]
fn ignored_directory_names_below_the_walk_root_are_still_skipped() {
    let tempdir = tempdir().unwrap();
    let vendored = tempdir.path().join("vendor");
    fs::create_dir_all(&vendored).unwrap();
    fs::write(vendored.join("broken.sh"), "echo \"$(\n").unwrap();
    fs::write(tempdir.path().join("ok.sh"), "#!/bin/sh\necho ok\n").unwrap();

    shuck(tempdir.path(), tempdir.path())
        .args(["check", "--no-cache", "."])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn unreadable_file_is_reported_with_its_path_and_the_run_continues() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("bad.sh"), b"echo \xff\xfe\n").unwrap();
    fs::write(tempdir.path().join("broken.sh"), "echo \"$(\n").unwrap();

    shuck(tempdir.path(), tempdir.path())
        .args(["check", "--no-cache", "."])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("broken.sh:1:"))
        .stderr(predicate::str::contains("bad.sh: cannot read file"));
}

#[test]
fn missing_explicit_input_is_reported_and_other_inputs_are_checked() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("broken.sh"), "echo \"$(\n").unwrap();

    shuck(tempdir.path(), tempdir.path())
        .args(["check", "--no-cache", "missing.sh", "broken.sh"])
        .assert()
        .code(2)
        .stdout(predicate::str::contains("broken.sh:1:"))
        .stderr(predicate::str::contains("missing.sh: cannot access path"));
}

#[cfg(unix)]
#[test]
fn unreadable_walked_file_does_not_abort_discovery() {
    use std::os::unix::fs::PermissionsExt;

    let tempdir = tempdir().unwrap();
    let locked = tempdir.path().join("locked");
    fs::write(&locked, "#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read(&locked).is_ok() {
        // Running with privileges that bypass file permissions.
        return;
    }
    fs::write(tempdir.path().join("broken.sh"), "echo \"$(\n").unwrap();

    let assert = shuck(tempdir.path(), tempdir.path())
        .args(["check", "--no-cache", "."])
        .assert();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    assert
        .code(2)
        .stdout(predicate::str::contains("broken.sh:1:"))
        .stderr(predicate::str::contains("locked: cannot open file"));
}

#[test]
fn format_reports_unreadable_file_and_formats_the_rest() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("bad.sh"), b"echo \xff\xfe\n").unwrap();
    let script = tempdir.path().join("script.sh");
    fs::write(&script, "#!/bin/bash\nif true;then\necho ok\nfi\n").unwrap();

    shuck(tempdir.path(), tempdir.path())
        .args(["format", "--no-cache", "."])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("bad.sh: cannot read file"));
    assert_ne!(
        fs::read_to_string(&script).unwrap(),
        "#!/bin/bash\nif true;then\necho ok\nfi\n"
    );
}

#[test]
fn watch_survives_an_input_that_vanishes_and_reappears() {
    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("watch.sh");
    fs::write(&script, "#!/bin/bash\nunused=1\necho ok\n").unwrap();

    let mut child = ProcessCommand::new(assert_cmd::cargo::cargo_bin("shuck"))
        .env("SHUCK_CACHE_DIR", cache_dir(tempdir.path()))
        .current_dir(tempdir.path())
        .args(["check", "--watch", "--output-format", "concise", "watch.sh"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (tx, rx) = mpsc::channel();
    for reader in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ] {
        let tx = tx.clone();
        thread::spawn(move || {
            for line in BufReader::new(reader).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
    }
    let mut output = String::new();
    let mut wait_for = |needle: &str, occurrences: usize| {
        let deadline = Instant::now() + Duration::from_secs(10);
        while output.matches(needle).count() < occurrences {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            match rx.recv_timeout(remaining) {
                Ok(line) => {
                    output.push_str(&line);
                    output.push('\n');
                }
                Err(_) => return false,
            }
        }
        true
    };

    let mut ok = wait_for("watch.sh:2:1: warning[C001]", 1);
    if ok {
        // Watch mode rebuilds its watcher right after printing a report;
        // events queued in the old watcher can be dropped with it, so let the
        // rebuild settle before changing the file.
        thread::sleep(Duration::from_millis(500));
        fs::remove_file(&script).unwrap();
        ok = wait_for("watch.sh: cannot access path", 1);
    }
    if ok {
        thread::sleep(Duration::from_millis(500));
        fs::write(&script, "#!/bin/bash\nunused=1\necho ok\n").unwrap();
        ok = wait_for("watch.sh:2:1: warning[C001]", 2);
    }
    let still_running = child.try_wait().unwrap().is_none();
    let _ = child.kill();
    let _ = child.wait_timeout(Duration::from_secs(5));

    assert!(ok && still_running, "watch output:\n{output}");
}

#[test]
fn corrupt_cache_files_are_cache_misses_for_check_and_clean() {
    let tempdir = tempdir().unwrap();
    fs::write(tempdir.path().join("broken.sh"), "echo \"$(\n").unwrap();
    shuck(tempdir.path(), tempdir.path())
        .args(["check", "."])
        .assert()
        .code(1);

    let files = cache_files(&cache_dir(tempdir.path()));
    assert!(!files.is_empty());
    // A varint length prefix claiming roughly 4 EiB.
    let mut crafted = vec![0xFD];
    crafted.extend_from_slice(&(1u64 << 62).to_le_bytes());
    for file in &files {
        fs::write(file, &crafted).unwrap();
    }

    shuck(tempdir.path(), tempdir.path())
        .args(["check", "."])
        .assert()
        .code(1)
        .stdout(predicate::str::contains("broken.sh:1:"));
    shuck(tempdir.path(), tempdir.path())
        .arg("clean")
        .assert()
        .success();
}

#[test]
fn format_skips_explicit_files_in_other_languages() {
    let tempdir = tempdir().unwrap();
    let python = "import os\nif True:\n    print( os.sep )\n";
    fs::write(tempdir.path().join("tool.py"), python).unwrap();
    fs::write(
        tempdir.path().join("tool"),
        format!("#!/usr/bin/env python3\n{python}"),
    )
    .unwrap();

    shuck(tempdir.path(), tempdir.path())
        .args(["format", "--no-cache", "tool.py", "tool"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "warning: skipping tool.py: not a shell script",
        ))
        .stderr(predicate::str::contains(
            "warning: skipping tool: not a shell script",
        ));
    assert_eq!(
        fs::read_to_string(tempdir.path().join("tool.py")).unwrap(),
        python
    );

    shuck(tempdir.path(), tempdir.path())
        .args(["check", "--no-cache", "tool.py"])
        .assert()
        .success()
        .stdout("");
}

#[cfg(unix)]
#[test]
fn format_write_back_keeps_permissions_and_symlinks() {
    use std::os::unix::fs::PermissionsExt;

    let tempdir = tempdir().unwrap();
    let target = tempdir.path().join("real.sh");
    let link = tempdir.path().join("link.sh");
    let unformatted = "#!/bin/bash\nif true;then\necho ok\nfi\n";
    fs::write(&target, unformatted).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o750)).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    shuck(tempdir.path(), tempdir.path())
        .args(["format", "--no-cache", "link.sh"])
        .assert()
        .success();

    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_ne!(fs::read_to_string(&target).unwrap(), unformatted);
    let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o750);
    let stray = fs::read_dir(tempdir.path())
        .unwrap()
        .filter_map(Result::ok)
        .any(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"));
    assert!(!stray, "temporary write-back file left behind");
}

#[cfg(unix)]
#[test]
fn check_fix_write_back_keeps_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let tempdir = tempdir().unwrap();
    let script = tempdir.path().join("fix.sh");
    fs::write(&script, "#!/bin/bash\nprintf '%s\\n' x &;\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    shuck(tempdir.path(), tempdir.path())
        .args(["check", "--fix", "--select", "S074", "fix.sh"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(&script).unwrap(),
        "#!/bin/bash\nprintf '%s\\n' x &\n"
    );
    let mode = fs::metadata(&script).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o755);
}

const WORKFLOW: &str = "on: push
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: |
          unused=1
          echo ok
";

#[test]
fn stdin_filename_workflow_is_extracted_like_a_file_on_disk() {
    let tempdir = tempdir().unwrap();

    shuck(tempdir.path(), tempdir.path())
        .args(["check", "--stdin-filename", ".github/workflows/ci.yml"])
        .write_stdin(WORKFLOW)
        .assert()
        .code(1)
        .stdout(predicate::str::contains("jobs.test.steps[0].run"))
        .stdout(predicate::str::contains("C001"))
        .stdout(predicate::str::contains("parse-error").not());

    // `--fix` echoes the host document unchanged.
    shuck(tempdir.path(), tempdir.path())
        .args([
            "check",
            "--fix",
            "--stdin-filename",
            ".github/workflows/ci.yml",
        ])
        .write_stdin(WORKFLOW)
        .assert()
        .code(1)
        .stdout(WORKFLOW);
}

#[test]
fn stdin_filename_for_plain_yaml_or_other_languages_is_skipped() {
    let tempdir = tempdir().unwrap();

    for (filename, contents) in [
        ("config.yaml", "key: [value\n"),
        ("tool.py", "print( 'hi' )\n"),
    ] {
        shuck(tempdir.path(), tempdir.path())
            .args(["check", "--stdin-filename", filename])
            .write_stdin(contents)
            .assert()
            .success()
            .stdout("")
            .stderr(predicate::str::contains(format!(
                "warning: skipping {filename}"
            )));
    }
}

#[test]
fn clean_only_touches_shuck_cache_entries() {
    let tempdir = tempdir().unwrap();
    let project = tempdir.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("ok.sh"), "#!/bin/sh\necho ok\n").unwrap();

    let cache_root = cache_dir(tempdir.path());
    let unrelated_empty = cache_root.join("photos").join("empty");
    let unrelated_version_file = cache_root.join("0.0.1").join("notes.txt");
    fs::create_dir_all(&unrelated_empty).unwrap();
    fs::create_dir_all(unrelated_version_file.parent().unwrap()).unwrap();
    fs::write(&unrelated_version_file, "keep me\n").unwrap();

    shuck(tempdir.path(), &project)
        .args(["check", "."])
        .assert()
        .success();
    assert!(!cache_files(&cache_root).is_empty());

    shuck(tempdir.path(), &project)
        .arg("clean")
        .assert()
        .success();

    assert!(cache_files(&cache_root).is_empty());
    assert!(unrelated_empty.is_dir());
    assert_eq!(
        fs::read_to_string(&unrelated_version_file).unwrap(),
        "keep me\n"
    );
}

#[cfg(unix)]
#[test]
fn clean_skips_unreadable_cache_entries() {
    use std::os::unix::fs::PermissionsExt;

    let tempdir = tempdir().unwrap();
    let cache_root = cache_dir(tempdir.path());
    let locked = cache_root.join("0.0.1");
    fs::create_dir_all(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    if fs::read_dir(&locked).is_ok() {
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        return;
    }

    let assert = shuck(tempdir.path(), tempdir.path()).arg("clean").assert();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    assert
        .success()
        .stderr(predicate::str::contains("warning: skipping"));
}

/// Copies the shuck binary byte-for-byte, so the copy is a distinct build
/// artifact as far as cache identity is concerned.
fn copy_shuck_binary(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(
        &path,
        fs::read(assert_cmd::cargo::cargo_bin("shuck")).unwrap(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

#[test]
fn cache_entries_are_not_shared_between_different_builds() {
    let tempdir = tempdir().unwrap();
    let project = tempdir.path().join("project");
    let bin_dir = tempdir.path().join("bin");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&bin_dir).unwrap();
    fs::write(project.join("ok.sh"), "#!/bin/sh\necho ok\n").unwrap();
    let cache_root = cache_dir(tempdir.path());

    let first = copy_shuck_binary(&bin_dir, "shuck-first");
    thread::sleep(Duration::from_millis(20));
    let second = copy_shuck_binary(&bin_dir, "shuck-second");

    let run = |binary: &Path| {
        let status = ProcessCommand::new(binary)
            .env("SHUCK_CACHE_DIR", &cache_root)
            .current_dir(&project)
            .args(["check", "."])
            .status()
            .unwrap();
        assert!(status.success());
    };

    run(&first);
    run(&first);
    assert_eq!(
        cache_files(&cache_root).len(),
        1,
        "same build reuses its entry"
    );
    run(&second);
    assert_eq!(
        cache_files(&cache_root).len(),
        2,
        "a different build must not reuse the first build's entry"
    );
}

#[test]
fn lint_true_directives_do_not_lint_files_outside_the_project() {
    let tempdir = tempdir().unwrap();
    let project = tempdir.path().join("project");
    let outside = tempdir.path().join("outside");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(project.join("shuck.toml"), "").unwrap();
    fs::write(outside.join("secret.sh"), "secret_unused=1\n").unwrap();
    fs::write(
        project.join("main.sh"),
        "#!/bin/bash\n# shuck: source=../outside/secret.sh lint=true\nsource ../outside/secret.sh\n",
    )
    .unwrap();

    for extra in [&[][..], &["--isolated"][..]] {
        shuck(tempdir.path(), &project)
            .args(["check", "--no-cache", "--output-format", "full"])
            .args(extra)
            .arg("main.sh")
            .assert()
            .stdout(predicate::str::contains("secret_unused").not())
            .stdout(predicate::str::contains("secret.sh:1:").not());
    }
}

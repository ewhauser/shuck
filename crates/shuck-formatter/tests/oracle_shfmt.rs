use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use shuck_formatter::{FormattedSource, ShellDialect, ShellFormatOptions, format_source};

const DEFAULT_SHFMT_ROOT: &str = "/Users/ewhauser/working/shfmt";

struct OracleCase {
    name: &'static str,
    fixture: &'static str,
    filename: &'static str,
    shfmt_flags: &'static [&'static str],
    options: ShellFormatOptions,
}

#[test]
#[ignore = "requires SHUCK_RUN_SHFMT_ORACLE=1 and a local shfmt checkout"]
fn selected_fixtures_match_local_shfmt() {
    if std::env::var_os("SHUCK_RUN_SHFMT_ORACLE").is_none() {
        eprintln!("set SHUCK_RUN_SHFMT_ORACLE=1 to run the local shfmt oracle");
        return;
    }

    let shfmt_root = oracle_root();
    assert!(
        shfmt_root.join("cmd/shfmt/main.go").is_file(),
        "local shfmt checkout not found at {}",
        shfmt_root.display()
    );

    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle-fixtures");
    for case in oracle_cases() {
        let source = fs::read_to_string(fixture_root.join(case.fixture)).unwrap();
        let shuck = run_shuck_formatter(&source, case.filename, &case.options);
        let shfmt = run_shfmt(&shfmt_root, &source, case.filename, case.shfmt_flags);

        assert_eq!(shuck, shfmt, "oracle mismatch for {}", case.name);
    }
}

fn oracle_root() -> PathBuf {
    std::env::var_os("SHUCK_SHFMT_ORACLE_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SHFMT_ROOT))
}

fn run_shuck_formatter(source: &str, filename: &str, options: &ShellFormatOptions) -> String {
    match format_source(source, Some(Path::new(filename)), options).unwrap() {
        FormattedSource::Unchanged => source.to_string(),
        FormattedSource::Formatted(formatted) => formatted,
    }
}

fn run_shfmt(root: &Path, source: &str, filename: &str, flags: &[&str]) -> String {
    let mut command = Command::new("go");
    command.arg("run").arg("./cmd/shfmt");
    command.arg("-filename").arg(filename);
    for flag in flags {
        command.arg(flag);
    }
    command.current_dir(root);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::inherit());

    let mut child = command.spawn().expect("spawn shfmt");
    child
        .stdin
        .as_mut()
        .expect("shfmt stdin")
        .write_all(source.as_bytes())
        .expect("write source to shfmt");
    let output = child.wait_with_output().expect("wait for shfmt");
    assert!(
        output.status.success(),
        "shfmt exited with {}",
        output.status
    );
    String::from_utf8(output.stdout).expect("utf8 shfmt output")
}

fn oracle_cases() -> Vec<OracleCase> {
    vec![
        OracleCase {
            name: "function next line",
            fixture: "function_next_line.sh",
            filename: "function_next_line.sh",
            shfmt_flags: &["-fn"],
            options: ShellFormatOptions::default().with_function_next_line(true),
        },
        OracleCase {
            name: "case arms",
            fixture: "case_default.sh",
            filename: "case_default.sh",
            shfmt_flags: &[],
            options: ShellFormatOptions::default(),
        },
        OracleCase {
            name: "space redirects",
            fixture: "space_redirects.sh",
            filename: "space_redirects.sh",
            shfmt_flags: &["-sr"],
            options: ShellFormatOptions::default().with_space_redirects(true),
        },
        OracleCase {
            name: "keep padding",
            fixture: "keep_padding.sh",
            filename: "keep_padding.sh",
            shfmt_flags: &["-kp"],
            options: ShellFormatOptions::default().with_keep_padding(true),
        },
        OracleCase {
            name: "function never split",
            fixture: "never_split.sh",
            filename: "never_split.sh",
            shfmt_flags: &["-ns"],
            options: ShellFormatOptions::default().with_never_split(true),
        },
        OracleCase {
            name: "nested heredoc",
            fixture: "nested_heredoc.sh",
            filename: "nested_heredoc.sh",
            shfmt_flags: &[],
            options: ShellFormatOptions::default(),
        },
        OracleCase {
            name: "binary next line",
            fixture: "binary_next_line.sh",
            filename: "binary_next_line.sh",
            shfmt_flags: &["-bn"],
            options: ShellFormatOptions::default().with_binary_next_line(true),
        },
        OracleCase {
            name: "simplify",
            fixture: "simplify.sh",
            filename: "simplify.sh",
            shfmt_flags: &["-s"],
            options: ShellFormatOptions::default().with_simplify(true),
        },
        OracleCase {
            name: "minify",
            fixture: "minify.sh",
            filename: "minify.sh",
            shfmt_flags: &["-mn"],
            options: ShellFormatOptions::default().with_minify(true),
        },
        OracleCase {
            name: "mksh select",
            fixture: "mksh_select.sh",
            filename: "script.mksh",
            shfmt_flags: &["-ln=mksh"],
            options: ShellFormatOptions::default().with_dialect(ShellDialect::Mksh),
        },
    ]
}

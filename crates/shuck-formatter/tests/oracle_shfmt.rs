use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use shuck_benchmark::TEST_FILES;
use shuck_formatter::{FormattedSource, ShellDialect, ShellFormatOptions, format_source};
use similar::TextDiff;

const BENCHMARK_ORACLE_FILE_COUNT: usize = 5;
const MAX_ORACLE_DIFF_LINES: usize = 200;

struct OracleCase {
    name: &'static str,
    fixture: &'static str,
    filename: &'static str,
    shfmt_flags: &'static [&'static str],
    options: ShellFormatOptions,
    skip_reason: Option<&'static str>,
}

struct ShfmtProbe {
    supported_flags: String,
}

#[test]
#[ignore = "requires SHUCK_RUN_SHFMT_ORACLE=1 and shfmt on PATH (for example via `nix develop`)"]
fn benchmark_corpus_matches_shfmt() {
    if std::env::var_os("SHUCK_RUN_SHFMT_ORACLE").is_none() {
        eprintln!("set SHUCK_RUN_SHFMT_ORACLE=1 to run the shfmt oracle");
        return;
    }

    let _ = probe_shfmt().expect("shfmt not found on PATH; run under `nix develop`");
    assert_eq!(
        TEST_FILES.len(),
        BENCHMARK_ORACLE_FILE_COUNT,
        "benchmark-backed oracle expects the benchmark corpus to stay at five scripts"
    );

    let options = ShellFormatOptions::default().with_dialect(ShellDialect::Bash);
    let mut mismatches = Vec::new();
    for file in TEST_FILES {
        let filename = format!("{}.bash", file.name);
        let shuck = run_shuck_formatter(file.source, &filename, &options);
        let shfmt = run_shfmt(file.source, &filename, &["-ln=bash"]);
        if let Some(mismatch) = render_oracle_mismatch(file.name, &filename, &shfmt, &shuck) {
            mismatches.push(mismatch);
        }
    }

    assert!(
        mismatches.is_empty(),
        "benchmark corpus diverged from shfmt:\n\n{}",
        mismatches.join("\n\n")
    );
}

#[test]
#[ignore = "requires SHUCK_RUN_SHFMT_ORACLE=1 and shfmt on PATH (for example via `nix develop`)"]
fn selected_fixtures_match_shfmt() {
    if std::env::var_os("SHUCK_RUN_SHFMT_ORACLE").is_none() {
        eprintln!("set SHUCK_RUN_SHFMT_ORACLE=1 to run the shfmt oracle");
        return;
    }

    let shfmt = probe_shfmt().expect("shfmt not found on PATH; run under `nix develop`");

    let fixture_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/oracle-fixtures");
    let mut ran_case = false;
    let mut mismatches = Vec::new();
    for case in oracle_cases() {
        if let Some(reason) = case.skip_reason {
            eprintln!("skipping oracle case `{}`: {}", case.name, reason);
            continue;
        }

        if !case.is_supported(&shfmt) {
            eprintln!(
                "skipping oracle case `{}` because installed shfmt does not support {:?}",
                case.name, case.shfmt_flags
            );
            continue;
        }

        let source = fs::read_to_string(fixture_root.join(case.fixture)).unwrap();
        let shuck = run_shuck_formatter(&source, case.filename, &case.options);
        let shfmt = run_shfmt(&source, case.filename, case.shfmt_flags);

        if let Some(mismatch) = render_oracle_mismatch(case.name, case.filename, &shfmt, &shuck) {
            mismatches.push(mismatch);
        }
        ran_case = true;
    }

    assert!(ran_case, "no oracle cases were compatible with this shfmt binary");
    assert!(
        mismatches.is_empty(),
        "fixture oracle diverged from shfmt:\n\n{}",
        mismatches.join("\n\n")
    );
}

impl OracleCase {
    fn is_supported(&self, shfmt: &ShfmtProbe) -> bool {
        self.shfmt_flags.iter().all(|flag| shfmt.supports_flag(flag))
    }
}

impl ShfmtProbe {
    fn supports_flag(&self, flag: &str) -> bool {
        match flag {
            "-ln=mksh" => self.supported_flags.contains("-ln, --language-dialect"),
            other => self.supported_flags.contains(other),
        }
    }
}

fn probe_shfmt() -> Option<ShfmtProbe> {
    let version = Command::new("shfmt")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    if !version.success() {
        return None;
    }

    let help = Command::new("shfmt").arg("--help").output().ok()?;
    if !help.status.success() {
        return None;
    }

    let mut supported_flags = String::from_utf8_lossy(&help.stdout).into_owned();
    supported_flags.push_str(&String::from_utf8_lossy(&help.stderr));

    Some(ShfmtProbe {
        supported_flags,
    })
}

fn run_shuck_formatter(source: &str, filename: &str, options: &ShellFormatOptions) -> String {
    match format_source(source, Some(Path::new(filename)), options).unwrap() {
        FormattedSource::Unchanged => source.to_string(),
        FormattedSource::Formatted(formatted) => formatted,
    }
}

fn run_shfmt(source: &str, filename: &str, flags: &[&str]) -> String {
    let mut command = Command::new("shfmt");
    command.arg("-filename").arg(filename);
    for flag in flags {
        command.arg(flag);
    }
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

fn render_oracle_mismatch(
    case_name: &str,
    filename: &str,
    shfmt: &str,
    shuck: &str,
) -> Option<String> {
    if shfmt == shuck {
        return None;
    }

    let diff = TextDiff::from_lines(shfmt, shuck)
        .unified_diff()
        .header(
            &format!("shfmt/{filename}"),
            &format!("shuck/{filename}"),
        )
        .to_string();

    Some(format!(
        "oracle mismatch for {case_name}\n{}",
        truncate_diff(&diff)
    ))
}

fn truncate_diff(diff: &str) -> String {
    let lines = diff.lines().collect::<Vec<_>>();
    if lines.len() <= MAX_ORACLE_DIFF_LINES {
        return diff.to_string();
    }

    let omitted = lines.len() - MAX_ORACLE_DIFF_LINES;
    let mut truncated = lines[..MAX_ORACLE_DIFF_LINES].join("\n");
    truncated.push_str(&format!(
        "\n... diff truncated, omitted {omitted} additional lines ..."
    ));
    truncated
}

fn oracle_cases() -> Vec<OracleCase> {
    vec![
        OracleCase {
            name: "function next line",
            fixture: "function_next_line.sh",
            filename: "function_next_line.sh",
            shfmt_flags: &["-fn"],
            options: ShellFormatOptions::default().with_function_next_line(true),
            skip_reason: None,
        },
        OracleCase {
            name: "case arms",
            fixture: "case_default.sh",
            filename: "case_default.sh",
            shfmt_flags: &[],
            options: ShellFormatOptions::default(),
            skip_reason: None,
        },
        OracleCase {
            name: "space redirects",
            fixture: "space_redirects.sh",
            filename: "space_redirects.sh",
            shfmt_flags: &["-sr"],
            options: ShellFormatOptions::default().with_space_redirects(true),
            skip_reason: None,
        },
        OracleCase {
            name: "keep padding",
            fixture: "keep_padding.sh",
            filename: "keep_padding.sh",
            shfmt_flags: &["-kp"],
            options: ShellFormatOptions::default().with_keep_padding(true),
            skip_reason: None,
        },
        OracleCase {
            name: "function never split",
            fixture: "never_split.sh",
            filename: "never_split.sh",
            shfmt_flags: &["-ns"],
            options: ShellFormatOptions::default().with_never_split(true),
            skip_reason: None,
        },
        OracleCase {
            name: "nested heredoc",
            fixture: "nested_heredoc.sh",
            filename: "nested_heredoc.sh",
            shfmt_flags: &[],
            options: ShellFormatOptions::default(),
            skip_reason: None,
        },
        OracleCase {
            name: "if body comment",
            fixture: "if_body_comment.sh",
            filename: "if_body_comment.sh",
            shfmt_flags: &[],
            options: ShellFormatOptions::default(),
            skip_reason: None,
        },
        OracleCase {
            name: "heredoc trailing comment",
            fixture: "heredoc_trailing_comment.sh",
            filename: "heredoc_trailing_comment.sh",
            shfmt_flags: &[],
            options: ShellFormatOptions::default(),
            skip_reason: None,
        },
        OracleCase {
            name: "declare heredoc",
            fixture: "decl_heredoc.sh",
            filename: "decl_heredoc.sh",
            shfmt_flags: &[],
            options: ShellFormatOptions::default(),
            skip_reason: None,
        },
        OracleCase {
            name: "binary next line",
            fixture: "binary_next_line.sh",
            filename: "binary_next_line.sh",
            shfmt_flags: &["-bn"],
            options: ShellFormatOptions::default().with_binary_next_line(true),
            skip_reason: None,
        },
        OracleCase {
            name: "simplify",
            fixture: "simplify.sh",
            filename: "simplify.bash",
            shfmt_flags: &["-s"],
            options: ShellFormatOptions::default().with_simplify(true),
            skip_reason: None,
        },
        OracleCase {
            name: "minify",
            fixture: "minify.sh",
            filename: "minify.sh",
            shfmt_flags: &["-mn"],
            options: ShellFormatOptions::default().with_minify(true),
            skip_reason: Some("Shuck minify currently drops the shebang while upstream shfmt preserves it"),
        },
        OracleCase {
            name: "mksh select",
            fixture: "mksh_select.sh",
            filename: "script.mksh",
            shfmt_flags: &["-ln=mksh"],
            options: ShellFormatOptions::default().with_dialect(ShellDialect::Mksh),
            skip_reason: None,
        },
    ]
}

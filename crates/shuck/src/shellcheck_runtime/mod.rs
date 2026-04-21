use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const HELP_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellCheckReplacement {
    pub line: usize,
    #[serde(rename = "endLine")]
    pub end_line: usize,
    pub column: usize,
    #[serde(rename = "endColumn")]
    pub end_column: usize,
    pub precedence: usize,
    #[serde(rename = "insertionPoint")]
    pub insertion_point: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellCheckFix {
    #[serde(default)]
    pub replacements: Vec<ShellCheckReplacement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShellCheckDiagnostic {
    #[serde(default)]
    pub file: String,
    pub code: u32,
    pub line: usize,
    #[serde(rename = "endLine")]
    pub end_line: usize,
    pub column: usize,
    #[serde(rename = "endColumn")]
    pub end_column: usize,
    pub level: String,
    pub message: String,
    #[serde(default)]
    pub fix: Option<ShellCheckFix>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCheckRun {
    pub diagnostics: Vec<ShellCheckDiagnostic>,
    pub parse_aborted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCheckProbe {
    pub command: String,
    pub version_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCheckProcessOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub status_code: i32,
}

pub fn probe_shellcheck() -> Option<ShellCheckProbe> {
    let output = Command::new("shellcheck").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }

    let version_text = normalize_shellcheck_version_text(&output.stdout);
    if version_text.is_empty() {
        return None;
    }

    Some(ShellCheckProbe {
        command: "shellcheck".into(),
        version_text,
    })
}

pub fn run_shellcheck_command<I, T>(
    shellcheck_path: &str,
    args: I,
    timeout: Duration,
) -> Result<ShellCheckProcessOutput, String>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let mut command = Command::new(shellcheck_path);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    for arg in args {
        command.arg(arg.into());
    }

    let mut child = command
        .spawn()
        .map_err(|err| format!("shellcheck exec: {err}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "shellcheck exec: failed to capture stdout".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "shellcheck exec: failed to capture stderr".to_owned())?;
    let stdout_reader = thread::spawn(move || read_shellcheck_pipe(stdout, "stdout"));
    let stderr_reader = thread::spawn(move || read_shellcheck_pipe(stderr, "stderr"));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait().map_err(|err| format!("shellcheck wait: {err}"))? {
            Some(status) => break status,
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stdout_reader.join();
                let _ = stderr_reader.join();
                return Err(format_timeout_message("shellcheck", timeout));
            }
            None => thread::sleep(Duration::from_millis(10)),
        }
    };

    let stdout = join_shellcheck_pipe(stdout_reader, "stdout")?;
    let stderr = join_shellcheck_pipe(stderr_reader, "stderr")?;

    Ok(ShellCheckProcessOutput {
        stdout,
        stderr,
        status_code: status.code().unwrap_or(-1),
    })
}

pub fn run_shellcheck_json1(
    path: &Path,
    shell: &str,
    shellcheck_path: &str,
    timeout: Duration,
) -> Result<ShellCheckRun, String> {
    let output = run_shellcheck_command(
        shellcheck_path,
        [
            OsString::from("--norc"),
            OsString::from("-s"),
            OsString::from(shell),
            OsString::from("-f"),
            OsString::from("json1"),
            path.as_os_str().to_os_string(),
        ],
        timeout,
    )?;

    // ShellCheck exits 1 when it reports diagnostics, which is not an execution error.
    if !matches!(output.status_code, 0 | 1) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("shellcheck exit {}: {stderr}", output.status_code));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        return Ok(ShellCheckRun {
            diagnostics: Vec::new(),
            parse_aborted: false,
        });
    }

    let diagnostics = decode_shellcheck_diagnostics(stdout.as_bytes())?;
    let parse_aborted = shellcheck_parse_aborted(&diagnostics);
    Ok(ShellCheckRun {
        diagnostics,
        parse_aborted,
    })
}

pub fn shellcheck_supported_shells(shellcheck_path: &str) -> HashMap<&'static str, ()> {
    let output = run_shellcheck_command(shellcheck_path, ["--help"], HELP_PROBE_TIMEOUT);
    let mut supported = output
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .map(|help| parse_shellcheck_supported_shells(&help))
        .unwrap_or_default();

    if supported.is_empty() {
        for shell in &["sh", "bash", "dash", "ksh", "busybox"] {
            supported.insert(shell, ());
        }
    }

    supported
}

pub fn parse_shellcheck_supported_shells(help: &str) -> HashMap<&'static str, ()> {
    let mut supported = HashMap::new();
    for line in help.lines() {
        if !line.contains("--shell=") {
            continue;
        }
        if let Some(start) = line.find('(')
            && let Some(end) = line[start + 1..].find(')')
        {
            let shells = &line[start + 1..start + 1 + end];
            for shell in shells.split(',') {
                match shell.trim() {
                    "sh" => {
                        supported.insert("sh", ());
                    }
                    "bash" => {
                        supported.insert("bash", ());
                    }
                    "dash" => {
                        supported.insert("dash", ());
                    }
                    "ksh" => {
                        supported.insert("ksh", ());
                    }
                    "busybox" => {
                        supported.insert("busybox", ());
                    }
                    "zsh" => {
                        supported.insert("zsh", ());
                    }
                    _ => {}
                }
            }
        }
    }
    supported
}

pub fn decode_shellcheck_diagnostics(data: &[u8]) -> Result<Vec<ShellCheckDiagnostic>, String> {
    let trimmed = String::from_utf8_lossy(data);
    let trimmed = trimmed.trim();

    if trimmed.is_empty() {
        return Err("empty shellcheck json output".into());
    }

    if trimmed.starts_with('[') {
        serde_json::from_str::<Vec<ShellCheckDiagnostic>>(trimmed)
            .map_err(|err| format!("decode shellcheck json array: {err}"))
    } else if trimmed.starts_with('{') {
        #[derive(Deserialize)]
        struct Wrapper {
            comments: Vec<ShellCheckDiagnostic>,
        }

        serde_json::from_str::<Wrapper>(trimmed)
            .map(|wrapper| wrapper.comments)
            .map_err(|err| format!("decode shellcheck json object: {err}"))
    } else {
        Err(format!(
            "decode shellcheck json: unexpected leading byte {:?}",
            trimmed.chars().next()
        ))
    }
}

pub fn shellcheck_parse_aborted(diags: &[ShellCheckDiagnostic]) -> bool {
    diags.iter().any(|diag| {
        if diag.level != "error" {
            return false;
        }

        matches!(diag.code, 1072 | 1073 | 1088) || {
            let lower = diag.message.to_ascii_lowercase();
            lower.contains("fix to allow more checks")
                || lower.contains("fix any mentioned problems and try again")
                || lower.contains("parsing stopped here")
        }
    })
}

pub fn normalize_shellcheck_version_text(output: &[u8]) -> String {
    String::from_utf8_lossy(output)
        .replace("\r\n", "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

fn format_timeout_message(label: &str, timeout: Duration) -> String {
    if timeout.as_millis() < 1000 {
        format!("{label} timed out after {}ms", timeout.as_millis())
    } else {
        format!("{label} timed out after {}s", timeout.as_secs())
    }
}

fn read_shellcheck_pipe<R: Read>(mut pipe: R, label: &str) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    pipe.read_to_end(&mut data)
        .map_err(|err| format!("shellcheck {label}: {err}"))?;
    Ok(data)
}

fn join_shellcheck_pipe(
    reader: thread::JoinHandle<Result<Vec<u8>, String>>,
    label: &str,
) -> Result<Vec<u8>, String> {
    reader
        .join()
        .map_err(|_| format!("shellcheck {label} reader panicked"))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_shellcheck_version_text() {
        let output = b"ShellCheck - shell script analysis tool\r\nversion: 0.11.0\r\n";
        assert_eq!(
            normalize_shellcheck_version_text(output),
            "ShellCheck - shell script analysis tool\nversion: 0.11.0"
        );
    }

    #[test]
    fn parse_shellcheck_supported_shells_parses_help() {
        let help = "Usage: shellcheck [OPTIONS...] FILES...\n\
            -s SHELLNAME        --shell=SHELLNAME          Specify dialect (sh, bash, dash, ksh, busybox)\n";

        let supported = parse_shellcheck_supported_shells(help);
        for shell in &["sh", "bash", "dash", "ksh", "busybox"] {
            assert!(supported.contains_key(shell), "expected {shell} in help");
        }
        assert!(!supported.contains_key("zsh"));
    }

    #[test]
    fn decode_shellcheck_json_array() {
        let data = br#"[{"file":"x.sh","line":1,"endLine":1,"column":1,"endColumn":2,"level":"warning","code":2034,"message":"unused","fix":null}]"#;
        let diagnostics = decode_shellcheck_diagnostics(data).unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, 2034);
    }

    #[test]
    fn decode_shellcheck_json_object() {
        let data = br#"{"comments":[{"file":"x.sh","line":1,"endLine":1,"column":1,"endColumn":2,"level":"warning","code":2034,"message":"unused","fix":null}]}"#;
        let diagnostics = decode_shellcheck_diagnostics(data).unwrap();
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, 2034);
    }

    #[test]
    fn shellcheck_parse_abort_detection_matches_known_codes() {
        assert!(shellcheck_parse_aborted(&[ShellCheckDiagnostic {
            file: "x.sh".into(),
            code: 1072,
            line: 1,
            end_line: 1,
            column: 1,
            end_column: 1,
            level: "error".into(),
            message: "Expected then".into(),
            fix: None,
        }]));
    }

    #[cfg(unix)]
    #[test]
    fn run_shellcheck_command_times_out() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

        let tempdir = tempfile::tempdir().unwrap();
        let shellcheck_path = tempdir.path().join("fake-shellcheck");

        fs::write(&shellcheck_path, "#!/bin/sh\nsleep 1\nprintf '[]'\n").unwrap();

        let mut permissions = fs::metadata(&shellcheck_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&shellcheck_path, permissions).unwrap();

        let err = run_shellcheck_command(
            shellcheck_path.to_str().unwrap(),
            ["--version"],
            Duration::from_millis(10),
        )
        .unwrap_err();

        assert_eq!(err, "shellcheck timed out after 10ms");
    }
}

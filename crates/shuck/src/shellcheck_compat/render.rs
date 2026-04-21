use std::cmp::min;
use std::io::{self, ErrorKind, Write};

use colored::Colorize;
use serde::Serialize;

use super::{
    CompatCliError, CompatDiagnostic, CompatFormat, CompatLevel, CompatOptions, CompatReport,
    use_color,
};
use crate::shellcheck_compat::optional::OptionalCheck;
use crate::shellcheck_runtime::{ShellCheckDiagnostic, ShellCheckFix};

pub fn usage_text() -> String {
    [
        "Usage: shellcheck [OPTIONS...] FILES...",
        "  -a, --check-sourced            Also report diagnostics from resolved sourced files",
        "  -C, --color[=WHEN]             Control color output with auto, always, or never",
        "  -i, --include=CODES            Keep only the listed SC codes",
        "  -e, --exclude=CODES            Drop the listed SC codes",
        "      --extended-analysis=BOOL   Toggle dataflow-heavy checks on or off",
        "  -f, --format=FORMAT            Choose checkstyle, diff, gcc, json, json1, quiet, or tty",
        "      --list-optional            Print the optional-check catalog",
        "      --norc                     Skip .shellcheckrc discovery",
        "      --rcfile=PATH              Load a specific shellcheck rc file",
        "  -o, --enable=CHECKS            Enable named optional checks",
        "  -P, --source-path=PATHS        Add search roots for sourced files",
        "  -s, --shell=SHELL              Force sh, bash, dash, ksh, or busybox parsing",
        "  -S, --severity=LEVEL           Filter to style, info, warning, or error and above",
        "  -V, --version                  Show version information",
        "  -W, --wiki-link-count=NUM      Limit wiki links in tty output",
        "  -x, --external-sources         Treat resolved sourced files as explicit inputs",
        "      --help                     Show this summary and exit",
    ]
    .join("\n")
}

pub fn print_error_help(message: &str, show_help: bool) -> io::Result<()> {
    let mut stderr = io::stderr().lock();
    writeln!(stderr, "{message}")?;
    if show_help {
        writeln!(stderr)?;
        writeln!(stderr, "{}", usage_text())?;
    }
    Ok(())
}

pub fn print_version() {
    println!("ShellCheck compatibility mode");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    println!("engine: shuck");
}

pub fn print_list_optional(checks: &[OptionalCheck]) {
    for (index, check) in checks.iter().enumerate() {
        if index > 0 {
            println!();
        }
        println!("name:    {}", check.name);
        println!("desc:    {}", check.description);
        println!("example: {}", check.example);
        println!("fix:     {}", check.guidance);
    }
}

pub fn print_report(report: &CompatReport, options: &CompatOptions) -> Result<(), CompatCliError> {
    let rendered = match options.format {
        CompatFormat::Checkstyle => render_checkstyle(report),
        CompatFormat::Diff => render_diff(report),
        CompatFormat::Gcc => render_gcc(report),
        CompatFormat::Json => render_json(report)?,
        CompatFormat::Json1 => render_json1(report)?,
        CompatFormat::Quiet => String::new(),
        CompatFormat::Tty => render_tty(report, options),
    };

    let mut stdout = io::stdout().lock();
    write_rendered(&mut stdout, &rendered)
}

fn write_rendered<W: Write>(writer: &mut W, rendered: &str) -> Result<(), CompatCliError> {
    match writer.write_all(rendered.as_bytes()) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Ok(()),
        Err(err) => Err(CompatCliError::runtime(
            2,
            format!("could not write report: {err}"),
        )),
    }
}

fn render_checkstyle(report: &CompatReport) -> String {
    let mut files = std::collections::BTreeMap::<&str, Vec<&CompatDiagnostic>>::new();
    for diagnostic in &report.diagnostics {
        files.entry(&diagnostic.file).or_default().push(diagnostic);
    }

    let mut output = String::from("<?xml version='1.0' encoding='UTF-8'?>\n");
    output.push_str("<checkstyle version='4.3'>\n");
    for (file, diagnostics) in files {
        output.push_str(&format!("<file name='{}' >\n", xml_escape(file)));
        for diagnostic in diagnostics {
            output.push_str(&format!(
                "<error line='{}' column='{}' severity='{}' message='{}' source='ShellCheck.SC{:04}' />\n",
                diagnostic.line,
                diagnostic.column,
                diagnostic.level.as_str(),
                xml_escape(&diagnostic.message),
                diagnostic.code
            ));
        }
        output.push_str("</file>\n");
    }
    output.push_str("</checkstyle>\n");
    output
}

fn render_diff(report: &CompatReport) -> String {
    let mut files = std::collections::BTreeMap::<&str, Vec<&CompatDiagnostic>>::new();
    for diagnostic in &report.diagnostics {
        files.entry(&diagnostic.file).or_default().push(diagnostic);
    }

    let mut output = String::new();
    for (file, diagnostics) in files {
        if diagnostics.is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("--- {file}\n+++ {file}\n"));
        output.push_str("@@ compatibility mode @@\n");
        for diagnostic in diagnostics {
            output.push_str(&format!(
                "# SC{:04} ({}) {}\n",
                diagnostic.code,
                diagnostic.level.as_str(),
                diagnostic.message
            ));
        }
    }
    output
}

fn render_gcc(report: &CompatReport) -> String {
    let mut output = String::new();
    for diagnostic in &report.diagnostics {
        output.push_str(&format!(
            "{}:{}:{}: {}: {} [SC{:04}]\n",
            diagnostic.file,
            diagnostic.line,
            diagnostic.column,
            diagnostic.level.gcc_label(),
            diagnostic.message,
            diagnostic.code
        ));
    }
    output
}

fn render_json(report: &CompatReport) -> Result<String, CompatCliError> {
    serde_json::to_string(&to_shellcheck_diagnostics(report))
        .map_err(|err| CompatCliError::runtime(2, format!("could not encode json output: {err}")))
}

fn render_json1(report: &CompatReport) -> Result<String, CompatCliError> {
    #[derive(Serialize)]
    struct Wrapper {
        comments: Vec<ShellCheckDiagnostic>,
    }

    serde_json::to_string(&Wrapper {
        comments: to_shellcheck_diagnostics(report),
    })
    .map_err(|err| CompatCliError::runtime(2, format!("could not encode json output: {err}")))
}

fn render_tty(report: &CompatReport, options: &CompatOptions) -> String {
    let use_color = use_color(options.color);
    let mut output = String::new();
    let mut grouped = std::collections::BTreeMap::<&str, Vec<&CompatDiagnostic>>::new();
    for diagnostic in &report.diagnostics {
        grouped
            .entry(&diagnostic.file)
            .or_default()
            .push(diagnostic);
    }

    for (file, diagnostics) in grouped {
        if diagnostics.is_empty() {
            continue;
        }

        let mut by_line = std::collections::BTreeMap::<usize, Vec<&CompatDiagnostic>>::new();
        for diagnostic in diagnostics {
            by_line.entry(diagnostic.line).or_default().push(diagnostic);
        }

        for (index, (line_no, line_diags)) in by_line.into_iter().enumerate() {
            if !output.is_empty() {
                output.push('\n');
            }
            if index == 0 {
                output.push_str(&format!(
                    "{} {}\n",
                    stylize("In", use_color, |text| text.bold()),
                    stylize(format!("{file} line {line_no}:"), use_color, |text| text
                        .bold())
                ));
            } else {
                output.push_str(&format!(
                    "{} {}\n",
                    stylize("In", use_color, |text| text.bold()),
                    stylize(format!("{file} line {line_no}:"), use_color, |text| text
                        .bold())
                ));
            }

            let source_line = line_diags
                .first()
                .and_then(|diagnostic| line_text(diagnostic, line_no))
                .unwrap_or_default();
            output.push_str(&format!("{source_line}\n"));
            for diagnostic in &line_diags {
                let marker = marker_for(diagnostic, source_line.len());
                let text = format!(
                    "{marker} SC{:04} ({}): {}",
                    diagnostic.code,
                    diagnostic.level.as_str(),
                    diagnostic.message
                );
                output.push_str(&format!(
                    "{}\n",
                    match diagnostic.level {
                        CompatLevel::Error => stylize(text, use_color, |value| value.red()),
                        CompatLevel::Warning => stylize(text, use_color, |value| value.yellow()),
                        CompatLevel::Info => stylize(text, use_color, |value| value.green()),
                        CompatLevel::Style => stylize(text, use_color, |value| value.cyan()),
                    }
                ));
            }
        }
    }

    let links = collect_links(report, options.wiki_link_count);
    if !links.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str("For more information:\n");
        for link in links {
            output.push_str(&format!("  {link}\n"));
        }
    }

    output
}

fn to_shellcheck_diagnostics(report: &CompatReport) -> Vec<ShellCheckDiagnostic> {
    report
        .diagnostics
        .iter()
        .map(|diagnostic| ShellCheckDiagnostic {
            file: diagnostic.file.clone(),
            code: diagnostic.code,
            line: diagnostic.line,
            end_line: diagnostic.end_line.max(diagnostic.line),
            column: diagnostic.column,
            end_column: diagnostic.end_column.max(diagnostic.column),
            level: diagnostic.level.as_str().to_owned(),
            message: diagnostic.message.clone(),
            fix: None::<ShellCheckFix>,
        })
        .collect()
}

fn collect_links(report: &CompatReport, count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }

    let mut links = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for diagnostic in &report.diagnostics {
        if seen.insert(diagnostic.code) {
            links.push(format!(
                "https://www.shellcheck.net/wiki/SC{:04} -- {}",
                diagnostic.code,
                truncate(&diagnostic.message, 60)
            ));
        }
        if links.len() >= count {
            break;
        }
    }
    links
}

fn line_text(diagnostic: &CompatDiagnostic, line_no: usize) -> Option<&str> {
    let source = diagnostic.source.as_deref()?;
    source.lines().nth(line_no.saturating_sub(1))
}

fn marker_for(diagnostic: &CompatDiagnostic, line_len: usize) -> String {
    let start = diagnostic.column.saturating_sub(1);
    let end = min(
        line_len.max(start + 1),
        diagnostic
            .end_column
            .max(diagnostic.column)
            .saturating_sub(1),
    );
    let width = end.saturating_sub(start).max(1);
    format!("{}{}", " ".repeat(start), "^".repeat(width))
}

fn truncate(value: &str, max_len: usize) -> String {
    if value.len() <= max_len {
        value.to_owned()
    } else {
        format!("{}...", &value[..max_len.saturating_sub(3)])
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\"', "&quot;")
        .replace('\'', "&apos;")
}

fn stylize(
    value: impl Into<String>,
    use_color: bool,
    style: impl FnOnce(colored::ColoredString) -> colored::ColoredString,
) -> String {
    let value = value.into();
    if use_color {
        style(value.normal()).to_string()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::write_rendered;

    struct BrokenPipeWriter;

    impl std::io::Write for BrokenPipeWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "broken pipe",
            ))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn broken_pipe_is_treated_as_success() {
        let mut writer = BrokenPipeWriter;
        assert!(write_rendered(&mut writer, "hello").is_ok());
    }
}

use std::cmp::min;
use std::collections::BTreeMap;
use std::io::{self, ErrorKind, Write};

use colored::Colorize;
use serde::Serialize;
use similar::TextDiff;

use super::{
    CompatCliError, CompatDiagnostic, CompatFormat, CompatLevel, CompatOptions, CompatReport,
    use_color,
};
use crate::shellcheck_compat::optional::OptionalCheck;
use crate::shellcheck_runtime::ShellCheckDiagnostic;

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

    if options.format == CompatFormat::Diff && rendered == no_auto_fix_diff_message() {
        let mut stderr = io::stderr().lock();
        return write_rendered(&mut stderr, &rendered);
    }

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
    let mut files = BTreeMap::<&str, Vec<&CompatDiagnostic>>::new();
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
    if report.diagnostics.is_empty() {
        return String::new();
    }

    struct FileDiffInput<'a> {
        source: &'a str,
        replacements: Vec<&'a crate::shellcheck_runtime::ShellCheckReplacement>,
    }

    let mut files = BTreeMap::<&str, FileDiffInput<'_>>::new();
    for diagnostic in &report.diagnostics {
        let Some(fix) = diagnostic.fix.as_ref() else {
            continue;
        };
        let Some(source) = diagnostic.source.as_deref() else {
            continue;
        };
        files
            .entry(&diagnostic.file)
            .and_modify(|entry| entry.replacements.extend(fix.replacements.iter()))
            .or_insert_with(|| FileDiffInput {
                source,
                replacements: fix.replacements.iter().collect(),
            });
    }

    if files.is_empty() {
        return no_auto_fix_diff_message();
    }

    let mut output = String::new();
    for (file, file_input) in files {
        let Some(fixed_source) = apply_replacements(file_input.source, &file_input.replacements)
        else {
            continue;
        };
        if fixed_source == file_input.source {
            continue;
        }

        output.push_str(
            &TextDiff::from_lines(file_input.source, &fixed_source)
                .unified_diff()
                .header(&format!("a/{file}"), &format!("b/{file}"))
                .to_string(),
        );
        output.push('\n');
    }

    if output.is_empty() {
        return no_auto_fix_diff_message();
    }

    output
}

fn no_auto_fix_diff_message() -> String {
    "Issues were detected, but none were auto-fixable. Use another format to see them.\n".to_owned()
}

fn apply_replacements(
    source: &str,
    replacements: &[&crate::shellcheck_runtime::ShellCheckReplacement],
) -> Option<String> {
    let mut edits = replacements
        .iter()
        .map(|replacement| {
            replacement_offset(source, replacement)
                .map(|offset| (offset, replacement.replacement.as_str()))
        })
        .collect::<Option<Vec<_>>>()?;
    edits.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.cmp(left.1)));

    let mut output = source.to_owned();
    for (offset, replacement) in edits {
        output.insert_str(offset, replacement);
    }
    Some(output)
}

fn replacement_offset(
    source: &str,
    replacement: &crate::shellcheck_runtime::ShellCheckReplacement,
) -> Option<usize> {
    match replacement.insertion_point.as_str() {
        "beforeStart" => byte_offset_for_line_column(source, replacement.line, replacement.column),
        "afterEnd" => {
            byte_offset_for_line_column(source, replacement.end_line, replacement.end_column)
        }
        _ => None,
    }
}

fn byte_offset_for_line_column(source: &str, line_number: usize, column: usize) -> Option<usize> {
    if line_number == 0 {
        return None;
    }

    let mut line_start = 0usize;
    for (index, segment) in source.split_inclusive('\n').enumerate() {
        if index + 1 == line_number {
            let line = segment.strip_suffix('\n').unwrap_or(segment);
            return byte_offset_in_line(line, column).map(|offset| line_start + offset);
        }
        line_start += segment.len();
    }

    if line_number == 1 && source.is_empty() {
        return byte_offset_in_line("", column);
    }

    None
}

fn byte_offset_in_line(line: &str, column: usize) -> Option<usize> {
    let target_chars = column.checked_sub(1)?;
    let char_count = line.chars().count();
    if target_chars > char_count {
        return None;
    }
    if target_chars == char_count {
        return Some(line.len());
    }

    line.char_indices()
        .nth(target_chars)
        .map(|(offset, _)| offset)
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
    let mut grouped = BTreeMap::<&str, Vec<&CompatDiagnostic>>::new();
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

        let mut by_line = BTreeMap::<usize, Vec<&CompatDiagnostic>>::new();
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
            fix: diagnostic.fix.clone(),
        })
        .collect()
}

fn collect_links(report: &CompatReport, count: usize) -> Vec<String> {
    if count == 0 {
        return Vec::new();
    }

    let mut links = Vec::new();
    let mut seen = BTreeMap::new();
    for diagnostic in &report.diagnostics {
        if seen.insert(diagnostic.code, ()).is_none() {
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
    if value.chars().count() <= max_len {
        value.to_owned()
    } else {
        let prefix = value
            .chars()
            .take(max_len.saturating_sub(3))
            .collect::<String>();
        format!("{prefix}...")
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
    use super::{truncate, write_rendered};

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

    #[test]
    fn truncate_handles_multibyte_text() {
        assert_eq!(truncate("naive cafe", 7), "naiv...");
        assert_eq!(truncate("naive café", 7), "naiv...");
        assert_eq!(truncate("emoji 😅 test", 9), "emoji ...");
    }
}

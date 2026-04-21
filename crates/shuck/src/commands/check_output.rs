use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use annotate_snippets::{AnnotationType, Renderer, Slice, Snippet, SourceAnnotation};
use colored::{ColoredString, Colorize};
use quick_junit::{NonSuccessKind, Report, TestCase, TestCaseStatus, TestSuite, XmlString};
use serde::Serialize;
use shuck_indexer::LineIndex;
use shuck_linter::{Category, RuleMetadata, code_to_rule, rule_metadata};

use crate::args::CheckOutputFormatArg;

const PARSE_ERROR_CODE: &str = "parse-error";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(super) enum DisplayedApplicability {
    Safe,
    Unsafe,
}

impl DisplayedApplicability {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Unsafe => "unsafe",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct DisplayPosition {
    pub(super) line: usize,
    pub(super) column: usize,
}

impl DisplayPosition {
    pub(super) const fn new(line: usize, column: usize) -> Self {
        Self { line, column }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DisplaySpan {
    pub(super) start: DisplayPosition,
    pub(super) end: DisplayPosition,
}

impl DisplaySpan {
    pub(super) const fn new(start: DisplayPosition, end: DisplayPosition) -> Self {
        Self { start, end }
    }

    pub(super) const fn point(line: usize, column: usize) -> Self {
        let position = DisplayPosition::new(line, column);
        Self::new(position, position)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct DisplayedEdit {
    pub(super) location: DisplayPosition,
    pub(super) end_location: DisplayPosition,
    pub(super) content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct DisplayedFix {
    pub(super) applicability: DisplayedApplicability,
    pub(super) message: Option<String>,
    pub(super) edits: Vec<DisplayedEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DisplayedDiagnosticKind {
    ParseError,
    Lint { code: String, severity: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DisplayedDiagnostic {
    pub(super) path: PathBuf,
    pub(super) relative_path: PathBuf,
    pub(super) absolute_path: PathBuf,
    pub(super) span: DisplaySpan,
    pub(super) message: String,
    pub(super) kind: DisplayedDiagnosticKind,
    pub(super) fix: Option<DisplayedFix>,
    pub(super) source: Option<Arc<str>>,
}

impl DisplayedDiagnostic {
    fn code(&self) -> &str {
        match &self.kind {
            DisplayedDiagnosticKind::ParseError => PARSE_ERROR_CODE,
            DisplayedDiagnosticKind::Lint { code, .. } => code,
        }
    }

    fn severity(&self) -> &str {
        match &self.kind {
            DisplayedDiagnosticKind::ParseError => "error",
            DisplayedDiagnosticKind::Lint { severity, .. } => severity,
        }
    }

    fn display_path_string(&self) -> String {
        self.path.display().to_string()
    }

    fn absolute_uri(&self) -> io::Result<String> {
        url::Url::from_file_path(&self.absolute_path)
            .map(|url| url.to_string())
            .map_err(|()| {
                io::Error::other(format!(
                    "failed to convert path to file URI: {}",
                    self.absolute_path.display()
                ))
            })
    }
}

pub(super) fn print_report_to(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
    output_format: CheckOutputFormatArg,
    use_color: bool,
) -> io::Result<()> {
    match output_format {
        CheckOutputFormatArg::Full => write_full_diagnostics(writer, diagnostics, use_color),
        CheckOutputFormatArg::Concise => write_concise_diagnostics(writer, diagnostics, use_color),
        CheckOutputFormatArg::Grouped => write_grouped_diagnostics(writer, diagnostics, use_color),
        CheckOutputFormatArg::Json => write_json_diagnostics(writer, diagnostics),
        CheckOutputFormatArg::JsonLines => write_json_lines_diagnostics(writer, diagnostics),
        CheckOutputFormatArg::Junit => write_junit_diagnostics(writer, diagnostics),
        CheckOutputFormatArg::Github => write_github_diagnostics(writer, diagnostics),
        CheckOutputFormatArg::Gitlab => write_gitlab_diagnostics(writer, diagnostics),
        CheckOutputFormatArg::Rdjson => write_rdjson_diagnostics(writer, diagnostics),
        CheckOutputFormatArg::Sarif => write_sarif_diagnostics(writer, diagnostics),
    }
}

fn write_full_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
    use_color: bool,
) -> io::Result<()> {
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        if index > 0 {
            writeln!(writer)?;
        }

        let rendered = format_full_diagnostic(diagnostic, use_color);
        writer.write_all(rendered.as_bytes())?;
        if !rendered.ends_with('\n') {
            writeln!(writer)?;
        }
    }

    Ok(())
}

fn write_concise_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
    use_color: bool,
) -> io::Result<()> {
    for diagnostic in diagnostics {
        writeln!(
            writer,
            "{}",
            format_concise_diagnostic(diagnostic, use_color)
        )?;
    }

    Ok(())
}

fn write_grouped_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
    use_color: bool,
) -> io::Result<()> {
    let mut grouped = BTreeMap::<PathBuf, Vec<&DisplayedDiagnostic>>::new();
    for diagnostic in diagnostics {
        grouped
            .entry(diagnostic.path.clone())
            .or_default()
            .push(diagnostic);
    }

    for (index, (path, messages)) in grouped.into_iter().enumerate() {
        if index > 0 {
            writeln!(writer)?;
        }

        let header = paint(path.display().to_string(), use_color, |value| {
            value.bold().underline()
        });
        writeln!(writer, "{header}:")?;

        for diagnostic in messages {
            let line = paint(diagnostic.span.start.line.to_string(), use_color, |value| {
                value.cyan()
            });
            let column = paint(
                diagnostic.span.start.column.to_string(),
                use_color,
                |value| value.cyan(),
            );
            writeln!(
                writer,
                "  {line}:{column}: {}",
                format_diagnostic_body(diagnostic, use_color)
            )?;
        }
    }

    Ok(())
}

fn write_json_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
) -> io::Result<()> {
    let values = diagnostics.iter().map(json_diagnostic).collect::<Vec<_>>();
    serde_json::to_writer_pretty(&mut *writer, &values).map_err(io::Error::other)?;
    writeln!(writer)?;
    Ok(())
}

fn write_json_lines_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
) -> io::Result<()> {
    for diagnostic in diagnostics {
        serde_json::to_writer(&mut *writer, &json_diagnostic(diagnostic))
            .map_err(io::Error::other)?;
        writeln!(writer)?;
    }
    Ok(())
}

fn write_junit_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
) -> io::Result<()> {
    let package = "org.shuck";
    let mut report = Report::new("shuck");

    if diagnostics.is_empty() {
        let mut suite = TestSuite::new("shuck");
        suite
            .extra
            .insert(XmlString::new("package"), XmlString::new(package));
        let mut case = TestCase::new("No errors found", TestCaseStatus::success());
        case.set_classname("shuck");
        suite.add_test_case(case);
        report.add_test_suite(suite);
    } else {
        let mut grouped = BTreeMap::<String, Vec<&DisplayedDiagnostic>>::new();
        for diagnostic in diagnostics {
            grouped
                .entry(diagnostic.display_path_string())
                .or_default()
                .push(diagnostic);
        }

        for (filename, diagnostics) in grouped {
            let mut suite = TestSuite::new(&filename);
            suite
                .extra
                .insert(XmlString::new("package"), XmlString::new(package));
            let classname = Path::new(&filename)
                .with_extension("")
                .to_string_lossy()
                .to_string();

            for diagnostic in diagnostics {
                let mut status = TestCaseStatus::non_success(NonSuccessKind::Failure);
                status.set_message(&diagnostic.message);
                status.set_description(format!(
                    "line {}, col {}, {}",
                    diagnostic.span.start.line, diagnostic.span.start.column, diagnostic.message
                ));

                let mut case = TestCase::new(format!("org.shuck.{}", diagnostic.code()), status);
                case.set_classname(&classname);
                case.extra.insert(
                    XmlString::new("line"),
                    XmlString::new(diagnostic.span.start.line.to_string()),
                );
                case.extra.insert(
                    XmlString::new("column"),
                    XmlString::new(diagnostic.span.start.column.to_string()),
                );
                suite.add_test_case(case);
            }

            report.add_test_suite(suite);
        }
    }

    report.serialize(&mut *writer).map_err(io::Error::other)?;
    writeln!(writer)?;
    Ok(())
}

fn write_github_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
) -> io::Result<()> {
    for diagnostic in diagnostics {
        let severity = match diagnostic.severity() {
            "hint" | "info" => "notice",
            "warning" => "warning",
            _ => "error",
        };
        let title = escape_github_property(&format!("shuck ({})", diagnostic.code()));
        let file = escape_github_property(&diagnostic.display_path_string());
        let body = escape_github_message(&format!(
            "{}:{}:{}: {} {}",
            diagnostic.path.display(),
            diagnostic.span.start.line,
            diagnostic.span.start.column,
            diagnostic.code(),
            diagnostic.message
        ));

        write!(writer, "::{severity} title={title},file={file}")?;
        if diagnostic.span.start.line == diagnostic.span.end.line {
            write!(
                writer,
                ",line={},col={},endLine={},endColumn={}",
                diagnostic.span.start.line,
                diagnostic.span.start.column,
                diagnostic.span.end.line,
                diagnostic.span.end.column.max(diagnostic.span.start.column),
            )?;
        } else {
            write!(
                writer,
                ",line={},endLine={}",
                diagnostic.span.start.line, diagnostic.span.end.line
            )?;
        }
        writeln!(writer, "::{body}")?;
    }

    Ok(())
}

fn write_gitlab_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
) -> io::Result<()> {
    let values = diagnostics
        .iter()
        .map(gitlab_diagnostic)
        .collect::<Vec<_>>();
    serde_json::to_writer_pretty(&mut *writer, &values).map_err(io::Error::other)?;
    writeln!(writer)?;
    Ok(())
}

fn write_rdjson_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
) -> io::Result<()> {
    let payload = RdjsonDiagnostics {
        source: RdjsonSource {
            name: "shuck",
            url: env!("CARGO_PKG_REPOSITORY"),
        },
        severity: "WARNING",
        diagnostics: diagnostics.iter().map(rdjson_diagnostic).collect(),
    };
    serde_json::to_writer_pretty(&mut *writer, &payload).map_err(io::Error::other)?;
    writeln!(writer)?;
    Ok(())
}

fn write_sarif_diagnostics(
    writer: &mut dyn Write,
    diagnostics: &[DisplayedDiagnostic],
) -> io::Result<()> {
    let results = diagnostics
        .iter()
        .map(sarif_result)
        .collect::<io::Result<Vec<_>>>()?;
    let mut rules = BTreeMap::<String, SarifRule>::new();
    for diagnostic in diagnostics {
        rules
            .entry(diagnostic.code().to_owned())
            .or_insert_with(|| sarif_rule(diagnostic));
    }

    let output = SarifOutput {
        schema: "https://json.schemastore.org/sarif-2.1.0.json",
        version: "2.1.0",
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "shuck",
                    information_uri: env!("CARGO_PKG_REPOSITORY"),
                    version: env!("CARGO_PKG_VERSION").to_owned(),
                    rules: rules.into_values().collect(),
                },
            },
            results,
        }],
    };

    serde_json::to_writer_pretty(&mut *writer, &output).map_err(io::Error::other)?;
    writeln!(writer)?;
    Ok(())
}

fn format_full_diagnostic(diagnostic: &DisplayedDiagnostic, use_color: bool) -> String {
    let Some(source) = diagnostic.source.as_deref() else {
        return format!("{}\n", format_concise_diagnostic(diagnostic, use_color));
    };
    let Some(snippet) = renderable_snippet(diagnostic.span, source) else {
        return format!("{}\n", format_concise_diagnostic(diagnostic, use_color));
    };

    let header = format_full_header(diagnostic, use_color);
    let origin = diagnostic.path.display().to_string();
    let snippet = Snippet {
        title: None,
        footer: vec![],
        slices: vec![Slice {
            source: snippet.source,
            line_start: snippet.line_start,
            origin: Some(origin.as_str()),
            fold: false,
            annotations: vec![SourceAnnotation {
                label: "",
                annotation_type: annotation_type(diagnostic),
                range: (snippet.range.start, snippet.range.end),
            }],
        }],
    };
    let renderer = if use_color {
        Renderer::styled()
    } else {
        Renderer::plain()
    };
    let rendered = renderer.render(snippet).to_string();

    format!("{header}\n{rendered}")
}

fn format_full_header(diagnostic: &DisplayedDiagnostic, use_color: bool) -> String {
    match &diagnostic.kind {
        DisplayedDiagnosticKind::ParseError => format!(
            "{}[{}]: {}",
            paint("error".to_owned(), use_color, |value| value.red().bold()),
            paint(PARSE_ERROR_CODE.to_owned(), use_color, |value| value
                .red()
                .bold()),
            diagnostic.message
        ),
        DisplayedDiagnosticKind::Lint { code, severity } => format!(
            "{}[{}]: {}",
            format_severity(severity, use_color),
            paint(code.clone(), use_color, |value| value.cyan().bold()),
            diagnostic.message
        ),
    }
}

fn format_concise_diagnostic(diagnostic: &DisplayedDiagnostic, use_color: bool) -> String {
    let path = paint(diagnostic.path.display().to_string(), use_color, |value| {
        value.bold()
    });
    let line = paint(diagnostic.span.start.line.to_string(), use_color, |value| {
        value.cyan()
    });
    let column = paint(
        diagnostic.span.start.column.to_string(),
        use_color,
        |value| value.cyan(),
    );

    match &diagnostic.kind {
        DisplayedDiagnosticKind::ParseError => {
            let label = paint("parse error".to_owned(), use_color, |value| {
                value.red().bold()
            });
            format!("{path}:{line}:{column}: {label} {}", diagnostic.message)
        }
        DisplayedDiagnosticKind::Lint { code, severity } => {
            let severity = format_severity(severity, use_color);
            let code = paint(code.clone(), use_color, |value| value.cyan().bold());
            format!(
                "{path}:{line}:{column}: {severity}[{code}] {}",
                diagnostic.message
            )
        }
    }
}

fn format_diagnostic_body(diagnostic: &DisplayedDiagnostic, use_color: bool) -> String {
    match &diagnostic.kind {
        DisplayedDiagnosticKind::ParseError => {
            let label = paint("parse error".to_owned(), use_color, |value| {
                value.red().bold()
            });
            format!("{label} {}", diagnostic.message)
        }
        DisplayedDiagnosticKind::Lint { code, severity } => {
            let severity = format_severity(severity, use_color);
            let code = paint(code.clone(), use_color, |value| value.cyan().bold());
            format!("{severity}[{code}] {}", diagnostic.message)
        }
    }
}

fn format_severity(severity: &str, use_color: bool) -> String {
    paint(severity.to_owned(), use_color, |value| match severity {
        "error" => value.red().bold(),
        "warning" => value.yellow().bold(),
        "info" | "hint" => value.blue().bold(),
        _ => value.bold(),
    })
}

fn paint(
    value: String,
    use_color: bool,
    style: impl FnOnce(ColoredString) -> ColoredString,
) -> String {
    if use_color {
        style(value.normal()).to_string()
    } else {
        value
    }
}

fn annotation_type(diagnostic: &DisplayedDiagnostic) -> AnnotationType {
    match &diagnostic.kind {
        DisplayedDiagnosticKind::ParseError => AnnotationType::Error,
        DisplayedDiagnosticKind::Lint { .. } => AnnotationType::Error,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct JsonLocation {
    row: usize,
    column: usize,
}

impl From<DisplayPosition> for JsonLocation {
    fn from(value: DisplayPosition) -> Self {
        Self {
            row: value.line,
            column: value.column,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct JsonEdit {
    content: String,
    location: JsonLocation,
    end_location: JsonLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct JsonFix {
    applicability: &'static str,
    message: Option<String>,
    edits: Vec<JsonEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct JsonDiagnostic {
    code: String,
    severity: String,
    url: Option<String>,
    message: String,
    fix: Option<JsonFix>,
    location: JsonLocation,
    end_location: JsonLocation,
    filename: String,
}

fn json_diagnostic(diagnostic: &DisplayedDiagnostic) -> JsonDiagnostic {
    JsonDiagnostic {
        code: diagnostic.code().to_owned(),
        severity: diagnostic.severity().to_owned(),
        url: None,
        message: diagnostic.message.clone(),
        fix: diagnostic.fix.as_ref().map(json_fix),
        location: diagnostic.span.start.into(),
        end_location: diagnostic.span.end.into(),
        filename: diagnostic.display_path_string(),
    }
}

fn json_fix(fix: &DisplayedFix) -> JsonFix {
    JsonFix {
        applicability: fix.applicability.as_str(),
        message: fix.message.clone(),
        edits: fix
            .edits
            .iter()
            .map(|edit| JsonEdit {
                content: edit.content.clone(),
                location: edit.location.into(),
                end_location: edit.end_location.into(),
            })
            .collect(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GitlabPosition {
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GitlabPositions {
    begin: GitlabPosition,
    end: GitlabPosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GitlabLocation {
    path: String,
    positions: GitlabPositions,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct GitlabDiagnostic {
    description: String,
    check_name: String,
    severity: &'static str,
    fingerprint: String,
    location: GitlabLocation,
}

fn gitlab_diagnostic(diagnostic: &DisplayedDiagnostic) -> GitlabDiagnostic {
    GitlabDiagnostic {
        description: format!("{}: {}", diagnostic.code(), diagnostic.message),
        check_name: diagnostic.code().to_owned(),
        severity: gitlab_severity(diagnostic.severity()),
        fingerprint: gitlab_fingerprint(diagnostic),
        location: GitlabLocation {
            path: diagnostic.display_path_string(),
            positions: GitlabPositions {
                begin: GitlabPosition {
                    line: diagnostic.span.start.line,
                    column: diagnostic.span.start.column,
                },
                end: GitlabPosition {
                    line: diagnostic.span.end.line,
                    column: diagnostic.span.end.column,
                },
            },
        },
    }
}

fn gitlab_severity(severity: &str) -> &'static str {
    match severity {
        "hint" | "info" => "info",
        "warning" => "minor",
        "error" => "major",
        _ => "critical",
    }
}

fn gitlab_fingerprint(diagnostic: &DisplayedDiagnostic) -> String {
    let mut hasher = DefaultHasher::new();
    diagnostic.code().hash(&mut hasher);
    diagnostic.path.hash(&mut hasher);
    diagnostic.message.hash(&mut hasher);
    diagnostic.span.start.line.hash(&mut hasher);
    diagnostic.span.start.column.hash(&mut hasher);
    diagnostic.span.end.line.hash(&mut hasher);
    diagnostic.span.end.column.hash(&mut hasher);
    diagnostic.severity().hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RdjsonSource {
    name: &'static str,
    url: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RdjsonDiagnostics {
    source: RdjsonSource,
    severity: &'static str,
    diagnostics: Vec<RdjsonDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RdjsonCode {
    value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RdjsonLineColumn {
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RdjsonRange {
    start: RdjsonLineColumn,
    end: RdjsonLineColumn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RdjsonLocation {
    path: String,
    range: RdjsonRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RdjsonSuggestion {
    range: RdjsonRange,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RdjsonDiagnostic {
    code: RdjsonCode,
    location: RdjsonLocation,
    message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    suggestions: Vec<RdjsonSuggestion>,
}

fn rdjson_diagnostic(diagnostic: &DisplayedDiagnostic) -> RdjsonDiagnostic {
    RdjsonDiagnostic {
        code: RdjsonCode {
            value: diagnostic.code().to_owned(),
            url: None,
        },
        location: RdjsonLocation {
            path: diagnostic.display_path_string(),
            range: rdjson_range(diagnostic.span.start, diagnostic.span.end),
        },
        message: diagnostic.message.clone(),
        suggestions: diagnostic
            .fix
            .as_ref()
            .map(|fix| {
                fix.edits
                    .iter()
                    .map(|edit| RdjsonSuggestion {
                        range: rdjson_range(edit.location, edit.end_location),
                        text: edit.content.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn rdjson_range(start: DisplayPosition, end: DisplayPosition) -> RdjsonRange {
    RdjsonRange {
        start: RdjsonLineColumn {
            line: start.line,
            column: start.column,
        },
        end: RdjsonLineColumn {
            line: end.line,
            column: end.column,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SarifOutput {
    #[serde(rename = "$schema")]
    schema: &'static str,
    version: &'static str,
    runs: Vec<SarifRun>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SarifRun {
    tool: SarifTool,
    results: Vec<SarifResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SarifTool {
    driver: SarifDriver,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifDriver {
    name: &'static str,
    information_uri: &'static str,
    version: String,
    rules: Vec<SarifRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRule {
    id: String,
    short_description: SarifMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_description: Option<SarifMessage>,
    help: SarifMessage,
    #[serde(skip_serializing_if = "Option::is_none")]
    help_uri: Option<String>,
    properties: SarifProperties,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifProperties {
    id: String,
    kind: String,
    name: String,
    #[serde(rename = "problem.severity")]
    problem_severity: SarifLevel,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult {
    rule_id: String,
    level: SarifLevel,
    message: SarifMessage,
    locations: Vec<SarifLocation>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    fixes: Vec<SarifFix>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SarifMessage {
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifLocation {
    physical_location: SarifPhysicalLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifPhysicalLocation {
    artifact_location: SarifArtifactLocation,
    region: SarifRegion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifArtifactLocation {
    uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifRegion {
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifFix {
    artifact_changes: Vec<SarifArtifactChange>,
    description: SarifDescription,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifDescription {
    text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifArtifactChange {
    artifact_location: SarifArtifactLocation,
    replacements: Vec<SarifReplacement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifReplacement {
    deleted_region: SarifRegion,
    #[serde(skip_serializing_if = "Option::is_none")]
    inserted_content: Option<SarifInsertedContent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifInsertedContent {
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
enum SarifLevel {
    Note,
    Warning,
    Error,
}

fn sarif_result(diagnostic: &DisplayedDiagnostic) -> io::Result<SarifResult> {
    let uri = diagnostic.absolute_uri()?;
    Ok(SarifResult {
        rule_id: diagnostic.code().to_owned(),
        level: sarif_level(diagnostic.severity()),
        message: SarifMessage {
            text: diagnostic.message.clone(),
        },
        locations: vec![SarifLocation {
            physical_location: SarifPhysicalLocation {
                artifact_location: SarifArtifactLocation { uri: uri.clone() },
                region: SarifRegion {
                    start_line: diagnostic.span.start.line,
                    start_column: diagnostic.span.start.column,
                    end_line: diagnostic.span.end.line,
                    end_column: diagnostic.span.end.column.max(diagnostic.span.start.column),
                },
            },
        }],
        fixes: diagnostic
            .fix
            .as_ref()
            .map(|fix| {
                vec![SarifFix {
                    description: SarifDescription {
                        text: fix.message.clone(),
                    },
                    artifact_changes: vec![SarifArtifactChange {
                        artifact_location: SarifArtifactLocation { uri },
                        replacements: fix
                            .edits
                            .iter()
                            .map(|edit| SarifReplacement {
                                deleted_region: SarifRegion {
                                    start_line: edit.location.line,
                                    start_column: edit.location.column,
                                    end_line: edit.end_location.line,
                                    end_column: edit.end_location.column.max(edit.location.column),
                                },
                                inserted_content: (!edit.content.is_empty()).then(|| {
                                    SarifInsertedContent {
                                        text: edit.content.clone(),
                                    }
                                }),
                            })
                            .collect(),
                    }],
                }]
            })
            .unwrap_or_default(),
    })
}

fn sarif_rule(diagnostic: &DisplayedDiagnostic) -> SarifRule {
    match &diagnostic.kind {
        DisplayedDiagnosticKind::ParseError => SarifRule {
            id: PARSE_ERROR_CODE.to_owned(),
            short_description: SarifMessage {
                text: "Shell source could not be parsed".to_owned(),
            },
            full_description: Some(SarifMessage {
                text: "The parser could not build a valid shell syntax tree for this file."
                    .to_owned(),
            }),
            help: SarifMessage {
                text: "Fix the reported syntax issue so analysis can continue.".to_owned(),
            },
            help_uri: None,
            properties: SarifProperties {
                id: PARSE_ERROR_CODE.to_owned(),
                kind: "parser".to_owned(),
                name: PARSE_ERROR_CODE.to_owned(),
                problem_severity: SarifLevel::Error,
            },
        },
        DisplayedDiagnosticKind::Lint { code, severity } => {
            let metadata = diagnostic_rule_metadata(diagnostic);
            let category = diagnostic_rule_category(diagnostic);
            SarifRule {
                id: code.clone(),
                short_description: SarifMessage {
                    text: metadata
                        .map(|metadata| metadata.description.to_owned())
                        .unwrap_or_else(|| diagnostic.message.clone()),
                },
                full_description: metadata.map(|metadata| SarifMessage {
                    text: metadata.rationale.to_owned(),
                }),
                help: SarifMessage {
                    text: metadata
                        .map(|metadata| metadata.rationale.to_owned())
                        .unwrap_or_else(|| diagnostic.message.clone()),
                },
                help_uri: None,
                properties: SarifProperties {
                    id: code.clone(),
                    kind: category.to_owned(),
                    name: code.clone(),
                    problem_severity: sarif_level(severity),
                },
            }
        }
    }
}

fn sarif_level(severity: &str) -> SarifLevel {
    match severity {
        "hint" | "info" => SarifLevel::Note,
        "warning" => SarifLevel::Warning,
        _ => SarifLevel::Error,
    }
}

fn diagnostic_rule_metadata(diagnostic: &DisplayedDiagnostic) -> Option<&'static RuleMetadata> {
    let DisplayedDiagnosticKind::Lint { code, .. } = &diagnostic.kind else {
        return None;
    };
    let rule = code_to_rule(code)?;
    rule_metadata(rule)
}

fn diagnostic_rule_category(diagnostic: &DisplayedDiagnostic) -> &'static str {
    let DisplayedDiagnosticKind::Lint { code, .. } = &diagnostic.kind else {
        return "parser";
    };
    let Some(rule) = code_to_rule(code) else {
        return "lint";
    };
    match rule.category() {
        Category::Correctness => "correctness",
        Category::Style => "style",
        Category::Performance => "performance",
        Category::Portability => "portability",
        Category::Security => "security",
    }
}

fn escape_github_property(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
        .replace(':', "%3A")
        .replace(',', "%2C")
}

fn escape_github_message(value: &str) -> String {
    value
        .replace('%', "%25")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

struct RenderableSnippet<'a> {
    source: &'a str,
    line_start: usize,
    range: Range<usize>,
}

fn renderable_snippet(span: DisplaySpan, source: &str) -> Option<RenderableSnippet<'_>> {
    let line_index = LineIndex::new(source);
    let start = position_offset(span.start, &line_index, source)?;
    let end = position_offset(span.end, &line_index, source)?;
    let line_start = span.start.line;
    let snippet_start = usize::from(line_index.line_start(line_start)?);
    let snippet_end = snippet_end_offset(span.end.line.max(span.start.line), &line_index, source)?;
    let absolute_range = highlighted_range(start..end.max(start), span.start, &line_index, source);

    Some(RenderableSnippet {
        source: &source[snippet_start..snippet_end],
        line_start,
        range: (absolute_range.start - snippet_start)..(absolute_range.end - snippet_start),
    })
}

fn highlighted_range(
    range: Range<usize>,
    position: DisplayPosition,
    line_index: &LineIndex,
    source: &str,
) -> Range<usize> {
    if range.start != range.end {
        return range;
    }

    let line_start = usize::from(line_index.line_start(position.line).unwrap_or_default());
    let line_end = usize::from(
        line_index
            .line_range(position.line, source)
            .map(|range| range.end())
            .unwrap_or_default(),
    );

    if range.start < line_end {
        let next = source[range.start..]
            .chars()
            .next()
            .map(|ch| range.start + ch.len_utf8())
            .unwrap_or(range.start);
        range.start..next
    } else if range.start > line_start {
        let previous = source[..range.start]
            .chars()
            .next_back()
            .map(|ch| range.start - ch.len_utf8())
            .unwrap_or(range.start);
        previous..range.start
    } else {
        range
    }
}

fn position_offset(
    position: DisplayPosition,
    line_index: &LineIndex,
    source: &str,
) -> Option<usize> {
    let line_start = usize::from(line_index.line_start(position.line)?);
    let line_range = line_index.line_range(position.line, source)?;
    let line_end = usize::from(line_range.end());
    let requested = line_start.saturating_add(position.column.saturating_sub(1));
    Some(requested.min(line_end))
}

fn snippet_end_offset(line: usize, line_index: &LineIndex, source: &str) -> Option<usize> {
    Some(usize::from(line_index.line_range(line, source)?.end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diagnostic_paths(path: &str) -> (PathBuf, PathBuf, PathBuf) {
        diagnostic_paths_with_relative(path, path)
    }

    fn diagnostic_paths_with_relative(
        display_path: &str,
        relative_path: &str,
    ) -> (PathBuf, PathBuf, PathBuf) {
        let display = PathBuf::from(display_path);
        let relative = PathBuf::from(relative_path);
        let absolute = std::env::temp_dir().join(display_path);
        (display, relative, absolute)
    }

    fn lint_diagnostic(
        path: &str,
        span: DisplaySpan,
        message: &str,
        severity: &str,
        code: &str,
        source: &str,
    ) -> DisplayedDiagnostic {
        let (path, relative_path, absolute_path) = diagnostic_paths(path);
        DisplayedDiagnostic {
            path,
            relative_path,
            absolute_path,
            span,
            message: message.to_owned(),
            kind: DisplayedDiagnosticKind::Lint {
                code: code.to_owned(),
                severity: severity.to_owned(),
            },
            fix: None,
            source: Some(Arc::<str>::from(source)),
        }
    }

    fn lint_diagnostic_with_fix(
        path: &str,
        span: DisplaySpan,
        message: &str,
        severity: &str,
        code: &str,
        source: &str,
    ) -> DisplayedDiagnostic {
        let mut diagnostic = lint_diagnostic(path, span, message, severity, code, source);
        diagnostic.fix = Some(DisplayedFix {
            applicability: DisplayedApplicability::Safe,
            message: Some("apply example fix".to_owned()),
            edits: vec![DisplayedEdit {
                location: DisplayPosition::new(1, 1),
                end_location: DisplayPosition::new(1, 5),
                content: "echo".to_owned(),
            }],
        });
        diagnostic
    }

    fn parse_diagnostic(
        path: &str,
        line: usize,
        column: usize,
        message: &str,
        source: &str,
    ) -> DisplayedDiagnostic {
        let (path, relative_path, absolute_path) = diagnostic_paths(path);
        DisplayedDiagnostic {
            path,
            relative_path,
            absolute_path,
            span: DisplaySpan::point(line, column),
            message: message.to_owned(),
            kind: DisplayedDiagnosticKind::ParseError,
            fix: None,
            source: Some(Arc::<str>::from(source)),
        }
    }

    fn lint_diagnostic_with_relative_path(
        display_path: &str,
        relative_path: &str,
        span: DisplaySpan,
        message: &str,
        severity: &str,
        code: &str,
        source: &str,
    ) -> DisplayedDiagnostic {
        let (path, relative_path, absolute_path) =
            diagnostic_paths_with_relative(display_path, relative_path);
        DisplayedDiagnostic {
            path,
            relative_path,
            absolute_path,
            span,
            message: message.to_owned(),
            kind: DisplayedDiagnosticKind::Lint {
                code: code.to_owned(),
                severity: severity.to_owned(),
            },
            fix: None,
            source: Some(Arc::<str>::from(source)),
        }
    }

    fn render_full(diagnostic: &DisplayedDiagnostic) -> String {
        format_full_diagnostic(diagnostic, false)
    }

    #[test]
    fn renders_single_line_lint_snippet() {
        let diagnostic = lint_diagnostic(
            "script.sh",
            DisplaySpan::new(DisplayPosition::new(1, 6), DisplayPosition::new(1, 10)),
            "legacy backticks",
            "warning",
            "S005",
            "echo `pwd`\n",
        );

        insta::assert_snapshot!(render_full(&diagnostic), @r"
warning[S005]: legacy backticks
 --> script.sh:1:6
  |
1 | echo `pwd`
  |      ^^^^
  |
");
    }

    #[test]
    fn renders_multi_line_lint_snippet() {
        let diagnostic = lint_diagnostic(
            "script.sh",
            DisplaySpan::new(DisplayPosition::new(2, 4), DisplayPosition::new(3, 9)),
            "quoted regular expression literal",
            "error",
            "C010",
            "if true; then\n  [[ $foo =~ \"bar\"\n    && $bar ]]\nfi\n",
        );

        insta::assert_snapshot!(render_full(&diagnostic), @r#"
error[C010]: quoted regular expression literal
 --> script.sh:2:4
  |
2 |     [[ $foo =~ "bar"
  |  ____^
3 | |     && $bar ]]
  | |________^
  |
"#);
    }

    #[test]
    fn renders_parse_error_snippet() {
        let diagnostic = parse_diagnostic(
            "broken.sh",
            2,
            1,
            "unterminated construct",
            "#!/bin/bash\nif true\n",
        );

        insta::assert_snapshot!(render_full(&diagnostic), @r"
error[parse-error]: unterminated construct
 --> broken.sh:2:1
  |
2 | if true
  | ^
  |
");
    }

    #[test]
    fn keeps_tabs_and_unicode_aligned() {
        let diagnostic = lint_diagnostic(
            "script.sh",
            DisplaySpan::new(DisplayPosition::new(2, 8), DisplayPosition::new(2, 12)),
            "legacy backticks",
            "warning",
            "S005",
            "printf '🔉'\n\tfoo=`pwd`\n",
        );

        insta::assert_snapshot!(render_full(&diagnostic), @r"
warning[S005]: legacy backticks
 --> script.sh:2:8
  |
2 | 	foo=`pwd`
  |       ^^^
  |
");
    }

    #[test]
    fn renders_concise_output_exactly() {
        let diagnostic = lint_diagnostic(
            "script.sh",
            DisplaySpan::new(DisplayPosition::new(3, 14), DisplayPosition::new(3, 18)),
            "example message",
            "warning",
            "C014",
            "echo ok\n",
        );

        assert_eq!(
            format_concise_diagnostic(&diagnostic, false),
            "script.sh:3:14: warning[C014] example message"
        );
    }

    #[test]
    fn renders_grouped_output() {
        let diagnostics = vec![
            lint_diagnostic(
                "alpha.sh",
                DisplaySpan::point(2, 1),
                "alpha message",
                "warning",
                "C001",
                "unused=1\n",
            ),
            parse_diagnostic("beta.sh", 3, 4, "broken syntax", "if true\n"),
        ];

        let mut output = Vec::new();
        print_report_to(
            &mut output,
            &diagnostics,
            CheckOutputFormatArg::Grouped,
            false,
        )
        .unwrap();

        insta::assert_snapshot!(String::from_utf8(output).unwrap(), @r"
alpha.sh:
  2:1: warning[C001] alpha message

beta.sh:
  3:4: parse error broken syntax
");
    }

    #[test]
    fn renders_github_output_for_multi_line_span() {
        let diagnostics = vec![lint_diagnostic(
            "script.sh",
            DisplaySpan::new(DisplayPosition::new(2, 4), DisplayPosition::new(3, 9)),
            "quoted regular expression literal",
            "error",
            "C010",
            "if true; then\n  [[ $foo =~ \"bar\"\n    && $bar ]]\nfi\n",
        )];

        let mut output = Vec::new();
        print_report_to(
            &mut output,
            &diagnostics,
            CheckOutputFormatArg::Github,
            false,
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "::error title=shuck (C010),file=script.sh,line=2,endLine=3::script.sh:2:4: C010 quoted regular expression literal\n",
        );
    }

    #[test]
    fn renders_json_output_with_fix() {
        let diagnostics = vec![lint_diagnostic_with_fix(
            "script.sh",
            DisplaySpan::point(2, 1),
            "variable is unused",
            "warning",
            "C001",
            "unused=1\n",
        )];

        let mut output = Vec::new();
        print_report_to(&mut output, &diagnostics, CheckOutputFormatArg::Json, false).unwrap();

        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        insta::assert_json_snapshot!(value, @r#"
        [
          {
            "code": "C001",
            "end_location": {
              "column": 1,
              "row": 2
            },
            "filename": "script.sh",
            "fix": {
              "applicability": "safe",
              "edits": [
                {
                  "content": "echo",
                  "end_location": {
                    "column": 5,
                    "row": 1
                  },
                  "location": {
                    "column": 1,
                    "row": 1
                  }
                }
              ],
              "message": "apply example fix"
            },
            "location": {
              "column": 1,
              "row": 2
            },
            "message": "variable is unused",
            "severity": "warning",
            "url": null
          }
        ]
        "#);
    }

    #[test]
    fn structured_outputs_use_display_paths() {
        let diagnostic = lint_diagnostic_with_relative_path(
            "workspace-a/script.sh",
            "script.sh",
            DisplaySpan::point(2, 1),
            "variable is unused",
            "warning",
            "C001",
            "unused=1\n",
        );

        let mut json_output = Vec::new();
        print_report_to(
            &mut json_output,
            std::slice::from_ref(&diagnostic),
            CheckOutputFormatArg::Json,
            false,
        )
        .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&json_output).unwrap();
        assert_eq!(json[0]["filename"], "workspace-a/script.sh");

        let mut github_output = Vec::new();
        print_report_to(
            &mut github_output,
            std::slice::from_ref(&diagnostic),
            CheckOutputFormatArg::Github,
            false,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(github_output).unwrap(),
            "::warning title=shuck (C001),file=workspace-a/script.sh,line=2,col=1,endLine=2,endColumn=1::workspace-a/script.sh:2:1: C001 variable is unused\n",
        );

        let mut rdjson_output = Vec::new();
        print_report_to(
            &mut rdjson_output,
            std::slice::from_ref(&diagnostic),
            CheckOutputFormatArg::Rdjson,
            false,
        )
        .unwrap();
        let rdjson: serde_json::Value = serde_json::from_slice(&rdjson_output).unwrap();
        assert_eq!(
            rdjson["diagnostics"][0]["location"]["path"],
            "workspace-a/script.sh"
        );
    }

    #[test]
    fn junit_groups_and_gitlab_fingerprints_use_display_paths() {
        let diagnostics = vec![
            lint_diagnostic_with_relative_path(
                "workspace-a/script.sh",
                "script.sh",
                DisplaySpan::point(2, 1),
                "first message",
                "warning",
                "C001",
                "unused=1\n",
            ),
            lint_diagnostic_with_relative_path(
                "workspace-b/script.sh",
                "script.sh",
                DisplaySpan::point(2, 1),
                "first message",
                "warning",
                "C001",
                "unused=1\n",
            ),
        ];

        let mut junit_output = Vec::new();
        print_report_to(
            &mut junit_output,
            &diagnostics,
            CheckOutputFormatArg::Junit,
            false,
        )
        .unwrap();
        let junit = String::from_utf8(junit_output).unwrap();
        assert!(junit.contains("testsuite name=\"workspace-a/script.sh\""));
        assert!(junit.contains("testsuite name=\"workspace-b/script.sh\""));

        let mut gitlab_output = Vec::new();
        print_report_to(
            &mut gitlab_output,
            &diagnostics,
            CheckOutputFormatArg::Gitlab,
            false,
        )
        .unwrap();
        let gitlab: serde_json::Value = serde_json::from_slice(&gitlab_output).unwrap();
        assert_eq!(gitlab[0]["location"]["path"], "workspace-a/script.sh");
        assert_eq!(gitlab[1]["location"]["path"], "workspace-b/script.sh");
        assert_ne!(gitlab[0]["fingerprint"], gitlab[1]["fingerprint"]);
    }

    #[test]
    fn renders_sarif_output_for_parse_error() {
        let diagnostics = vec![parse_diagnostic(
            "broken.sh",
            2,
            6,
            "unterminated construct",
            "#!/bin/bash\nif true\n",
        )];

        let mut output = Vec::new();
        print_report_to(
            &mut output,
            &diagnostics,
            CheckOutputFormatArg::Sarif,
            false,
        )
        .unwrap();

        let value: serde_json::Value = serde_json::from_slice(&output).unwrap();
        insta::assert_json_snapshot!(value, {
          ".runs[0].results[0].locations[0].physicalLocation.artifactLocation.uri" => "[URI]",
          ".runs[0].tool.driver.version" => "[VERSION]"
        }, @r#"
        {
          "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
          "runs": [
            {
              "results": [
                {
                  "level": "error",
                  "locations": [
                    {
                      "physicalLocation": {
                        "artifactLocation": {
                          "uri": "[URI]"
                        },
                        "region": {
                          "endColumn": 6,
                          "endLine": 2,
                          "startColumn": 6,
                          "startLine": 2
                        }
                      }
                    }
                  ],
                  "message": {
                    "text": "unterminated construct"
                  },
                  "ruleId": "parse-error"
                }
              ],
              "tool": {
                "driver": {
                  "informationUri": "https://github.com/ewhauser/shuck",
                  "name": "shuck",
                  "rules": [
                    {
                      "fullDescription": {
                        "text": "The parser could not build a valid shell syntax tree for this file."
                      },
                      "help": {
                        "text": "Fix the reported syntax issue so analysis can continue."
                      },
                      "id": "parse-error",
                      "properties": {
                        "id": "parse-error",
                        "kind": "parser",
                        "name": "parse-error",
                        "problem.severity": "error"
                      },
                      "shortDescription": {
                        "text": "Shell source could not be parsed"
                      }
                    }
                  ],
                  "version": "[VERSION]"
                }
              }
            }
          ],
          "version": "2.1.0"
        }
        "#);
    }
}

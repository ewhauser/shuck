#![warn(missing_docs)]
#![cfg_attr(not(test), warn(clippy::unwrap_used))]

//! WebAssembly bindings for source-only Shuck linting and formatting.

use std::path::Path;
use std::str::FromStr;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use shuck_ast::{TextRange, TextSize};
use shuck_formatter::{
    FormattedSource, IndentStyle, ShellDialect as FormatDialect, ShellFormatOptions,
};
use shuck_indexer::LineIndex;
use shuck_linter::{
    AnalysisRequest, Applicability, Diagnostic as ShuckDiagnostic, LinterSettings, RuleSelector,
    ShellDialect as LintDialect,
};
use shuck_parser::parser::Parser;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TYPESCRIPT_TYPES: &str = r#"
export type Shell = "sh" | "bash" | "dash" | "ksh" | "mksh" | "zsh";
export type DiagnosticSeverity = "hint" | "warning" | "error";
export type FixApplicability = "safe" | "unsafe";

export interface Position {
  /** Zero-based source line. */
  line: number;
  /** Zero-based UTF-16 code-unit offset on the line. */
  character: number;
}

export interface Range {
  start: Position;
  end: Position;
}

export interface TextEdit {
  range: Range;
  newText: string;
}

export interface DiagnosticFix {
  title: string;
  applicability: FixApplicability;
  edits: TextEdit[];
}

export interface Diagnostic {
  code: string;
  message: string;
  severity: DiagnosticSeverity;
  range: Range;
  fix?: DiagnosticFix;
}

export interface LintOptions {
  /** Logical filename used for dialect inference; no file is read. */
  filename?: string;
  /** Explicit shell dialect. Omit to infer it from the source and filename. */
  shell?: Shell;
  /** Rule selectors to enable. Omit for Shuck's defaults; [] enables no rules. */
  select?: string[];
  /** Rule selectors to remove from the selected or default rules. */
  ignore?: string[];
}

export interface FormatOptions {
  /** Logical filename used for dialect inference; no file is read. */
  filename?: string;
  /** Explicit shell dialect. Omit to infer it from the source and filename. */
  shell?: Shell;
  indentStyle?: "tab" | "space";
  indentWidth?: number;
  binaryNextLine?: boolean;
  switchCaseIndent?: boolean;
  spaceRedirects?: boolean;
  keepPadding?: boolean;
  functionNextLine?: boolean;
  neverSplit?: boolean;
  simplify?: boolean;
  minify?: boolean;
}

export function lint(source: string, options?: LintOptions): Diagnostic[];
export function format(source: string, options?: FormatOptions): string;
"#;

/// Return the Shuck version embedded in this package.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// Lint shell source and return structured editor diagnostics.
#[wasm_bindgen(skip_typescript)]
pub fn lint(source: &str, options: JsValue) -> Result<JsValue, JsValue> {
    let options = decode_options(options)?;
    let diagnostics = lint_source(source, &options).map_err(js_error)?;
    encode_value(&diagnostics)
}

/// Format shell source and return the complete formatted text.
#[wasm_bindgen(skip_typescript)]
pub fn format(source: &str, options: JsValue) -> Result<String, JsValue> {
    let options = decode_options(options)?;
    format_source(source, &options).map_err(js_error)
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ShellName {
    Sh,
    Bash,
    Dash,
    Ksh,
    Mksh,
    Zsh,
}

impl ShellName {
    const fn lint_dialect(self) -> LintDialect {
        match self {
            Self::Sh => LintDialect::Sh,
            Self::Bash => LintDialect::Bash,
            Self::Dash => LintDialect::Dash,
            Self::Ksh => LintDialect::Ksh,
            Self::Mksh => LintDialect::Mksh,
            Self::Zsh => LintDialect::Zsh,
        }
    }

    const fn format_dialect(self) -> FormatDialect {
        match self {
            Self::Sh | Self::Dash | Self::Ksh => FormatDialect::Posix,
            Self::Bash => FormatDialect::Bash,
            Self::Mksh => FormatDialect::Mksh,
            Self::Zsh => FormatDialect::Zsh,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct LintOptions {
    filename: Option<String>,
    shell: Option<ShellName>,
    select: Option<Vec<String>>,
    ignore: Vec<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum IndentStyleName {
    Tab,
    Space,
}

impl From<IndentStyleName> for IndentStyle {
    fn from(value: IndentStyleName) -> Self {
        match value {
            IndentStyleName::Tab => Self::Tab,
            IndentStyleName::Space => Self::Space,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct FormatOptions {
    filename: Option<String>,
    shell: Option<ShellName>,
    indent_style: Option<IndentStyleName>,
    indent_width: Option<u8>,
    binary_next_line: Option<bool>,
    switch_case_indent: Option<bool>,
    space_redirects: Option<bool>,
    keep_padding: Option<bool>,
    function_next_line: Option<bool>,
    never_split: Option<bool>,
    simplify: Option<bool>,
    minify: Option<bool>,
}

#[derive(Debug, Serialize)]
struct Diagnostic {
    code: String,
    message: String,
    severity: String,
    range: Range,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<DiagnosticFix>,
}

#[derive(Debug, Serialize)]
struct DiagnosticFix {
    title: String,
    applicability: String,
    edits: Vec<TextEdit>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TextEdit {
    range: Range,
    new_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct Range {
    start: Position,
    end: Position,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
struct Position {
    line: u32,
    character: u32,
}

fn lint_source(source: &str, options: &LintOptions) -> Result<Vec<Diagnostic>, String> {
    let path = options.filename.as_deref().map(Path::new);
    let shell = options.shell.map_or_else(
        || inferred_lint_dialect(source, path),
        ShellName::lint_dialect,
    );
    let settings = linter_settings(options, shell)?;
    let parsed = Parser::with_profile(source, shell.shell_profile()).parse();
    let diagnostics = AnalysisRequest::from_parse_result(&parsed, source, &settings)
        .with_optional_source_path(path)
        .lint();
    let lines = LineIndex::new(source);

    Ok(diagnostics
        .iter()
        .map(|diagnostic| diagnostic_for_js(diagnostic, source, &lines))
        .collect())
}

fn inferred_lint_dialect(source: &str, path: Option<&Path>) -> LintDialect {
    match LintDialect::infer(source, path) {
        LintDialect::Unknown => LintDialect::Bash,
        shell => shell,
    }
}

fn linter_settings(options: &LintOptions, shell: LintDialect) -> Result<LinterSettings, String> {
    let mut settings = LinterSettings::default();
    if let Some(select) = &options.select {
        settings.rules = selectors(select)?
            .iter()
            .fold(shuck_linter::RuleSet::EMPTY, |rules, selector| {
                rules.union(&selector.into_rule_set())
            });
    }
    for selector in selectors(&options.ignore)? {
        settings.rules = settings.rules.subtract(&selector.into_rule_set());
    }
    settings.shell = shell;
    settings.resolve_source_closure = false;
    Ok(settings)
}

fn selectors(values: &[String]) -> Result<Vec<RuleSelector>, String> {
    values
        .iter()
        .map(|value| RuleSelector::from_str(value).map_err(|error| error.to_string()))
        .collect()
}

fn diagnostic_for_js(diagnostic: &ShuckDiagnostic, source: &str, lines: &LineIndex) -> Diagnostic {
    let fix = diagnostic.fix.as_ref().map(|fix| DiagnosticFix {
        title: diagnostic
            .fix_title
            .clone()
            .unwrap_or_else(|| diagnostic.message.clone()),
        applicability: match fix.applicability() {
            Applicability::Safe => "safe",
            Applicability::Unsafe => "unsafe",
        }
        .to_owned(),
        edits: fix
            .edits()
            .iter()
            .map(|edit| TextEdit {
                range: range_for_js(edit.range(), source, lines),
                new_text: edit.content().to_owned(),
            })
            .collect(),
    });

    Diagnostic {
        code: diagnostic.code().to_owned(),
        message: diagnostic.message.clone(),
        severity: diagnostic.severity.as_str().to_owned(),
        range: range_for_js(diagnostic.span.to_range(), source, lines),
        fix,
    }
}

fn range_for_js(range: TextRange, source: &str, lines: &LineIndex) -> Range {
    Range {
        start: position_for_js(usize::from(range.start()), source, lines),
        end: position_for_js(usize::from(range.end()), source, lines),
    }
}

fn position_for_js(offset: usize, source: &str, lines: &LineIndex) -> Position {
    let offset = clamp_to_char_boundary(source, offset);
    let line = lines.line_number(TextSize::new(offset as u32));
    let line_start = lines.line_start(line).map(usize::from).unwrap_or_default();
    let character = source[line_start..offset].encode_utf16().count();

    Position {
        line: u32::try_from(line.saturating_sub(1)).unwrap_or(u32::MAX),
        character: u32::try_from(character).unwrap_or(u32::MAX),
    }
}

fn clamp_to_char_boundary(source: &str, offset: usize) -> usize {
    let mut offset = offset.min(source.len());
    while offset > 0 && !source.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn format_source(source: &str, options: &FormatOptions) -> Result<String, String> {
    let path = options.filename.as_deref().map(Path::new);
    let mut format_options = ShellFormatOptions::default();

    if let Some(shell) = options.shell {
        format_options = format_options.with_dialect(shell.format_dialect());
    }
    if let Some(indent_style) = options.indent_style {
        format_options = format_options.with_indent_style(indent_style.into());
    }
    if let Some(indent_width) = options.indent_width {
        format_options = format_options.with_indent_width(indent_width);
    }
    if let Some(value) = options.binary_next_line {
        format_options = format_options.with_binary_next_line(value);
    }
    if let Some(value) = options.switch_case_indent {
        format_options = format_options.with_switch_case_indent(value);
    }
    if let Some(value) = options.space_redirects {
        format_options = format_options.with_space_redirects(value);
    }
    if let Some(value) = options.keep_padding {
        format_options = format_options.with_keep_padding(value);
    }
    if let Some(value) = options.function_next_line {
        format_options = format_options.with_function_next_line(value);
    }
    if let Some(value) = options.never_split {
        format_options = format_options.with_never_split(value);
    }
    if let Some(value) = options.simplify {
        format_options = format_options.with_simplify(value);
    }
    if let Some(value) = options.minify {
        format_options = format_options.with_minify(value);
    }

    match shuck_formatter::format_source(source, path, &format_options)
        .map_err(|error| error.to_string())?
    {
        FormattedSource::Unchanged => Ok(source.to_owned()),
        FormattedSource::Formatted(formatted) => Ok(formatted),
    }
}

fn decode_options<T>(value: JsValue) -> Result<T, JsValue>
where
    T: Default + DeserializeOwned,
{
    if value.is_null() || value.is_undefined() {
        return Ok(T::default());
    }

    let serialized = js_sys::JSON::stringify(&value)?
        .as_string()
        .ok_or_else(|| js_error("options must be a JSON-compatible object"))?;
    serde_json::from_str(&serialized).map_err(js_error)
}

fn encode_value<T: Serialize>(value: &T) -> Result<JsValue, JsValue> {
    let serialized = serde_json::to_string(value).map_err(js_error)?;
    js_sys::JSON::parse(&serialized)
}

fn js_error(error: impl std::fmt::Display) -> JsValue {
    js_sys::Error::new(&error.to_string()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lint_returns_diagnostics_and_utf16_ranges() {
        let source = "😀; echo $name\n";
        let diagnostics = lint_source(
            source,
            &LintOptions {
                shell: Some(ShellName::Bash),
                select: Some(vec!["ALL".to_owned()]),
                ..LintOptions::default()
            },
        )
        .expect("lint request should succeed");

        assert!(!diagnostics.is_empty());
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "S001")
        );
        let name_offset = source.find("$name").expect("fixture contains expansion");
        assert_eq!(
            position_for_js(name_offset, source, &LineIndex::new(source)),
            Position {
                line: 0,
                character: 9,
            }
        );
    }

    #[test]
    fn lint_rejects_unknown_rule_selectors() {
        let error = lint_source(
            "echo ok\n",
            &LintOptions {
                select: Some(vec!["NOT-A-RULE".to_owned()]),
                ..LintOptions::default()
            },
        )
        .expect_err("unknown selector should fail");

        assert!(error.contains("unknown rule selector"));
    }

    #[test]
    fn formatter_uses_source_only_options() {
        let formatted = format_source(
            "hello(){\necho hi\n}\n",
            &FormatOptions {
                shell: Some(ShellName::Bash),
                indent_style: Some(IndentStyleName::Space),
                indent_width: Some(2),
                ..FormatOptions::default()
            },
        )
        .expect("format request should succeed");

        assert_eq!(formatted, "hello() {\n  echo hi\n}\n");
    }
}

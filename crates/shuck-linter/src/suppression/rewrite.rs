use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use rustc_hash::FxHashMap;
use shuck_ast::{TextRange, TextSize};
use shuck_indexer::Indexer;
use shuck_parser::{
    Error as ParseError, ShellDialect as ParseShellDialect, ShellProfile,
    parser::{ParseResult, Parser},
};

use crate::{Diagnostic, LinterSettings, Rule, ShellDialect, lint_file_at_path_with_parse_result};

use super::{
    ShellCheckCodeMap, SuppressionAction, SuppressionDirective, SuppressionIndex,
    SuppressionSource, first_statement_line, parse_directives,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddIgnoreResult {
    pub directives_added: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub parse_error: Option<AddIgnoreParseError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddIgnoreParseError {
    pub message: String,
    pub line: usize,
    pub column: usize,
}

pub fn add_ignores_to_path(
    path: &Path,
    settings: &LinterSettings,
    reason: Option<&str>,
) -> Result<AddIgnoreResult> {
    let shellcheck_map = ShellCheckCodeMap::default();
    let mut source =
        fs::read_to_string(path).with_context(|| format!("read source from {}", path.display()))?;
    let mut analysis = analyze_source(path, &source, settings, &shellcheck_map);
    let mut directives_added = 0usize;

    let target_lines = analysis
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.span.start.line)
        .collect::<BTreeSet<_>>();

    for line in target_lines {
        analysis = analyze_source(path, &source, settings, &shellcheck_map);
        let line_diagnostics = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.span.start.line == line)
            .cloned()
            .collect::<Vec<_>>();
        if line_diagnostics.is_empty() {
            continue;
        }

        let Some(edit) = build_ignore_edit(
            &source,
            &analysis,
            line,
            &line_diagnostics,
            reason,
            &shellcheck_map,
        ) else {
            continue;
        };

        let candidate_source = apply_edit(&source, &edit);
        let candidate_analysis = analyze_source(path, &candidate_source, settings, &shellcheck_map);
        if !edit_is_valid(&analysis, &candidate_analysis, line, &line_diagnostics) {
            continue;
        }

        source = candidate_source;
        analysis = candidate_analysis;
        directives_added += 1;
    }

    if directives_added > 0 {
        fs::write(path, source.as_bytes())
            .with_context(|| format!("write updated source to {}", path.display()))?;
    }

    Ok(AddIgnoreResult {
        directives_added,
        diagnostics: analysis.diagnostics,
        parse_error: analysis.parse_error,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnalyzedSource {
    directives: Vec<SuppressionDirective>,
    diagnostics: Vec<Diagnostic>,
    strict_parse_error: Option<AddIgnoreParseError>,
    parse_error: Option<AddIgnoreParseError>,
    indexer: Indexer,
}

fn analyze_source(
    path: &Path,
    source: &str,
    settings: &LinterSettings,
    shellcheck_map: &ShellCheckCodeMap,
) -> AnalyzedSource {
    let shell = resolve_shell(settings, source, Some(path));
    let parse_result = parse_for_lint(source, shell);
    let indexer = Indexer::new(source, &parse_result);
    let directives = parse_directives(
        source,
        &parse_result.file,
        indexer.comment_index(),
        shellcheck_map,
    );
    let suppression_index = (!directives.is_empty()).then(|| {
        SuppressionIndex::new(
            &directives,
            &parse_result.file,
            first_statement_line(&parse_result.file).unwrap_or(u32::MAX),
        )
    });
    let diagnostics = lint_file_at_path_with_parse_result(
        &parse_result,
        source,
        &indexer,
        settings,
        suppression_index.as_ref(),
        Some(path),
    );
    let strict_parse_error = strict_parse_error(&parse_result);
    let parse_error = strict_parse_error
        .clone()
        .filter(|_| diagnostics.is_empty());

    AnalyzedSource {
        directives,
        diagnostics,
        strict_parse_error,
        parse_error,
        indexer,
    }
}

fn strict_parse_error(parse_result: &ParseResult) -> Option<AddIgnoreParseError> {
    if !parse_result.is_err() {
        return None;
    }

    let ParseError::Parse {
        message,
        line,
        column,
    } = parse_result.strict_error();
    Some(AddIgnoreParseError {
        message: message.clone(),
        line,
        column,
    })
}

fn resolve_shell(
    settings: &LinterSettings,
    source: &str,
    source_path: Option<&Path>,
) -> ShellDialect {
    if settings.shell == ShellDialect::Unknown {
        ShellDialect::infer(source, source_path)
    } else {
        settings.shell
    }
}

fn inferred_shell_profile(shell: ShellDialect) -> ShellProfile {
    let dialect = match shell {
        ShellDialect::Sh | ShellDialect::Dash | ShellDialect::Ksh => ParseShellDialect::Posix,
        ShellDialect::Mksh => ParseShellDialect::Mksh,
        ShellDialect::Zsh => ParseShellDialect::Zsh,
        ShellDialect::Unknown | ShellDialect::Bash => ParseShellDialect::Bash,
    };
    ShellProfile::native(dialect)
}

fn parse_for_lint(source: &str, shell: ShellDialect) -> ParseResult {
    Parser::with_profile(source, inferred_shell_profile(shell)).parse()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct IgnoreEdit {
    range: TextRange,
    replacement: String,
}

fn build_ignore_edit(
    source: &str,
    analysis: &AnalyzedSource,
    line: usize,
    diagnostics: &[Diagnostic],
    reason: Option<&str>,
    shellcheck_map: &ShellCheckCodeMap,
) -> Option<IgnoreEdit> {
    let line_range = analysis.indexer.line_index().line_range(line, source)?;
    let mut line_rules = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.rule)
        .collect::<Vec<_>>();
    line_rules.sort_unstable_by_key(|rule| rule.code());
    line_rules.dedup();
    let existing_ignore = analysis.directives.iter().find(|directive| {
        directive.source == SuppressionSource::Shuck
            && directive.action == SuppressionAction::Ignore
            && usize::try_from(directive.line).ok() == Some(line)
    });

    let mut merged_rules = existing_ignore
        .map(|directive| directive.codes.clone())
        .unwrap_or_default();
    for rule in line_rules {
        if !merged_rules.contains(&rule) {
            merged_rules.push(rule);
        }
    }
    merged_rules.sort_unstable_by_key(|rule| rule.code());
    merged_rules.dedup();

    let comment_reason = reason
        .filter(|reason| !reason.is_empty())
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .or_else(|| {
            existing_ignore.and_then(|directive| {
                existing_ignore_reason(directive.range.slice(source), shellcheck_map)
            })
        });

    let mut comment = format!("# shuck: ignore={}", join_codes(&merged_rules));
    if let Some(comment_reason) = comment_reason {
        comment.push_str(" # ");
        comment.push_str(comment_reason);
    }

    if let Some(directive) = existing_ignore {
        let replacement = preserve_trailing_carriage_return(directive.range.slice(source), comment);
        return (directive.range.slice(source) != replacement).then_some(IgnoreEdit {
            range: directive.range,
            replacement,
        });
    }

    if line_ends_with_continuation(line_range, source) {
        return None;
    }

    let insertion_offset = inline_comment_insertion_offset(line_range, source);
    Some(IgnoreEdit {
        range: TextRange::new(insertion_offset, insertion_offset),
        replacement: format!("  {comment}"),
    })
}

fn preserve_trailing_carriage_return(existing: &str, mut replacement: String) -> String {
    if existing.ends_with('\r') {
        replacement.push('\r');
    }
    replacement
}

fn existing_ignore_reason<'a>(
    comment_text: &'a str,
    shellcheck_map: &ShellCheckCodeMap,
) -> Option<&'a str> {
    let body = strip_comment_prefix(comment_text);
    let remainder = strip_prefix_ignore_ascii_case(body, "shuck:")?;
    let (without_reason, reason) = remainder
        .split_once('#')
        .map_or((remainder, None), |(before, after)| {
            (before, Some(after.trim()))
        });
    let (action, codes) = without_reason.split_once('=')?;
    if parse_shuck_action(action.trim()) != Some(SuppressionAction::Ignore)
        || parse_codes(codes, |code| resolve_suppression_code(code, shellcheck_map)).is_empty()
    {
        return None;
    }

    reason.filter(|reason| !reason.is_empty())
}

fn edit_is_valid(
    current: &AnalyzedSource,
    candidate: &AnalyzedSource,
    line: usize,
    line_diagnostics: &[Diagnostic],
) -> bool {
    if candidate.strict_parse_error != current.strict_parse_error {
        return false;
    }

    let line = match u32::try_from(line) {
        Ok(line) => line,
        Err(_) => return false,
    };
    let mut target_rules = line_diagnostics
        .iter()
        .map(|diagnostic| diagnostic.rule)
        .collect::<Vec<_>>();
    target_rules.sort_unstable_by_key(|rule| rule.code());
    target_rules.dedup();
    let recognized = candidate.directives.iter().any(|directive| {
        directive.source == SuppressionSource::Shuck
            && directive.action == SuppressionAction::Ignore
            && directive.line == line
            && target_rules
                .iter()
                .all(|rule| directive.codes.iter().any(|candidate| candidate == rule))
    });
    if !recognized {
        return false;
    }

    if candidate.diagnostics.iter().any(|diagnostic| {
        diagnostic.span.start.line == usize::try_from(line).unwrap_or_default()
            && target_rules.contains(&diagnostic.rule)
    }) {
        return false;
    }

    diagnostics_match_after_removing_targets(
        &current.diagnostics,
        &candidate.diagnostics,
        line_diagnostics,
    )
}

fn diagnostics_match_after_removing_targets(
    current: &[Diagnostic],
    candidate: &[Diagnostic],
    removed: &[Diagnostic],
) -> bool {
    let mut current_counts = diagnostic_counts(current);
    let removed_counts = diagnostic_counts(removed);

    for (key, removed_count) in removed_counts {
        let Some(current_count) = current_counts.get_mut(&key) else {
            return false;
        };
        if *current_count < removed_count {
            return false;
        }
        *current_count -= removed_count;
    }

    current_counts.retain(|_, count| *count > 0);
    current_counts == diagnostic_counts(candidate)
}

fn diagnostic_counts(diagnostics: &[Diagnostic]) -> FxHashMap<DiagnosticKey, usize> {
    let mut counts = FxHashMap::default();
    for key in diagnostics.iter().map(DiagnosticKey::new) {
        *counts.entry(key).or_insert(0usize) += 1;
    }
    counts
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DiagnosticKey {
    rule: Rule,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    message: String,
}

impl DiagnosticKey {
    fn new(diagnostic: &Diagnostic) -> Self {
        Self {
            rule: diagnostic.rule,
            start_line: diagnostic.span.start.line,
            start_column: diagnostic.span.start.column,
            end_line: diagnostic.span.end.line,
            end_column: diagnostic.span.end.column,
            message: diagnostic.message.clone(),
        }
    }
}

fn apply_edit(source: &str, edit: &IgnoreEdit) -> String {
    let mut output = String::with_capacity(source.len() + edit.replacement.len());
    let start = usize::from(edit.range.start());
    let end = usize::from(edit.range.end());
    output.push_str(&source[..start]);
    output.push_str(&edit.replacement);
    output.push_str(&source[end..]);
    output
}

fn inline_comment_insertion_offset(line_range: TextRange, source: &str) -> TextSize {
    let mut end = line_range.end();
    if usize::from(end) > usize::from(line_range.start())
        && source.as_bytes()[usize::from(end) - 1] == b'\r'
    {
        end = TextSize::new(end.to_u32() - 1);
    }
    end
}

fn line_ends_with_continuation(line_range: TextRange, source: &str) -> bool {
    line_range
        .slice(source)
        .strip_suffix('\r')
        .unwrap_or(line_range.slice(source))
        .ends_with('\\')
}

fn join_codes(rules: &[Rule]) -> String {
    let mut rendered = String::new();
    for (index, rule) in rules.iter().enumerate() {
        if index > 0 {
            rendered.push_str(", ");
        }
        rendered.push_str(rule.code());
    }
    rendered
}

fn strip_comment_prefix(text: &str) -> &str {
    text.trim_start().trim_start_matches('#').trim_start()
}

fn strip_prefix_ignore_ascii_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = text.get(..prefix.len())?;
    candidate
        .eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

fn parse_shuck_action(value: &str) -> Option<SuppressionAction> {
    if value.eq_ignore_ascii_case("disable") {
        Some(SuppressionAction::Disable)
    } else if value.eq_ignore_ascii_case("disable-file") {
        Some(SuppressionAction::DisableFile)
    } else if value.eq_ignore_ascii_case("ignore") {
        Some(SuppressionAction::Ignore)
    } else {
        None
    }
}

fn parse_codes(value: &str, mut resolve: impl FnMut(&str) -> Vec<Rule>) -> Vec<Rule> {
    value
        .split(',')
        .flat_map(|code| {
            let code = code.trim();
            if code.is_empty() {
                Vec::new()
            } else {
                resolve(code)
            }
        })
        .collect()
}

fn resolve_suppression_code(code: &str, shellcheck_map: &ShellCheckCodeMap) -> Vec<Rule> {
    let mut rules = resolve_rule_code(code).into_iter().collect::<Vec<_>>();
    for rule in shellcheck_map.resolve_all(code) {
        if !rules.contains(&rule) {
            rules.push(rule);
        }
    }
    rules
}

fn resolve_rule_code(code: &str) -> Option<Rule> {
    crate::code_to_rule(code).or_else(|| {
        let upper = code.to_ascii_uppercase();
        (upper != code)
            .then(|| crate::code_to_rule(&upper))
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::Rule;

    fn run_add_ignore_with_settings(
        source: &str,
        settings: &LinterSettings,
        reason: Option<&str>,
    ) -> (AddIgnoreResult, String) {
        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("script.sh");
        fs::write(&path, source).unwrap();

        let result = add_ignores_to_path(&path, settings, reason).unwrap();
        let updated = fs::read_to_string(&path).unwrap();

        (result, updated)
    }

    fn run_add_ignore(source: &str, reason: Option<&str>) -> (AddIgnoreResult, String) {
        run_add_ignore_with_settings(source, &LinterSettings::default(), reason)
    }

    #[test]
    fn adds_inline_ignore_for_single_line_diagnostic() {
        let (result, updated) = run_add_ignore("#!/bin/bash\necho $foo\n", None);

        assert_eq!(result.directives_added, 1);
        assert!(result.diagnostics.is_empty());
        assert!(result.parse_error.is_none());
        assert_eq!(updated, "#!/bin/bash\necho $foo  # shuck: ignore=C006\n");
    }

    #[test]
    fn merges_multiple_codes_on_a_single_line() {
        let settings =
            LinterSettings::for_rules([Rule::UndefinedVariable, Rule::UnquotedExpansion]);
        let source = "#!/bin/bash\necho $foo\n";
        let (result, updated) = run_add_ignore_with_settings(source, &settings, None);

        assert_eq!(result.directives_added, 1);
        assert_eq!(
            updated,
            "#!/bin/bash\necho $foo  # shuck: ignore=C006, S001\n"
        );
    }

    #[test]
    fn merges_with_existing_ignore_and_preserves_reason() {
        let settings =
            LinterSettings::for_rules([Rule::UndefinedVariable, Rule::UnquotedExpansion]);
        let source = "#!/bin/bash\necho $foo  # shuck: ignore=S001 # legacy\n";
        let (result, updated) = run_add_ignore_with_settings(source, &settings, None);

        assert_eq!(result.directives_added, 1);
        assert_eq!(
            updated,
            "#!/bin/bash\necho $foo  # shuck: ignore=C006, S001 # legacy\n"
        );
    }

    #[test]
    fn replaces_existing_reason_when_cli_reason_is_provided() {
        let settings =
            LinterSettings::for_rules([Rule::UndefinedVariable, Rule::UnquotedExpansion]);
        let source = "#!/bin/bash\necho $foo  # shuck: ignore=S001 # legacy\n";
        let (result, updated) =
            run_add_ignore_with_settings(source, &settings, Some("intentional"));

        assert_eq!(result.directives_added, 1);
        assert_eq!(
            updated,
            "#!/bin/bash\necho $foo  # shuck: ignore=C006, S001 # intentional\n"
        );
    }

    #[test]
    fn is_idempotent_after_adding_ignore() {
        let tempdir = tempdir().unwrap();
        let path = tempdir.path().join("script.sh");
        fs::write(&path, "#!/bin/bash\necho $foo\n").unwrap();

        let first = add_ignores_to_path(&path, &LinterSettings::default(), None).unwrap();
        let second = add_ignores_to_path(&path, &LinterSettings::default(), None).unwrap();
        let updated = fs::read_to_string(&path).unwrap();

        assert_eq!(first.directives_added, 1);
        assert_eq!(second.directives_added, 0);
        assert!(second.diagnostics.is_empty());
        assert_eq!(updated, "#!/bin/bash\necho $foo  # shuck: ignore=C006\n");
    }

    #[test]
    fn leaves_existing_trailing_comments_unsupported() {
        let source = "#!/bin/bash\necho $foo # existing comment\n";
        let (result, updated) = run_add_ignore(source, None);

        assert_eq!(result.directives_added, 0);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(updated, source);
    }

    #[test]
    fn leaves_continuation_lines_unsupported() {
        let source = "#!/bin/bash\necho $foo \\\n&& echo ok\n";
        let (result, updated) = run_add_ignore(source, None);

        assert_eq!(result.directives_added, 0);
        assert_eq!(result.diagnostics.len(), 1);
        assert_eq!(updated, source);
    }

    #[test]
    fn keeps_raw_parse_errors_as_remaining_failures() {
        let (result, updated) = run_add_ignore("#!/bin/bash\necho \"unterminated\n", None);

        assert_eq!(result.directives_added, 0);
        assert!(result.diagnostics.is_empty());
        assert!(result.parse_error.is_some());
        assert_eq!(updated, "#!/bin/bash\necho \"unterminated\n");
    }

    #[test]
    fn preserves_crlf_line_endings_when_appending_ignores() {
        let source = "#!/bin/bash\r\necho $foo\r\n";
        let (result, updated) = run_add_ignore(source, None);

        assert_eq!(result.directives_added, 1);
        assert_eq!(
            updated,
            "#!/bin/bash\r\necho $foo  # shuck: ignore=C006\r\n"
        );
    }

    #[test]
    fn preserves_crlf_line_endings_when_rewriting_existing_ignores() {
        let settings =
            LinterSettings::for_rules([Rule::UndefinedVariable, Rule::UnquotedExpansion]);
        let source = "#!/bin/bash\r\necho $foo  # shuck: ignore=S001 # legacy\r\n";
        let (result, updated) = run_add_ignore_with_settings(source, &settings, None);

        assert_eq!(result.directives_added, 1);
        assert_eq!(
            updated,
            "#!/bin/bash\r\necho $foo  # shuck: ignore=C006, S001 # legacy\r\n"
        );
    }

    #[test]
    fn rejects_candidate_edits_that_introduce_parse_errors() {
        let settings = LinterSettings::for_rule(Rule::UndefinedVariable);
        let shellcheck_map = ShellCheckCodeMap::default();
        let path = Path::new("script.sh");
        let current_source = "#!/bin/bash\necho $foo\necho $bar\n";
        let candidate_source =
            "#!/bin/bash\necho $foo  # shuck: ignore=C006\necho $bar\necho \"unterminated\n";

        let current = analyze_source(path, current_source, &settings, &shellcheck_map);
        let candidate = analyze_source(path, candidate_source, &settings, &shellcheck_map);
        let line_diagnostics = current
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.span.start.line == 2)
            .cloned()
            .collect::<Vec<_>>();

        assert!(current.strict_parse_error.is_none());
        assert!(candidate.parse_error.is_none());
        assert!(candidate.strict_parse_error.is_some());
        assert!(!edit_is_valid(&current, &candidate, 2, &line_diagnostics));
    }

    #[test]
    fn rejects_candidate_edits_that_change_unrelated_diagnostics() {
        let settings = LinterSettings::for_rule(Rule::UndefinedVariable);
        let shellcheck_map = ShellCheckCodeMap::default();
        let path = Path::new("script.sh");
        let current_source = "#!/bin/bash\necho $foo\necho $bar\n";
        let candidate_source = "#!/bin/bash\necho $foo  # shuck: ignore=C006\necho ok\n";

        let current = analyze_source(path, current_source, &settings, &shellcheck_map);
        let candidate = analyze_source(path, candidate_source, &settings, &shellcheck_map);
        let line_diagnostics = current
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.span.start.line == 2)
            .cloned()
            .collect::<Vec<_>>();

        assert_eq!(current.diagnostics.len(), 2);
        assert_eq!(candidate.diagnostics.len(), 0);
        assert!(!edit_is_valid(&current, &candidate, 2, &line_diagnostics));
    }
}

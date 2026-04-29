use rustc_hash::FxHashMap;
use shuck_ast::{
    BuiltinCommand, Command, CompoundCommand, File, HeredocBodyPartNode, Redirect, Span, Stmt,
    TextSize,
};

use crate::Rule;

use super::{SuppressionAction, SuppressionDirective};

/// Per-file suppression index.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SuppressionIndex {
    by_rule: FxHashMap<Rule, RuleSuppressionIndex>,
}

impl SuppressionIndex {
    /// Build from parsed directives and a precomputed suppression command ordering.
    pub(crate) fn from_sorted_command_spans(
        directives: &[SuppressionDirective],
        sorted_command_spans: &[Span],
        first_stmt_line: u32,
    ) -> Self {
        let mut ordered = directives.iter().collect::<Vec<_>>();
        ordered.sort_by_key(|directive| {
            (
                directive.line,
                directive.range.start().to_u32(),
                directive.range.end().to_u32(),
            )
        });

        let mut by_rule = FxHashMap::default();
        for directive in ordered {
            let directive_range = (matches!(directive.action, SuppressionAction::Disable)
                && directive.line >= first_stmt_line)
                .then(|| next_command_range_after(sorted_command_spans, directive.range.end()))
                .flatten();

            for &rule in &directive.codes {
                let index = by_rule
                    .entry(rule)
                    .or_insert_with(RuleSuppressionIndex::default);
                match directive.action {
                    SuppressionAction::DisableFile => index.whole_file = true,
                    SuppressionAction::Ignore => index.lines.push(directive.line),
                    SuppressionAction::Disable => {
                        if directive.line < first_stmt_line {
                            index.whole_file = true;
                        } else if let Some(range) = directive_range {
                            index.ranges.push(range);
                        }
                    }
                }
            }
        }

        for index in by_rule.values_mut() {
            index.lines.sort_unstable();
            index.lines.dedup();
            index
                .ranges
                .sort_unstable_by_key(|range| (range.start_line, range.end_line));
            merge_overlapping_ranges(&mut index.ranges);
        }

        Self { by_rule }
    }

    /// Check if `rule` is suppressed on `line`.
    pub fn is_suppressed(&self, rule: Rule, line: u32) -> bool {
        self.by_rule
            .get(&rule)
            .is_some_and(|index| index.is_suppressed(line))
    }
}

/// First command line in the file, if any.
pub(crate) fn first_statement_line(file: &File) -> Option<u32> {
    file.body
        .iter()
        .filter_map(|command| u32::try_from(command.span.start.line).ok())
        .min()
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RuleSuppressionIndex {
    whole_file: bool,
    lines: Vec<u32>,
    ranges: Vec<LineRange>,
}

impl RuleSuppressionIndex {
    fn is_suppressed(&self, line: u32) -> bool {
        if self.whole_file {
            return true;
        }

        if self.lines.binary_search(&line).is_ok() {
            return true;
        }

        let candidate = self
            .ranges
            .partition_point(|range| range.start_line <= line);
        if let Some(range) = candidate
            .checked_sub(1)
            .and_then(|index| self.ranges.get(index))
            && line <= range.end_line
        {
            return true;
        }
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineRange {
    start_line: u32,
    end_line: u32,
}

fn merge_overlapping_ranges(ranges: &mut Vec<LineRange>) {
    if ranges.len() < 2 {
        return;
    }

    let mut merged = Vec::with_capacity(ranges.len());
    let mut current = ranges[0];

    for range in ranges.iter().copied().skip(1) {
        if range.start_line <= current.end_line {
            current.end_line = current.end_line.max(range.end_line);
        } else {
            merged.push(current);
            current = range;
        }
    }
    merged.push(current);
    *ranges = merged;
}

pub(crate) fn sort_command_spans_for_lookup(spans: &mut [Span]) {
    // Sort by start offset first, then prefer wider spans when two commands
    // start at the same byte offset so parent statements keep priority over
    // children during `next_command_range_after`.
    spans.sort_by(|left, right| {
        left.start
            .offset
            .cmp(&right.start.offset)
            .then_with(|| right.end.offset.cmp(&left.end.offset))
    });
}

fn next_command_range_after(spans: &[Span], offset: TextSize) -> Option<LineRange> {
    let offset = offset.to_u32() as usize;
    let idx = spans.partition_point(|span| span.start.offset <= offset);
    spans.get(idx).copied().and_then(line_range)
}

fn line_range(span: Span) -> Option<LineRange> {
    let start_line = u32::try_from(span.start.line).ok()?;
    let mut end_line = u32::try_from(span.end.line).ok()?;
    if span.end.offset > span.start.offset && span.end.column == 1 {
        end_line = end_line.saturating_sub(1);
    }

    Some(LineRange {
        start_line,
        end_line: end_line.max(start_line),
    })
}

pub(crate) fn statement_suppression_span(stmt: &Stmt) -> Span {
    let mut span = command_suppression_base_span(stmt);
    extend_span_with_redirects(&mut span, &stmt.redirects);
    span
}

fn command_suppression_base_span(stmt: &Stmt) -> Span {
    match &stmt.command {
        Command::Simple(command) => command.span,
        Command::Builtin(BuiltinCommand::Break(command)) => command.span,
        Command::Builtin(BuiltinCommand::Continue(command)) => command.span,
        Command::Builtin(BuiltinCommand::Return(command)) => command.span,
        Command::Builtin(BuiltinCommand::Exit(command)) => command.span,
        Command::Decl(command) => command.span,
        Command::Binary(command) => command.span,
        Command::Compound(command) => compound_suppression_base_span(command, stmt.span),
        Command::Function(command) => command.span,
        Command::AnonymousFunction(command) => command.span,
    }
}

fn compound_suppression_base_span(command: &CompoundCommand, fallback: Span) -> Span {
    match command {
        CompoundCommand::If(command) => command.span,
        CompoundCommand::For(command) => command.span,
        CompoundCommand::Repeat(command) => command.span,
        CompoundCommand::Foreach(command) => command.span,
        CompoundCommand::ArithmeticFor(command) => command.span,
        CompoundCommand::While(command) => command.span,
        CompoundCommand::Until(command) => command.span,
        CompoundCommand::Case(command) => command.span,
        CompoundCommand::Select(command) => command.span,
        CompoundCommand::Subshell(_) | CompoundCommand::BraceGroup(_) => fallback,
        CompoundCommand::Arithmetic(command) => command.span,
        CompoundCommand::Time(command) => command.span,
        CompoundCommand::Conditional(command) => command.span,
        CompoundCommand::Coproc(command) => command.span,
        CompoundCommand::Always(command) => command.span,
    }
}

fn extend_span_with_redirects(span: &mut Span, redirects: &[Redirect]) {
    for redirect in redirects {
        if let Some(heredoc) = redirect.heredoc() {
            extend_span_with_heredoc_body(span, &heredoc.body.parts);
        }
    }
}

fn extend_span_with_heredoc_body(span: &mut Span, parts: &[HeredocBodyPartNode]) {
    for part in parts {
        match &part.kind {
            shuck_ast::HeredocBodyPart::Variable(_)
            | shuck_ast::HeredocBodyPart::Parameter(_)
            | shuck_ast::HeredocBodyPart::CommandSubstitution { .. }
            | shuck_ast::HeredocBodyPart::ArithmeticExpansion { .. } => {
                extend_span(span, part.span);
            }
            shuck_ast::HeredocBodyPart::Literal(_) => {}
        }
    }
}

fn extend_span(span: &mut Span, extension: Span) {
    if extension.start.line == 0 || extension.end.line == 0 {
        return;
    }
    if span.start.line == 0 || extension.start.offset < span.start.offset {
        span.start = extension.start;
    }
    if span.end.line == 0 || extension.end.offset > span.end.offset {
        span.end = extension.end;
    }
}

#[cfg(test)]
mod tests {
    use shuck_indexer::Indexer;
    use shuck_parser::parser::{Parser, ShellDialect};

    use super::*;
    use crate::{LinterSemanticArtifacts, ShellCheckCodeMap, parse_directives};

    fn suppression_index(source: &str) -> SuppressionIndex {
        suppression_index_with_dialect(source, ShellDialect::Bash)
    }

    fn suppression_index_with_dialect(source: &str, dialect: ShellDialect) -> SuppressionIndex {
        let output = Parser::with_dialect(source, dialect).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        let directives = parse_directives(
            source,
            &output.file,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        let semantic = LinterSemanticArtifacts::build(&output.file, source, &indexer);
        SuppressionIndex::from_sorted_command_spans(
            &directives,
            semantic.suppression_command_spans(),
            first_statement_line(&output.file).unwrap_or(u32::MAX),
        )
    }

    #[test]
    fn applies_disable_file_directives_to_the_entire_file() {
        let source = "echo $foo # shuck: disable-file=C006\n";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UndefinedVariable, 1));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 200));
    }

    #[test]
    fn applies_shuck_disable_to_the_next_command() {
        let source = "\
foo='a b'
echo $foo
# shuck: disable=C006
echo $foo
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UndefinedVariable, 2));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 4));
        assert!(!index.is_suppressed(Rule::UndefinedVariable, 5));
    }

    #[test]
    fn applies_shuck_ignore_only_to_the_directive_line() {
        let source = "\
foo='a b'
echo $foo # shuck: ignore=C006
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UndefinedVariable, 1));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 2));
        assert!(!index.is_suppressed(Rule::UndefinedVariable, 3));
    }

    #[test]
    fn applies_dead_code_alias_suppressions() {
        let source = "\
exit 0
# shuck: disable=SH-293
echo dead
echo still_dead
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnreachableAfterExit, 1));
        assert!(index.is_suppressed(Rule::UnreachableAfterExit, 3));
        assert!(!index.is_suppressed(Rule::UnreachableAfterExit, 4));
    }

    #[test]
    fn applies_legacy_shellcheck_dead_code_suppression_alias() {
        let source = "\
exit 0
# shellcheck disable=SC2365
echo dead
echo still_dead
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnreachableAfterExit, 1));
        assert!(index.is_suppressed(Rule::UnreachableAfterExit, 3));
        assert!(!index.is_suppressed(Rule::UnreachableAfterExit, 4));
    }

    #[test]
    fn applies_shuck_disable_with_shellcheck_codes_to_the_next_command() {
        let source = "\
foo='a b'
echo $foo
# shuck: disable=SC2086
echo $foo
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 2));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn promotes_shuck_disable_before_the_first_statement_to_file_scope() {
        let source = "\
#!/bin/bash
# shuck: disable=S001

echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 1));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
    }

    #[test]
    fn promotes_shellcheck_directives_before_the_first_statement_to_file_scope() {
        let source = "\
#!/bin/bash
# shellcheck disable=SC2086

echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 1));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
    }

    #[test]
    fn promotes_shellcheck_disable_all_before_the_first_statement_to_file_scope() {
        let source = "\
#!/bin/bash
# shellcheck disable=all

[ \"$a\" = 1 -a \"$b\" = 2 ]
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::CompoundTestOperator, 4));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_with_shuck_codes_to_the_next_command() {
        let source = "\
foo='a b'
# shellcheck disable=S001
echo $foo
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 4));
    }

    #[test]
    fn scopes_shuck_disable_after_then_header_to_the_next_command() {
        let source = "\
foo='a b'
if true; then # shuck: disable=S001
  echo $foo
fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_to_the_next_multiline_command() {
        let source = "\
echo ready
# shellcheck disable=SC2086
if true; then
  echo $foo
fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 5));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 6));
    }

    #[test]
    fn scopes_shellcheck_disable_inside_function_to_heredoc_body() {
        let source = "\
#!/bin/bash
echo ready
emit_file() {
  # shellcheck disable=SC2154
  cat \"$path\" <<EOF
value=$body_value
other=${other_value}
EOF
  echo \"$later\"
}
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UndefinedVariable, 5));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 6));
        assert!(index.is_suppressed(Rule::UndefinedVariable, 7));
        assert!(!index.is_suppressed(Rule::UndefinedVariable, 9));
    }

    #[test]
    fn finds_next_commands_inside_command_substitutions() {
        let source = "\
echo ready
value=\"$(
  # shellcheck disable=SC2086
  echo $foo
)\"
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 6));
    }

    #[test]
    fn scopes_shellcheck_disable_to_the_full_multiline_binary_statement() {
        let source = "\
foo='a b'
# shellcheck disable=SC2086
echo $foo &&
  echo $bar
echo $baz
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_to_the_next_function_body() {
        let source = "\
# shellcheck disable=SC2059
plugin_current_command() {
  printf \"$terminal_format\" \"$plugin\" \"$version\" \"$description\" 1>&2
}
printf \"$later\" value
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::PrintfFormatVariable, 3));
        assert!(index.is_suppressed(Rule::PrintfFormatVariable, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_case_label_to_the_next_command() {
        let source = "\
case $x in
  on) # shellcheck disable=SC2086
    echo $foo
    ;;
esac
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_if_header_to_the_next_command() {
        let source = "\
foo='a b'
if # shellcheck disable=SC2086
  echo $foo
then
  echo $foo
fi
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_then_header_to_the_next_command() {
        let source = "\
foo='a b'
if true; then # shellcheck disable=SC2086
  echo $foo
fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_elif_then_header_to_the_next_command() {
        let source = "\
foo='a b'
if false; then
  :
elif true; then # shellcheck disable=SC2086
  echo $foo
fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 5));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 7));
    }

    #[test]
    fn scopes_shellcheck_disable_after_for_do_header_to_the_next_command() {
        let source = "\
foo='a b'
for item in 1; do # shellcheck disable=SC2086
  echo $foo
done
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_brace_group_opener_to_the_next_command() {
        let source = "\
foo='a b'
{ # shellcheck disable=SC2086
  echo $foo
}
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_then_inline_brace_group_opener() {
        let source = "\
foo='a b'
if true; then { # shellcheck disable=SC2086
  echo $foo
}; fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_then_inline_subshell_opener() {
        let source = "\
foo='a b'
if true; then ( # shellcheck disable=SC2086
  echo $foo
); fi
echo $foo
";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 5));
    }

    #[test]
    fn scopes_shellcheck_disable_after_zsh_brace_if_headers() {
        let source = "\
foo='a b'
if [[ -n $foo ]] { # shellcheck disable=SC2086
  echo $foo
} elif [[ -n $foo ]] { # shellcheck disable=SC2086
  echo $foo
} else { # shellcheck disable=SC2086
  echo $foo
}
echo $foo
";
        let index = suppression_index_with_dialect(source, ShellDialect::Zsh);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 5));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 7));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 9));
    }

    #[test]
    fn scopes_shellcheck_disable_after_zsh_brace_loop_headers() {
        let source = "\
foo='a b'
for item in 1; { # shellcheck disable=SC2086
  echo $foo
}
repeat 2 { # shellcheck disable=SC2086
  echo $foo
}
foreach item (1 2) { # shellcheck disable=SC2086
  echo $foo
}
echo $foo
";
        let index = suppression_index_with_dialect(source, ShellDialect::Zsh);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 3));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 6));
        assert!(index.is_suppressed(Rule::UnquotedExpansion, 9));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 11));
    }

    #[test]
    fn ignores_shellcheck_directives_after_keyword_like_arguments() {
        let source = "\
echo if # shellcheck disable=SC2086
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 2));
    }

    #[test]
    fn ignores_shellcheck_directives_after_keyword_suffixes_inside_words() {
        let source = "\
foo='a b'
for item in to-do # shellcheck disable=SC2086
do
  echo $foo
done
echo $foo
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 6));
    }

    #[test]
    fn ignores_case_label_directives_after_same_line_body_commands() {
        let source = "\
case $x in
  on) echo $foo # shellcheck disable=SC2086
    echo $bar
    ;;
esac
";
        let index = suppression_index(source);

        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 2));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 3));
    }

    #[test]
    fn scopes_case_label_directives_inside_command_substitution_arguments() {
        let source = "\
printf '%s\\n' \"$(
  case $x in
    on) # shellcheck disable=SC2086
      echo $foo
      ;;
  esac
  echo $bar
)\"\n";
        let index = suppression_index(source);

        assert!(index.is_suppressed(Rule::UnquotedExpansion, 4));
        assert!(!index.is_suppressed(Rule::UnquotedExpansion, 7));
    }
}

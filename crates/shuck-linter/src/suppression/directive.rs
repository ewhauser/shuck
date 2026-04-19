use shuck_ast::{CaseItem, Command, CompoundCommand, File, IfSyntax, StmtSeq, TextRange, TextSize};
use shuck_indexer::{CommentIndex, IndexedComment};

use crate::{
    Rule, code_to_rule,
    rules::common::query::{CommandWalkOptions, iter_commands},
};

use super::ShellCheckCodeMap;

/// A parsed suppression directive from a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuppressionDirective {
    /// The action: disable, enable, or disable-file.
    pub action: SuppressionAction,
    /// Which directive syntax produced this.
    pub source: SuppressionSource,
    /// Rule codes this directive applies to.
    pub codes: Vec<Rule>,
    /// The comment's source range.
    pub range: TextRange,
    /// 1-based line number of the directive.
    pub line: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionAction {
    Disable,
    Enable,
    DisableFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuppressionSource {
    Shuck,
    ShellCheck,
}

/// Parse all suppression directives from a file's comments.
pub fn parse_directives(
    source: &str,
    file: &File,
    comment_index: &CommentIndex,
    shellcheck_map: &ShellCheckCodeMap,
) -> Vec<SuppressionDirective> {
    let mut directives = Vec::new();

    for comment in comment_index.comments() {
        let Some(comment) = normalized_comment(source, comment) else {
            continue;
        };

        if let Some(directive) = parse_shuck_directive(&comment) {
            directives.push(directive);
        } else if let Some(directive) =
            parse_shellcheck_directive(source, &comment, file, shellcheck_map)
        {
            directives.push(directive);
        }
    }

    directives.sort_by_key(|directive| {
        (
            directive.line,
            directive.range.start().to_u32(),
            directive.range.end().to_u32(),
        )
    });
    directives
}

#[derive(Debug, Clone, Copy)]
struct NormalizedComment<'a> {
    text: &'a str,
    range: TextRange,
    line: u32,
    is_own_line: bool,
}

fn normalized_comment<'a>(
    source: &'a str,
    comment: &IndexedComment,
) -> Option<NormalizedComment<'a>> {
    let start = usize::from(comment.range.start()).min(source.len());
    let end = usize::from(comment.range.end()).min(source.len());
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = source[end..]
        .find('\n')
        .map_or(source.len(), |index| end + index);
    let line = &source[line_start..line_end];
    let relative_start = start.saturating_sub(line_start).min(line.len());
    let relative_end = end.saturating_sub(line_start).min(line.len());
    let marker = find_comment_marker(line, relative_start, relative_end)?;
    let comment_start = line_start + marker;

    Some(NormalizedComment {
        text: &source[comment_start..line_end],
        range: TextRange::new(
            TextSize::new(comment_start as u32),
            TextSize::new(line_end as u32),
        ),
        line: u32::try_from(comment.line).ok()?,
        is_own_line: is_horizontal_whitespace(&source[line_start..comment_start]),
    })
}

fn find_comment_marker(line: &str, start: usize, end: usize) -> Option<usize> {
    line.match_indices('#')
        .min_by_key(|(index, _)| {
            (
                index.abs_diff(start),
                usize::from(*index < start),
                index.abs_diff(end),
            )
        })
        .map(|(index, _)| index)
}

fn is_horizontal_whitespace(text: &str) -> bool {
    text.chars().all(|ch| matches!(ch, ' ' | '\t' | '\r'))
}

fn parse_shuck_directive(comment: &NormalizedComment<'_>) -> Option<SuppressionDirective> {
    let body = strip_comment_prefix(comment.text);
    let remainder = strip_prefix_ignore_ascii_case(body, "shuck:")?;
    let remainder = remainder
        .split_once('#')
        .map_or(remainder, |(before, _)| before);
    let (action, codes) = remainder.split_once('=')?;
    let action = parse_shuck_action(action.trim())?;
    let codes = parse_codes(codes, resolve_rule_code);
    if codes.is_empty() {
        return None;
    }

    Some(SuppressionDirective {
        action,
        source: SuppressionSource::Shuck,
        codes,
        range: comment.range,
        line: comment.line,
    })
}

fn parse_shellcheck_directive(
    source: &str,
    comment: &NormalizedComment<'_>,
    file: &File,
    shellcheck_map: &ShellCheckCodeMap,
) -> Option<SuppressionDirective> {
    if !comment.is_own_line
        && !shellcheck_directive_can_apply_to_following_command(source, file, comment.range)
        && !is_case_label_directive(comment, file)
    {
        return None;
    }

    let body = strip_comment_prefix(comment.text);
    let remainder = strip_prefix_ignore_ascii_case(body, "shellcheck")?;
    if let Some(first) = remainder.chars().next()
        && !first.is_ascii_whitespace()
    {
        return None;
    }

    let mut codes = Vec::new();
    for part in remainder.split_ascii_whitespace() {
        if let Some(group) = strip_prefix_ignore_ascii_case(part, "disable=") {
            for code in group.split(',') {
                let code = code.trim();
                if code.is_empty() {
                    continue;
                }
                if code.eq_ignore_ascii_case("all") {
                    codes.extend(Rule::iter());
                } else {
                    codes.extend(shellcheck_map.resolve_all(code));
                }
            }
        }
    }

    if codes.is_empty() {
        return None;
    }

    Some(SuppressionDirective {
        action: SuppressionAction::Disable,
        source: SuppressionSource::ShellCheck,
        codes,
        range: comment.range,
        line: comment.line,
    })
}

pub(crate) fn shellcheck_directive_can_apply_to_following_command(
    source: &str,
    file: &File,
    comment_range: TextRange,
) -> bool {
    let Some(context) = inline_directive_context(source, comment_range) else {
        return false;
    };

    if context.prefix.trim_end().ends_with(';') {
        return true;
    }

    iter_commands(
        &file.body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    )
    .any(|visit| match visit.command {
        Command::Compound(CompoundCommand::If(command)) => {
            let if_header = command.condition.first().is_some_and(|stmt| {
                next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                    && command_start_segment_matches(
                        source,
                        visit.stmt.span.start.offset,
                        context.comment_start,
                        "if",
                    )
            });

            let then_header = matches!(command.syntax, IfSyntax::ThenFi { .. })
                && body_header_matches(
                    source,
                    command.condition.span.end.offset,
                    &command.then_branch,
                    &context,
                    "then",
                );

            let mut previous_branch_end = command.then_branch.span.end.offset;
            let mut elif_header = false;
            let mut elif_then_header = false;
            for (condition, branch) in &command.elif_branches {
                elif_header |= condition.first().is_some_and(|stmt| {
                    next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                        && gap_segment_ends_with_keyword(
                            source,
                            previous_branch_end,
                            context.comment_start,
                            "elif",
                        )
                });
                elif_then_header |= body_header_matches(
                    source,
                    condition.span.end.offset,
                    branch,
                    &context,
                    "then",
                );
                previous_branch_end = branch.span.end.offset;
            }

            let else_header = command.else_branch.as_ref().is_some_and(|branch| {
                branch.first().is_some_and(|stmt| {
                    next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                        && gap_segment_ends_with_keyword(
                            source,
                            previous_branch_end,
                            context.comment_start,
                            "else",
                        )
                })
            });

            if_header || then_header || elif_header || elif_then_header || else_header
        }
        Command::Compound(CompoundCommand::For(command)) => body_header_matches(
            source,
            visit.stmt.span.start.offset,
            &command.body,
            &context,
            "do",
        ),
        Command::Compound(CompoundCommand::Repeat(command)) => body_header_matches(
            source,
            visit.stmt.span.start.offset,
            &command.body,
            &context,
            "do",
        ),
        Command::Compound(CompoundCommand::Foreach(command)) => body_header_matches(
            source,
            visit.stmt.span.start.offset,
            &command.body,
            &context,
            "do",
        ),
        Command::Compound(CompoundCommand::ArithmeticFor(command)) => body_header_matches(
            source,
            visit.stmt.span.start.offset,
            &command.body,
            &context,
            "do",
        ),
        Command::Compound(CompoundCommand::While(command)) => {
            loop_header_matches(
                source,
                visit.stmt.span.start.offset,
                &command.condition,
                context,
                "while",
            ) || body_header_matches(
                source,
                command.condition.span.end.offset,
                &command.body,
                &context,
                "do",
            )
        }
        Command::Compound(CompoundCommand::Until(command)) => {
            loop_header_matches(
                source,
                visit.stmt.span.start.offset,
                &command.condition,
                context,
                "until",
            ) || body_header_matches(
                source,
                command.condition.span.end.offset,
                &command.body,
                &context,
                "do",
            )
        }
        Command::Compound(CompoundCommand::Select(command)) => body_header_matches(
            source,
            visit.stmt.span.start.offset,
            &command.body,
            &context,
            "do",
        ),
        Command::Compound(CompoundCommand::Subshell(body)) => body.first().is_some_and(|stmt| {
            next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                && context.prefix.trim_end().ends_with('(')
        }),
        Command::Compound(CompoundCommand::BraceGroup(body)) => body.first().is_some_and(|stmt| {
            next_command_starts_after_comment_line(stmt.span.start.offset, &context)
                && context.prefix.trim_end().ends_with('{')
        }),
        _ => false,
    })
}

fn is_case_label_directive(comment: &NormalizedComment<'_>, file: &File) -> bool {
    iter_commands(
        &file.body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    )
    .any(|visit| {
        let Command::Compound(CompoundCommand::Case(command)) = visit.command else {
            return false;
        };

        command
            .cases
            .iter()
            .any(|case| case_label_directive(case, comment))
    })
}

fn case_label_directive(case: &CaseItem, comment: &NormalizedComment<'_>) -> bool {
    let Some(pattern) = case.patterns.last() else {
        return false;
    };

    let Some(pattern_line) = u32::try_from(pattern.span.end.line).ok() else {
        return false;
    };
    if pattern_line != comment.line || usize::from(comment.range.start()) <= pattern.span.end.offset
    {
        return false;
    }

    case.body
        .first()
        .is_none_or(|stmt| u32::try_from(stmt.span.start.line).ok() != Some(comment.line))
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

#[derive(Debug, Clone, Copy)]
struct InlineDirectiveContext<'a> {
    prefix: &'a str,
    line_end: usize,
    comment_start: usize,
}

fn inline_directive_context<'a>(
    source: &'a str,
    comment_range: TextRange,
) -> Option<InlineDirectiveContext<'a>> {
    let comment_start = usize::from(comment_range.start()).min(source.len());
    let line_start = source[..comment_start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = source[comment_start..]
        .find('\n')
        .map_or(source.len(), |index| comment_start + index);

    Some(InlineDirectiveContext {
        prefix: &source[line_start..comment_start],
        line_end,
        comment_start,
    })
}

fn next_command_starts_after_comment_line(
    next_command_start: usize,
    context: &InlineDirectiveContext<'_>,
) -> bool {
    next_command_start >= context.line_end
}

fn command_start_segment_matches(
    source: &str,
    command_start: usize,
    comment_start: usize,
    keyword: &str,
) -> bool {
    segment_ends_with_keyword(source, command_start, comment_start, keyword)
}

fn gap_segment_ends_with_keyword(
    source: &str,
    gap_start: usize,
    comment_start: usize,
    keyword: &str,
) -> bool {
    segment_ends_with_keyword(source, gap_start.min(comment_start), comment_start, keyword)
}

fn segment_ends_with_keyword(source: &str, start: usize, end: usize, keyword: &str) -> bool {
    let Some(segment) = source.get(start.min(end)..end) else {
        return false;
    };
    let trimmed = segment.trim_end_matches([' ', '\t', '\r']);
    let Some(prefix) = trimmed.strip_suffix(keyword) else {
        return false;
    };

    prefix
        .chars()
        .next_back()
        .is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_'))
}

fn loop_header_matches(
    source: &str,
    command_start: usize,
    condition: &StmtSeq,
    context: InlineDirectiveContext<'_>,
    keyword: &str,
) -> bool {
    condition.first().is_some_and(|stmt| {
        next_command_starts_after_comment_line(stmt.span.start.offset, &context)
            && command_start_segment_matches(source, command_start, context.comment_start, keyword)
    })
}

fn body_header_matches(
    source: &str,
    header_end: usize,
    body: &StmtSeq,
    context: &InlineDirectiveContext<'_>,
    keyword: &str,
) -> bool {
    body.first().is_some_and(|stmt| {
        next_command_starts_after_comment_line(stmt.span.start.offset, context)
            && gap_segment_ends_with_keyword(source, header_end, context.comment_start, keyword)
    })
}

fn parse_shuck_action(value: &str) -> Option<SuppressionAction> {
    if value.eq_ignore_ascii_case("disable") {
        Some(SuppressionAction::Disable)
    } else if value.eq_ignore_ascii_case("enable") {
        Some(SuppressionAction::Enable)
    } else if value.eq_ignore_ascii_case("disable-file") {
        Some(SuppressionAction::DisableFile)
    } else {
        None
    }
}

fn parse_codes(value: &str, mut resolve: impl FnMut(&str) -> Option<Rule>) -> Vec<Rule> {
    value
        .split(',')
        .filter_map(|code| {
            let code = code.trim();
            if code.is_empty() { None } else { resolve(code) }
        })
        .collect()
}

fn resolve_rule_code(code: &str) -> Option<Rule> {
    code_to_rule(code).or_else(|| {
        let upper = code.to_ascii_uppercase();
        (upper != code).then(|| code_to_rule(&upper)).flatten()
    })
}

#[cfg(test)]
mod tests {
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;

    use super::*;

    fn directives(source: &str) -> Vec<SuppressionDirective> {
        let output = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &output);
        parse_directives(
            source,
            &output.file,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        )
    }

    #[test]
    fn parses_shuck_directives_and_strips_reasons() {
        let directives = directives("# shuck: disable=C006,S001 # legacy code\n");

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].action, SuppressionAction::Disable);
        assert_eq!(directives[0].source, SuppressionSource::Shuck);
        assert_eq!(
            directives[0].codes,
            vec![Rule::UndefinedVariable, Rule::UnquotedExpansion]
        );
        assert_eq!(directives[0].line, 1);
    }

    #[test]
    fn parses_dead_code_rule_aliases() {
        let directives = directives(
            "\
# shuck: disable=C124
# shuck: disable=SH-293
",
        );

        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].codes, vec![Rule::UnreachableAfterExit]);
        assert_eq!(directives[1].codes, vec![Rule::UnreachableAfterExit]);
    }

    #[test]
    fn parses_shellcheck_directives_on_own_line_and_after_code() {
        let source = "\
# shellcheck disable=SC2086 disable=SC2034 disable=SC7777
echo $foo
case $x in
  on) # shellcheck disable=SC2034
    value=1
    ;;
esac
";
        let directives = directives(source);

        assert_eq!(directives.len(), 2);
        assert_eq!(directives[0].action, SuppressionAction::Disable);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(
            directives[0].codes,
            vec![Rule::UnquotedExpansion, Rule::UnusedAssignment]
        );
        assert_eq!(directives[1].action, SuppressionAction::Disable);
        assert_eq!(directives[1].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[1].codes, vec![Rule::UnusedAssignment]);
    }

    #[test]
    fn rejects_shellcheck_directives_after_regular_code() {
        let source = "\
value=1 # shellcheck disable=SC2086
echo $foo
";
        let directives = directives(source);

        assert!(directives.is_empty());
    }

    #[test]
    fn rejects_case_label_directives_after_same_line_commands() {
        let source = "\
case $x in
  on) echo \"$x\" # shellcheck disable=SC2086
      ;;
esac
";
        let directives = directives(source);

        assert!(directives.is_empty());
    }

    #[test]
    fn rejects_shellcheck_directives_after_keyword_like_arguments() {
        let source = "\
echo if # shellcheck disable=SC2086
echo $foo
echo { # shellcheck disable=SC2086
echo $bar
";
        let directives = directives(source);

        assert!(directives.is_empty());
    }

    #[test]
    fn parses_shellcheck_directives_after_control_flow_headers_and_group_openers() {
        let source = "\
if # shellcheck disable=SC2086
  echo $foo
then # shellcheck disable=SC2086
  echo $foo
elif # shellcheck disable=SC2086
  echo $bar
then
  :
else # shellcheck disable=SC2086
  echo $baz
fi
while # shellcheck disable=SC2086
  echo $foo
do # shellcheck disable=SC2086
  echo $foo
done
until # shellcheck disable=SC2086
  echo $foo
do
  :
done
{ # shellcheck disable=SC2086
  echo $foo
}
( # shellcheck disable=SC2086
  echo $foo
)
";
        let directives = directives(source);

        assert_eq!(directives.len(), 9);
        assert!(directives.iter().all(|directive| {
            directive.source == SuppressionSource::ShellCheck
                && directive.codes == vec![Rule::UnquotedExpansion]
        }));
    }

    #[test]
    fn parses_shellcheck_directives_after_elif_then_header() {
        let source = "\
if false; then
  :
elif true; then # shellcheck disable=SC2086
  echo $foo
fi
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes, vec![Rule::UnquotedExpansion]);
        assert_eq!(directives[0].line, 3);
    }

    #[test]
    fn parses_shellcheck_directives_after_for_do_header() {
        let source = "\
for item in 1; do # shellcheck disable=SC2086
  echo $foo
done
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes, vec![Rule::UnquotedExpansion]);
        assert_eq!(directives[0].line, 1);
    }

    #[test]
    fn parses_shellcheck_directives_after_then_inline_group_openers() {
        let source = "\
if true; then { # shellcheck disable=SC2086
  echo $foo
}; fi
if true; then ( # shellcheck disable=SC2086
  echo $bar
); fi
";
        let directives = directives(source);

        assert_eq!(directives.len(), 2);
        assert!(directives.iter().all(|directive| {
            directive.source == SuppressionSource::ShellCheck
                && directive.codes == vec![Rule::UnquotedExpansion]
        }));
    }

    #[test]
    fn parses_shellcheck_disable_all() {
        let directives = directives("# shellcheck disable=all\n");

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes.len(), Rule::COUNT);
        assert!(directives[0].codes.contains(&Rule::CompoundTestOperator));
        assert!(directives[0].codes.contains(&Rule::UnquotedExpansion));
    }

    #[test]
    fn ignores_malformed_and_unknown_directives() {
        let source = "\
# shuck: disable=
# shuck: foobar=C001
# shuck disable=C001
# shuck: disable=SH-039
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].codes, vec![Rule::UndefinedVariable]);
    }

    #[test]
    fn parses_shellcheck_directives_inside_command_substitutions() {
        let source = "\
value=\"$(
  # shellcheck disable=SC2086
  echo $foo
)\"
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes, vec![Rule::UnquotedExpansion]);
        assert_eq!(directives[0].line, 2);
    }

    #[test]
    fn parses_case_label_directives_inside_command_substitution_arguments() {
        let source = "\
printf '%s\\n' \"$(
  case $x in
    on) # shellcheck disable=SC2086
      echo $foo
      ;;
  esac
)\"\n";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(directives[0].codes, vec![Rule::UnquotedExpansion]);
        assert_eq!(directives[0].line, 3);
    }
}

use shuck_ast::{TextRange, TextSize};
use shuck_indexer::{CommentIndex, IndexedComment};

use crate::{Rule, code_to_rule};

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
        } else if let Some(directive) = parse_shellcheck_directive(&comment, shellcheck_map) {
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
    comment: &NormalizedComment<'_>,
    shellcheck_map: &ShellCheckCodeMap,
) -> Option<SuppressionDirective> {
    if !comment.is_own_line {
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
                codes.extend(shellcheck_map.resolve_all(code));
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
    fn parses_own_line_shellcheck_directives_only() {
        let source = "\
# shellcheck disable=SC2086 disable=SC2034 disable=SC7777
echo $foo
value=1 # shellcheck disable=SC2034
";
        let directives = directives(source);

        assert_eq!(directives.len(), 1);
        assert_eq!(directives[0].action, SuppressionAction::Disable);
        assert_eq!(directives[0].source, SuppressionSource::ShellCheck);
        assert_eq!(
            directives[0].codes,
            vec![Rule::UnquotedExpansion, Rule::UnusedAssignment]
        );
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
}

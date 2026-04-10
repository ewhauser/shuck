use shuck_ast::{BraceQuoteContext, BraceSyntaxKind, Span, Word, WordPart, WordPartNode};

use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct LiteralBraces;

impl Violation for LiteralBraces {
    fn rule() -> Rule {
        Rule::LiteralBraces
    }

    fn message(&self) -> String {
        "literal braces may be interpreted as brace syntax".to_owned()
    }
}

pub fn literal_braces(checker: &mut Checker) {
    let source = checker.source();
    let facts = checker.facts();
    let mut spans = Vec::new();

    for fact in facts.word_facts() {
        if fact.expansion_context() == Some(ExpansionContext::RegexOperand) {
            continue;
        }

        spans.extend(
            fact.word()
                .brace_syntax()
                .iter()
                .copied()
                .filter(|brace| {
                    brace.kind == BraceSyntaxKind::Literal
                        && brace.quote_context == BraceQuoteContext::Unquoted
                })
                .filter(|brace| !is_find_exec_placeholder(facts, fact, brace.span, source))
                .flat_map(|brace| brace_literal_edge_spans(brace.span, source)),
        );

        spans.extend(escaped_parameter_expansion_brace_edge_spans(fact.word(), source));
    }

    checker.report_all_dedup(spans, || LiteralBraces);
}

fn is_find_exec_placeholder(
    facts: &crate::facts::LinterFacts<'_>,
    fact: &crate::facts::WordFact<'_>,
    brace_span: Span,
    source: &str,
) -> bool {
    if brace_span.slice(source) != "{}" {
        return false;
    }
    if fact.expansion_context() != Some(ExpansionContext::CommandArgument) {
        return false;
    }

    let command = facts.command(fact.command_id());
    if command
        .body_name_word()
        .is_some_and(|name_word| name_word.span == fact.span())
    {
        return false;
    }

    let has_exec_terminator = command
        .body_args()
        .iter()
        .any(|arg| matches!(arg.span.slice(source), "+" | "\\;"));
    if has_exec_terminator {
        return true;
    }

    let is_find = command.static_utility_name_is("find")
        || command
            .body_name_word()
            .is_some_and(|name_word| name_word.span.slice(source).ends_with("find"));
    let has_exec_flag = command.body_args().iter().any(|arg| {
        matches!(
            arg.span.slice(source),
            "-exec" | "-execdir" | "-ok" | "-okdir"
        )
    });

    is_find && has_exec_flag
}

fn brace_literal_edge_spans(span: Span, source: &str) -> Vec<Span> {
    let text = span.slice(source);
    let Some(open_offset) = text.find('{') else {
        return Vec::new();
    };
    let Some(close_offset) = text.rfind('}') else {
        return Vec::new();
    };

    let open = span.start.advanced_by(&text[..open_offset]);
    let close = span.start.advanced_by(&text[..close_offset]);
    vec![
        Span::from_positions(open, open),
        Span::from_positions(close, close),
    ]
}

#[derive(Debug, Clone, Copy)]
struct LiteralBraceCandidate {
    open_offset: usize,
    after_escaped_dollar: bool,
    has_runtime_shell_sigil_inside: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DynamicBraceExcludedSpanKind {
    Quoted,
    RuntimeShellSyntax,
}

#[derive(Debug, Clone, Copy)]
struct DynamicBraceExcludedSpan {
    start_offset: usize,
    end_offset: usize,
    kind: DynamicBraceExcludedSpanKind,
}

fn escaped_parameter_expansion_brace_edge_spans(word: &Word, source: &str) -> Vec<Span> {
    let span = word.span;
    let text = span.slice(source);
    let mut spans = Vec::new();
    let mut literal_stack: Vec<LiteralBraceCandidate> = Vec::new();
    let mut excluded = Vec::new();
    collect_dynamic_brace_exclusions(&word.parts, span.start.offset, source, &mut excluded);
    excluded.sort_by_key(|span| (span.start_offset, span.end_offset));
    let mut excluded_index = 0usize;
    let mut index = 0usize;
    let mut previous_char = None;
    let mut previous_char_escaped = false;

    while index < text.len() {
        while let Some(excluded_span) = excluded.get(excluded_index).copied() {
            if excluded_span.end_offset <= index {
                excluded_index += 1;
                continue;
            }

            if excluded_span.start_offset > index {
                break;
            }

            if excluded_span.kind == DynamicBraceExcludedSpanKind::RuntimeShellSyntax
                && let Some(current) = literal_stack.last_mut()
            {
                current.has_runtime_shell_sigil_inside = true;
            }
            if excluded_span.kind == DynamicBraceExcludedSpanKind::RuntimeShellSyntax
                && previous_char == Some('$')
                && previous_char_escaped
            {
                let excluded_text = &text[excluded_span.start_offset..excluded_span.end_offset];
                let open_offset = if excluded_text.starts_with("${") {
                    Some(excluded_span.start_offset + '$'.len_utf8())
                } else if excluded_text.starts_with('{') {
                    Some(excluded_span.start_offset)
                } else {
                    None
                };
                if let Some(open_offset) = open_offset
                    && excluded_text.ends_with('}')
                    && excluded_span.end_offset > open_offset + 1
                {
                    let open = span.start.advanced_by(&text[..open_offset]);
                    let close = span
                        .start
                        .advanced_by(&text[..excluded_span.end_offset - '}'.len_utf8()]);
                    spans.push(Span::from_positions(open, open));
                    spans.push(Span::from_positions(close, close));
                }
            }
            previous_char = None;
            previous_char_escaped = false;
            index = excluded_span.end_offset;
            excluded_index += 1;
        }

        if index >= text.len() {
            break;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                previous_char = Some(escaped);
                previous_char_escaped = true;
                index += escaped.len_utf8();
            } else {
                previous_char = Some('\\');
                previous_char_escaped = false;
            }
            continue;
        }

        if ch == '{' {
            literal_stack.push(LiteralBraceCandidate {
                open_offset: index,
                after_escaped_dollar: previous_char == Some('$') && previous_char_escaped,
                has_runtime_shell_sigil_inside: false,
            });
        } else if ch == '}' && let Some(candidate) = literal_stack.pop() {
            if index > candidate.open_offset + 1
                && (candidate.after_escaped_dollar || candidate.has_runtime_shell_sigil_inside)
            {
                let open = span.start.advanced_by(&text[..candidate.open_offset]);
                let close = span.start.advanced_by(&text[..index]);
                spans.push(Span::from_positions(open, open));
                spans.push(Span::from_positions(close, close));
            }
        }

        previous_char = Some(ch);
        previous_char_escaped = false;
        index += ch_len;
    }

    spans.extend(raw_escaped_parameter_brace_edge_spans(word, source));
    spans
}

fn collect_dynamic_brace_exclusions(
    parts: &[WordPartNode],
    word_base_offset: usize,
    source: &str,
    out: &mut Vec<DynamicBraceExcludedSpan>,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {}
            WordPart::DoubleQuoted { .. } if !part.span.slice(source).starts_with("\\\"") => {
                out.push(DynamicBraceExcludedSpan {
                    start_offset: part.span.start.offset - word_base_offset,
                    end_offset: part.span.end.offset - word_base_offset,
                    kind: DynamicBraceExcludedSpanKind::Quoted,
                });
            }
            WordPart::DoubleQuoted { .. } => {}
            WordPart::SingleQuoted { .. } => {
                out.push(DynamicBraceExcludedSpan {
                    start_offset: part.span.start.offset - word_base_offset,
                    end_offset: part.span.end.offset - word_base_offset,
                    kind: DynamicBraceExcludedSpanKind::Quoted,
                });
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => out.push(DynamicBraceExcludedSpan {
                start_offset: part.span.start.offset - word_base_offset,
                end_offset: part.span.end.offset - word_base_offset,
                kind: DynamicBraceExcludedSpanKind::RuntimeShellSyntax,
            }),
        }
    }
}

fn raw_escaped_parameter_brace_edge_spans(word: &Word, source: &str) -> Vec<Span> {
    let span = word.span;
    let text = span.slice(source);
    let mut excluded = Vec::new();
    collect_raw_escaped_parameter_exclusions(&word.parts, span.start.offset, source, &mut excluded);
    excluded.sort_by_key(|span| (span.start_offset, span.end_offset));

    let mut spans = Vec::new();
    let mut excluded_index = 0usize;
    let mut index = 0usize;
    let mut previous_char = None;
    let mut previous_char_escaped = false;
    let mut escaped_parameter_stack = Vec::new();
    let mut parameter_depth = 0usize;

    while index < text.len() {
        while let Some(excluded_span) = excluded.get(excluded_index).copied() {
            if excluded_span.end_offset <= index {
                excluded_index += 1;
                continue;
            }
            if excluded_span.start_offset > index {
                break;
            }

            previous_char = None;
            previous_char_escaped = false;
            index = excluded_span.end_offset;
            excluded_index += 1;
        }

        if index >= text.len() {
            break;
        }

        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        let ch_len = ch.len_utf8();

        if ch == '\\' {
            index += ch_len;
            if let Some(escaped) = text[index..].chars().next() {
                previous_char = Some(escaped);
                previous_char_escaped = true;
                index += escaped.len_utf8();
            } else {
                previous_char = Some('\\');
                previous_char_escaped = false;
            }
            continue;
        }

        if ch == '{' {
            if previous_char == Some('$') && previous_char_escaped {
                escaped_parameter_stack.push(index);
            } else if previous_char == Some('$') && !previous_char_escaped {
                parameter_depth += 1;
            }
        } else if ch == '}' {
            if parameter_depth > 0 {
                parameter_depth -= 1;
            } else if let Some(open_offset) = escaped_parameter_stack.pop() {
                let open = span.start.advanced_by(&text[..open_offset]);
                let close = span.start.advanced_by(&text[..index]);
                spans.push(Span::from_positions(open, open));
                spans.push(Span::from_positions(close, close));
            }
        }

        previous_char = Some(ch);
        previous_char_escaped = false;
        index += ch_len;
    }

    spans
}

fn collect_raw_escaped_parameter_exclusions(
    parts: &[WordPartNode],
    word_base_offset: usize,
    source: &str,
    out: &mut Vec<DynamicBraceExcludedSpan>,
) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_)
            | WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::Transformation { .. }
            | WordPart::ZshQualifiedGlob(_) => {}
            WordPart::DoubleQuoted { .. } if !part.span.slice(source).starts_with("\\\"") => {
                out.push(DynamicBraceExcludedSpan {
                    start_offset: part.span.start.offset - word_base_offset,
                    end_offset: part.span.end.offset - word_base_offset,
                    kind: DynamicBraceExcludedSpanKind::Quoted,
                });
            }
            WordPart::DoubleQuoted { .. } => {}
            WordPart::SingleQuoted { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. } => out.push(DynamicBraceExcludedSpan {
                start_offset: part.span.start.offset - word_base_offset,
                end_offset: part.span.end.offset - word_base_offset,
                kind: DynamicBraceExcludedSpanKind::Quoted,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_literal_unquoted_brace_pair_edges() {
        let source = "#!/bin/bash\necho HEAD@{1}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.column, 11);
        assert_eq!(diagnostics[1].span.start.column, 13);
    }

    #[test]
    fn ignores_quoted_and_expanding_braces() {
        let source = "#!/bin/bash\necho \"HEAD@{1}\" x{a,b}y\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_find_exec_placeholder_and_regex_quantifier() {
        let source = "\
#!/bin/bash
find . -exec echo {} \\;
if [[ \"$hash\" =~ ^[a-f0-9]{40}$ ]]; then
  :
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_escaped_dollar_literal_braces() {
        let source = "\
#!/bin/bash
eval command sudo \\\"\\${sudo_args[@]}\\\"
echo [0-9a-f]{$HASHLEN}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 4);
    }
}

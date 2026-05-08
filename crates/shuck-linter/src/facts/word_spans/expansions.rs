use super::*;
use crate::facts::words::{
    WordSubtreeVisitor, WordTraversalContext, WordTraversalState, walk_word_subtree,
};

pub fn expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_expansion_spans(word, traversal_context_without_source(), &mut spans);
    spans
}

#[allow(dead_code)]
pub fn collect_active_expansion_spans_in_source(
    word: &Word,
    locator: Locator<'_>,
    spans: &mut Vec<Span>,
) {
    collect_expansion_spans(
        word,
        WordTraversalContext {
            source: locator.source(),
            locator: Some(locator),
            shell_dialect: shuck_semantic::ShellDialect::Bash,
        },
        spans,
    );
    normalize_command_substitution_spans(spans, locator);
    spans.extend(
        word.brace_syntax()
            .iter()
            .copied()
            .filter(|brace| brace.expands())
            .map(|brace| brace.span),
    );
    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
}

#[allow(dead_code)]
pub fn collect_scalar_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_scalar_expansion_spans(word, false, spans);
}

#[allow(dead_code)]
pub fn collect_unquoted_scalar_expansion_part_spans(word: &Word, spans: &mut Vec<Span>) {
    collect_scalar_expansion_spans(word, true, spans);
}

pub fn word_double_quoted_scalar_only_expansion_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_double_quoted_scalar_only_expansion_spans(&word.parts, false, &mut spans)
        .then_some(spans)
        .filter(|spans| !spans.is_empty())
        .unwrap_or_default()
}

pub fn collect_word_literal_part_spans_excluding_parameter_operator_tails(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    spans.extend(
        word.parts
            .iter()
            .enumerate()
            .filter_map(|(index, part)| match &part.kind {
                WordPart::Literal(_)
                    if !literal_part_is_parameter_operator_tail(&word.parts, index, source) =>
                {
                    Some(part.span)
                }
                _ => None,
            }),
    );
}

pub fn word_has_single_literal_part(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [part] if matches!(part.kind, WordPart::Literal(_))
    )
}

pub fn collect_word_literal_scan_segments_excluding_expansions(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let mut excluded = Vec::new();
    collect_literal_scan_exclusions(&word.parts, &mut excluded);
    collect_scan_span_excluding(word.span, &excluded, source, spans);
}

pub fn word_unquoted_assign_default_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_unquoted_assign_default_spans(&word.parts, false, &mut spans);
    spans
}

pub fn word_use_replacement_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_use_replacement_spans(&word.parts, &mut spans);
    spans
}

pub(crate) fn collect_unquoted_assign_default_spans(
    parts: &[WordPartNode],
    quoted: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_unquoted_assign_default_spans(parts, true, spans);
            }
            _ if !quoted && part_uses_assign_default_operator(&part.kind) => spans.push(part.span),
            _ => {}
        }
    }
}

fn collect_expansion_spans<'a>(
    word: &'a Word,
    context: WordTraversalContext<'a>,
    spans: &mut Vec<Span>,
) {
    let mut visitor = ExpansionSpanVisitor { spans };
    walk_word_subtree(word, context, &mut visitor);
}

#[allow(dead_code)]
fn collect_scalar_expansion_spans(word: &Word, only_unquoted: bool, spans: &mut Vec<Span>) {
    let mut visitor = ScalarExpansionSpanVisitor {
        spans,
        only_unquoted,
    };
    walk_word_subtree(word, traversal_context_without_source(), &mut visitor);
}

struct ExpansionSpanVisitor<'spans> {
    spans: &'spans mut Vec<Span>,
}

impl<'a> WordSubtreeVisitor<'a> for ExpansionSpanVisitor<'_> {
    fn visit_part(&mut self, part: &'a WordPartNode, state: WordTraversalState<'a>) {
        if !state.processes_root_word() {
            return;
        }

        match &part.kind {
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::DoubleQuoted { .. } => {}
            WordPart::Variable(name) if matches!(name.as_str(), "@" | "*") => {
                self.spans.push(part.span);
            }
            WordPart::Variable(_)
            | WordPart::ZshQualifiedGlob(_)
            | WordPart::CommandSubstitution { .. }
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
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => self.spans.push(part.span),
        }
    }
}

#[allow(dead_code)]
struct ScalarExpansionSpanVisitor<'spans> {
    spans: &'spans mut Vec<Span>,
    only_unquoted: bool,
}

impl<'a> WordSubtreeVisitor<'a> for ScalarExpansionSpanVisitor<'_> {
    fn visit_part(&mut self, part: &'a WordPartNode, state: WordTraversalState<'a>) {
        if !state.processes_root_word() {
            return;
        }
        let quoted = state.in_double_quote;

        match &part.kind {
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::DoubleQuoted { .. } => {}
            WordPart::ZshQualifiedGlob(_) => {}
            WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {}
            WordPart::Parameter(parameter) => {
                if parameter_is_scalar_like(parameter) && (!self.only_unquoted || !quoted) {
                    self.spans.push(part.span);
                }
            }
            WordPart::Variable(name) if matches!(name.as_str(), "@" | "*") => {}
            WordPart::Variable(_)
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayLength(_)
            | WordPart::Substring { .. }
            | WordPart::PrefixMatch { .. } => {
                if !self.only_unquoted || !quoted {
                    self.spans.push(part.span);
                }
            }
            WordPart::ParameterExpansion { reference, .. }
            | WordPart::IndirectExpansion { reference, .. }
            | WordPart::Transformation { reference, .. } => {
                if !reference.has_array_selector() && (!self.only_unquoted || !quoted) {
                    self.spans.push(part.span);
                }
            }
            WordPart::ArrayAccess(reference) => {
                if !reference.has_array_selector() && (!self.only_unquoted || !quoted) {
                    self.spans.push(part.span);
                }
            }
            WordPart::ArrayIndices(_) | WordPart::ArraySlice { .. } => {}
        }
    }
}

fn traversal_context_without_source<'a>() -> WordTraversalContext<'a> {
    WordTraversalContext {
        source: "",
        locator: None,
        shell_dialect: shuck_semantic::ShellDialect::Bash,
    }
}

pub(crate) fn collect_use_replacement_spans(parts: &[WordPartNode], spans: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => collect_use_replacement_spans(parts, spans),
            WordPart::Parameter(parameter) if parameter_uses_replacement_operator(parameter) => {
                spans.push(part.span);
            }
            WordPart::ParameterExpansion { operator, .. }
            | WordPart::IndirectExpansion {
                operator: Some(operator),
                ..
            } if matches!(operator.as_ref(), ParameterOp::UseReplacement) => spans.push(part.span),
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::ZshQualifiedGlob(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::Length(_)
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::Substring { .. }
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::Transformation { .. } => {}
            WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::IndirectExpansion { .. } => {}
        }
    }
}

pub(crate) fn collect_double_quoted_scalar_affix_state(
    parts: &[WordPartNode],
    saw_literal: &mut bool,
    saw_scalar_expansion: &mut bool,
    literal_span: &mut Option<Span>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {
                *saw_literal = true;
                if literal_span.is_none() {
                    *literal_span = Some(part.span);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_double_quoted_scalar_affix_state(
                    parts,
                    saw_literal,
                    saw_scalar_expansion,
                    literal_span,
                ) {
                    return false;
                }
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::Transformation { .. } => {
                *saw_scalar_expansion = true;
            }
            WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                return false;
            }
        }
    }

    true
}

pub(crate) fn collect_double_quoted_scalar_only_expansion_spans(
    parts: &[WordPartNode],
    inside_double_quotes: bool,
    spans: &mut Vec<Span>,
) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_double_quoted_scalar_only_expansion_spans(parts, true, spans) {
                    return false;
                }
            }
            WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::Length(_)
            | WordPart::Substring { .. }
            | WordPart::IndirectExpansion { .. }
            | WordPart::Transformation { .. } => {
                if !inside_double_quotes {
                    return false;
                }
                spans.push(part.span);
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::ArrayAccess(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::ArraySlice { .. }
            | WordPart::PrefixMatch { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ZshQualifiedGlob(_) => {
                return false;
            }
        }
    }

    true
}

pub(crate) fn literal_part_is_parameter_operator_tail(
    parts: &[WordPartNode],
    index: usize,
    source: &str,
) -> bool {
    let Some(previous) = index.checked_sub(1).and_then(|index| parts.get(index)) else {
        return false;
    };
    if !matches!(
        previous.kind,
        WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::IndirectExpansion { .. }
    ) {
        return false;
    }

    let text = parts[index].span.slice(source);
    text.ends_with('}') && (text.starts_with('/') || text.starts_with('%') || text.starts_with('#'))
}

pub(crate) fn collect_literal_scan_exclusions(parts: &[WordPartNode], excluded: &mut Vec<Span>) {
    for part in parts {
        match &part.kind {
            WordPart::Literal(_) => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_literal_scan_exclusions(parts, excluded);
            }
            WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::SingleQuoted { .. }
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
            | WordPart::ZshQualifiedGlob(_) => excluded.push(part.span),
        }
    }
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Span, Word};
    use shuck_parser::parser::Parser;

    use super::{
        collect_scalar_expansion_part_spans, word_double_quoted_scalar_only_expansion_spans,
        word_unquoted_assign_default_spans,
    };

    fn scalar_expansion_part_spans(word: &Word, _source: &str) -> Vec<Span> {
        let mut spans = Vec::new();
        collect_scalar_expansion_part_spans(word, &mut spans);
        spans
    }

    #[test]
    fn scalar_expansion_spans_ignore_array_splats_and_command_substitutions() {
        let source = "printf '%s\\n' prefix${name}suffix ${arr[@]} ${arr[0]} ${arr[@]:-fallback} ${arr[*]:-fallback} ${arr[@]@Q} ${arr[*]@Q} ${arr[0]:-fallback} $(date)\n";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            scalar_expansion_part_spans(&command.args[1], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${name}"]
        );
        assert!(
            scalar_expansion_part_spans(&command.args[2], source).is_empty(),
            "array splats should be left to S008"
        );
        assert_eq!(
            scalar_expansion_part_spans(&command.args[3], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[0]}"]
        );
        assert!(
            scalar_expansion_part_spans(&command.args[4], source).is_empty(),
            "array splats with default operators should be left to array rules"
        );
        assert!(
            scalar_expansion_part_spans(&command.args[5], source).is_empty(),
            "star-selector array splats with default operators should be left to array rules"
        );
        assert!(
            scalar_expansion_part_spans(&command.args[6], source).is_empty(),
            "array splat transformations should be left to array rules"
        );
        assert!(
            scalar_expansion_part_spans(&command.args[7], source).is_empty(),
            "star-splat transformations should stay on the star-parameter path"
        );
        assert_eq!(
            scalar_expansion_part_spans(&command.args[8], source)
                .iter()
                .map(|span| span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[0]:-fallback}"]
        );
        assert!(
            scalar_expansion_part_spans(&command.args[9], source).is_empty(),
            "command substitutions should be left to S004"
        );
    }

    #[test]
    fn word_unquoted_assign_default_spans_track_only_unquoted_assignment_defaults() {
        let source = "\
printf '%s\\n' ${x=} ${x:=a} ${x:-a} \"${x=}\" \"${x:=a}\" prefix${x=}suffix ${!name:=fallback} ${name/pat/repl}
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_unquoted_assign_default_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(
            spans,
            vec!["${x=}", "${x:=a}", "${x=}", "${!name:=fallback}"]
        );
    }

    #[test]
    fn word_double_quoted_scalar_only_expansion_spans_ignore_literal_affixes() {
        let source = "\
printf '%s\\n' \"$a\" \"$a\"\"$b\" \"prefix$a\" \"$a$(printf '%s' x)\" $a \"$a\"/\"$b\"
";
        let output = Parser::new(source).parse().unwrap();
        let command = &output.file.body[0].command;
        let shuck_ast::Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        let spans = command
            .args
            .iter()
            .flat_map(word_double_quoted_scalar_only_expansion_spans)
            .map(|span| span.slice(source))
            .collect::<Vec<_>>();

        assert_eq!(spans, vec!["$a", "$a", "$b"]);
    }
}

use shuck_ast::{Position, Span};

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, ExpansionContext, Rule,
    SimpleTestOperatorFamily, SimpleTestShape, Violation, WordFactContext, WordQuote,
};

pub struct TildeInStringComparison;

impl Violation for TildeInStringComparison {
    fn rule() -> Rule {
        Rule::TildeInStringComparison
    }

    fn message(&self) -> String {
        "quoted `~/...` stays literal in string comparisons; use `$HOME` or an unquoted tilde"
            .to_owned()
    }
}

pub fn tilde_in_string_comparison(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|command| {
            let mut spans = Vec::new();
            if let Some(simple_test) = command.simple_test() {
                spans.extend(collect_simple_test_spans(checker, simple_test, source));
            }
            if let Some(conditional) = command.conditional() {
                spans.extend(collect_conditional_spans(conditional, source));
            }
            spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TildeInStringComparison);
}

fn collect_simple_test_spans(
    checker: &Checker<'_>,
    simple_test: &crate::SimpleTestFact<'_>,
    source: &str,
) -> Vec<Span> {
    if simple_test.effective_shape() != SimpleTestShape::Binary
        || simple_test.effective_operator_family() != SimpleTestOperatorFamily::StringBinary
    {
        return Vec::new();
    }

    [0usize, 2usize]
        .into_iter()
        .filter_map(|index| {
            let word = *simple_test.effective_operands().get(index)?;
            let fact = checker.facts().word_fact(
                word.span,
                WordFactContext::Expansion(ExpansionContext::CommandArgument),
            )?;
            word_fact_tilde_span(fact, source)
        })
        .collect()
}

fn collect_conditional_spans(conditional: &crate::ConditionalFact<'_>, source: &str) -> Vec<Span> {
    conditional
        .nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() == ConditionalOperatorFamily::StringBinary =>
            {
                Some([binary.left(), binary.right()])
            }
            _ => None,
        })
        .flatten()
        .filter_map(|operand| conditional_operand_tilde_span(operand, source))
        .collect()
}

fn word_fact_tilde_span(fact: crate::WordOccurrenceRef<'_, '_>, source: &str) -> Option<Span> {
    let classification = fact.classification();
    (classification.quote != WordQuote::Unquoted && classification.is_fixed_literal())
        .then(|| {
            fact.static_text()
                .filter(|text| text.starts_with("~/"))
                .and_then(|_| quoted_tilde_span(fact.span(), source))
        })
        .flatten()
}

fn conditional_operand_tilde_span(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> Option<Span> {
    if let Some(word) = operand.word()
        && let Some(classification) = operand.word_classification()
    {
        if classification.quote == WordQuote::Unquoted || !classification.is_fixed_literal() {
            return None;
        }

        return crate::static_word_text(word, source)
            .filter(|text| text.starts_with("~/"))
            .and_then(|_| quoted_tilde_span(word.span, source));
    }

    operand
        .class()
        .is_fixed_literal()
        .then(|| operand.expression().span())
        .and_then(|span| {
            let raw = span.slice(source);
            raw.chars()
                .next()
                .filter(|quote| matches!(quote, '"' | '\''))
                .and_then(|_| raw.get(1..))
                .filter(|suffix| suffix.starts_with("~/"))
                .and_then(|_| quoted_tilde_span_from_raw(span, raw))
        })
}

fn quoted_tilde_span(span: Span, source: &str) -> Option<Span> {
    let raw = span.slice(source);
    quoted_tilde_span_from_raw(span, raw)
}

fn quoted_tilde_span_from_raw(span: Span, raw: &str) -> Option<Span> {
    let start_index = raw.find("~/")?;
    let suffix = &raw[start_index..];
    let end_index = start_index + suffix.find(['"', '\'']).unwrap_or(suffix.len());
    let start = advance_position(span.start, &raw[..start_index]);
    let end = advance_position(start, &raw[start_index..end_index]);
    Some(Span::from_positions(start, end))
}

fn advance_position(mut position: Position, text: &str) -> Position {
    position = position.advanced_by(text);
    position
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_home_relative_paths_in_string_comparisons() {
        let source = "\
#!/bin/bash
[ \"$profile\" = \"~/.bashrc\" ]
[ \"~/.bashrc\" = \"$profile\" ]
[[ \"$profile\" == \"~/.profile\" ]]
[ \"$profile\" != '~/.zshrc' ]
[ ! = \"~/.bashrc\" ]
[ ! \"$profile\" = \"~/.bashrc\" ]
[ ! \"~/.bashrc\" = \"$profile\" ]
[ ! \"$profile\" != '~/.zshrc' ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TildeInStringComparison),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "~/.bashrc",
                "~/.bashrc",
                "~/.profile",
                "~/.zshrc",
                "~/.bashrc",
                "~/.bashrc",
                "~/.bashrc",
                "~/.zshrc",
            ]
        );
    }

    #[test]
    fn ignores_unquoted_tilde_and_non_home_tilde_literals() {
        let source = "\
#!/bin/bash
[ \"$profile\" = ~/.bashrc ]
[ \"$profile\" = \"~user/.bashrc\" ]
[ \"$profile\" = \"~\" ]
[ \"$profile\" = \"foo~/.bashrc\" ]
[[ \"$profile\" == a~/.bashrc ]]
[ ! \"$profile\" = ~/.bashrc ]
[ ! \"$profile\" = \"~user/.bashrc\" ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TildeInStringComparison),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_quoted_tilde_literals_outside_string_comparisons() {
        let source = "\
#!/bin/bash
profile='~/.bash_profile'
VAGRANT_HOME=\"~/.vagrant.d\"
[ -e '~/.bash_profile' ]
printf '%s\n' \"~/.config/powershell/profile.ps1\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TildeInStringComparison),
        );

        assert!(diagnostics.is_empty());
    }
}

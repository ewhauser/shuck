use shuck_ast::{Span, Word, WordPart, WordPartNode};

use crate::rules::common::{
    expansion::ExpansionContext,
    query::{self, CommandWalkOptions},
};
use crate::{Checker, Rule, Violation};

pub struct TrapStringExpansion;

impl Violation for TrapStringExpansion {
    fn rule() -> Rule {
        Rule::TrapStringExpansion
    }

    fn message(&self) -> String {
        "double-quoted trap handlers expand variables when the trap is set".to_owned()
    }
}

pub fn trap_string_expansion(checker: &mut Checker) {
    let source = checker.source();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            query::visit_expansion_words(command, source, &mut |word, context| {
                if context != ExpansionContext::TrapAction {
                    return;
                }

                for span in double_quoted_expansion_part_spans(word) {
                    checker.report_dedup(TrapStringExpansion, span);
                }
            });
        },
    );
}

fn double_quoted_expansion_part_spans(word: &Word) -> Vec<Span> {
    let mut spans = Vec::new();
    collect_double_quoted_expansion_spans(&word.parts, false, &mut spans);
    spans
}

fn collect_double_quoted_expansion_spans(
    parts: &[WordPartNode],
    inside_double_quotes: bool,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                collect_double_quoted_expansion_spans(parts, true, spans);
            }
            WordPart::Variable(_)
            | WordPart::CommandSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
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
            | WordPart::Transformation { .. }
                if inside_double_quotes =>
            {
                spans.push(part.span)
            }
            WordPart::Literal(_) => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_each_expansion_inside_the_trap_action() {
        let source = "trap \"echo $x $(date) ${y}\" EXIT\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::TrapStringExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x", "$(date)", "${y}"]
        );
    }

    #[test]
    fn ignores_trap_listing_modes() {
        let source = "trap -p EXIT\ntrap -l TERM\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::TrapStringExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_expansions_inside_mixed_quoted_trap_words() {
        let source = "trap foo\"$x\"bar\"$(date)\" EXIT\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::TrapStringExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x", "$(date)"]
        );
    }
}

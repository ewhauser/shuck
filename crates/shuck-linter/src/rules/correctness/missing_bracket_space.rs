use shuck_ast::Span;
use std::collections::HashSet;

use crate::{Checker, CommandFact, Rule, Violation};

pub struct MissingBracketSpace;

impl Violation for MissingBracketSpace {
    fn rule() -> Rule {
        Rule::MissingBracketSpace
    }

    fn message(&self) -> String {
        "this unary `[` test operator is missing its operand before the closing `]`".to_owned()
    }
}

pub fn missing_bracket_space(checker: &mut Checker) {
    let source = checker.source();
    let mut seen_lines = HashSet::new();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| missing_bracket_space_span(fact, source))
        .filter(|span| seen_lines.insert(span.start.line))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MissingBracketSpace);
}

fn missing_bracket_space_span(fact: &CommandFact<'_>, source: &str) -> Option<Span> {
    if !fact.static_utility_name_is("[") {
        return None;
    }

    let args = fact.body_args();
    let last = args.last()?;
    let text = last.span.slice(source);
    if text == "]" || !text.ends_with(']') || text.ends_with("\\]") {
        return None;
    }

    unary_glued_test(args, source)
}

fn unary_glued_test(args: &[&shuck_ast::Word], source: &str) -> Option<Span> {
    let [first, second] = args else {
        let [bang, operator, operand] = args else {
            return None;
        };
        return (bang.span.slice(source) == "!"
            && is_unary_test_operator(operator.span.slice(source))
            && operand
                .span
                .slice(source)
                .strip_suffix(']')
                .is_some_and(|prefix| !prefix.is_empty()))
        .then_some(Span::from_positions(operand.span.start, operand.span.start));
    };

    (is_unary_test_operator(first.span.slice(source))
        && second
            .span
            .slice(source)
            .strip_suffix(']')
            .is_some_and(|prefix| !prefix.is_empty()))
    .then_some(Span::from_positions(second.span.start, second.span.start))
}

fn is_unary_test_operator(text: &str) -> bool {
    matches!(
        text,
        "-n"
            | "-z"
            | "-b"
            | "-c"
            | "-d"
            | "-e"
            | "-f"
            | "-g"
            | "-h"
            | "-k"
            | "-p"
            | "-r"
            | "-s"
            | "-t"
            | "-u"
            | "-v"
            | "-w"
            | "-x"
            | "-L"
            | "-O"
            | "-G"
            | "-S"
            | "-N"
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_bracket_tests_with_a_glued_closing_bracket() {
        let source = "\
#!/bin/sh
if [ -d /tmp]; then
  :
fi
if [ \"$dir\" = /tmp]; then
  :
fi
if [ -n \"$dir\"]; then
  :
fi
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::MissingBracketSpace));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![(2, 9), (8, 9)]
        );
    }

    #[test]
    fn keeps_only_the_first_unary_match_per_line() {
        let source = "\
#!/bin/sh
if [ ! -d \"$a\"] || [ ! -d \"$b\"]; then
  :
fi
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::MissingBracketSpace));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            (diagnostics[0].span.start.line, diagnostics[0].span.start.column),
            (2, 11)
        );
    }

    #[test]
    fn ignores_well_spaced_or_differently_malformed_tests() {
        let source = "\
#!/bin/sh
if [ -d /tmp ]; then
  :
fi
if [ x = \"]\"; then
  :
fi
echo /tmp]
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::MissingBracketSpace));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

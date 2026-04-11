use shuck_ast::{RedirectKind, Span};

use crate::{Checker, CommandFact, RedirectFact, Rule, SimpleTestSyntax, Violation};

pub struct GreaterThanInTest;

impl Violation for GreaterThanInTest {
    fn rule() -> Rule {
        Rule::GreaterThanInTest
    }

    fn message(&self) -> String {
        "`>` inside `[` redirects output instead of comparing values".to_owned()
    }
}

pub fn greater_than_in_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| comparison_redirect_span(command, checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GreaterThanInTest);
}

fn comparison_redirect_span(command: &CommandFact<'_>, source: &str) -> Option<Span> {
    let simple_test = command.simple_test()?;
    if simple_test.syntax() != SimpleTestSyntax::Bracket {
        return None;
    }

    let opening_bracket = command.body_word_span()?;
    let closing_bracket = command.body_args().last()?;

    command.redirect_facts().iter().find_map(|redirect| {
        internal_plain_output_redirect_span(redirect, opening_bracket, closing_bracket.span, source)
    })
}

fn internal_plain_output_redirect_span(
    redirect: &RedirectFact<'_>,
    opening_bracket_span: Span,
    closing_bracket_span: Span,
    source: &str,
) -> Option<Span> {
    let redirect_data = redirect.redirect();
    if redirect_data.kind != RedirectKind::Output {
        return None;
    }

    let target = redirect_data.word_target()?;
    if redirect_data.span.start.offset < opening_bracket_span.end.offset
        || redirect_data.span.start.offset >= closing_bracket_span.start.offset
    {
        return None;
    }

    let operator_text = source
        .get(redirect_data.span.start.offset..target.span.start.offset)?
        .trim_end();
    if operator_text != ">" {
        return None;
    }

    Some(Span::from_positions(
        redirect_data.span.start,
        redirect_data.span.start.advanced_by(">"),
    ))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_plain_greater_than_redirects_inside_bracket_tests() {
        let source = "\
#!/bin/bash
[ \"$version\" > \"10\" ]
[ 1 > 2 ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GreaterThanInTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![">", ">"]
        );
    }

    #[test]
    fn ignores_redirects_after_the_test_and_non_operator_literals() {
        let source = "\
#!/bin/bash
>\"$log\" [ \"$value\" ]
[ \"$value\" ] > \"$log\"
[ \"$value\" \\> \"$other\" ]
[ \"$value\" \">\" \"$other\" ]
test \"$value\" > \"$other\"
[[ \"$value\" > \"$other\" ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GreaterThanInTest));

        assert!(diagnostics.is_empty());
    }
}

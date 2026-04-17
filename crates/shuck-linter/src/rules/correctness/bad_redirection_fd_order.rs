use shuck_ast::{RedirectKind, Span};

use crate::{Checker, RedirectFact, Rule, Violation};

pub struct BadRedirectionFdOrder;

impl Violation for BadRedirectionFdOrder {
    fn rule() -> Rule {
        Rule::BadRedirectionFdOrder
    }

    fn message(&self) -> String {
        "use a standard descriptor redirection form like `2>&1`".to_owned()
    }
}

pub fn bad_redirection_fd_order(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            fact.redirect_facts()
                .iter()
                .filter_map(|redirect| malformed_numeric_target_span(redirect, checker.source()))
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BadRedirectionFdOrder);
}

fn malformed_numeric_target_span(redirect: &RedirectFact<'_>, source: &str) -> Option<Span> {
    let target_span = redirect.target_span()?;
    let target_text = target_span.slice(source);
    if target_text.is_empty() || !target_text.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    let redirect_data = redirect.redirect();
    if !matches!(
        redirect_data.kind,
        RedirectKind::Output
            | RedirectKind::Clobber
            | RedirectKind::Append
            | RedirectKind::OutputBoth
    ) {
        return None;
    }

    let redirect_text = redirect_data.span.slice(source);
    let operator_offset = redirect_text.find('>')?;
    let start = redirect_data
        .span
        .start
        .advanced_by(&redirect_text[..operator_offset]);
    Some(Span::from_positions(start, target_span.end))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_malformed_numeric_redirect_targets() {
        let source = "\
#!/bin/sh
echo ok >2
echo ok 2>1
echo ok >>2
echo ok 2>>1
echo ok 2&>1
echo ok 2&>>1
echo ok &>1
echo ok &>>1
echo ok 2 &>1
echo ok &>01
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BadRedirectionFdOrder),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        );
    }

    #[test]
    fn ignores_standard_or_non_numeric_forms() {
        let source = "\
#!/bin/sh
echo ok 2>&1
echo ok >&1
echo ok &>file
echo ok 2&>file
echo ok &>\"1\"
echo ok &>-1
echo ok &>+1
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BadRedirectionFdOrder),
        );

        assert!(diagnostics.is_empty());
    }
}

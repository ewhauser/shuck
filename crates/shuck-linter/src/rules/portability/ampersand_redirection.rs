use shuck_ast::{RedirectKind, Span};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct AmpersandRedirection;

impl Violation for AmpersandRedirection {
    fn rule() -> Rule {
        Rule::AmpersandRedirection
    }

    fn message(&self) -> String {
        "use of `&>` is not portable in `sh`".to_owned()
    }
}

pub fn ampersand_redirection(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirect_facts().iter())
        .filter_map(|redirect| {
            (redirect.redirect().kind == RedirectKind::OutputBoth).then(|| {
                let span = redirect.redirect().span;
                Span::from_positions(span.start, span.start.advanced_by("&>"))
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AmpersandRedirection);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_ampersand_redirection_in_sh() {
        let source = "\
#!/bin/sh
: &>out
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirection),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "&>");
    }

    #[test]
    fn ignores_ampersand_redirection_in_bash() {
        let source = "\
#!/bin/bash
: &>out
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AmpersandRedirection),
        );

        assert!(diagnostics.is_empty());
    }
}

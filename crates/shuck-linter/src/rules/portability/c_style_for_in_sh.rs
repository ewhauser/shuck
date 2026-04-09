use shuck_ast::{Command, CompoundCommand, Span};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct CStyleForInSh;

impl Violation for CStyleForInSh {
    fn rule() -> Rule {
        Rule::CStyleForInSh
    }

    fn message(&self) -> String {
        "C-style `for ((...))` loops are not portable in `sh` scripts".to_owned()
    }
}

pub fn c_style_for_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| match fact.command() {
            Command::Compound(CompoundCommand::ArithmeticFor(_)) => {
                Some(keyword_span(fact.span_in_source(checker.source()), "for"))
            }
            _ => None,
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || CStyleForInSh);
}

fn keyword_span(span: Span, keyword: &str) -> Span {
    Span::from_positions(span.start, span.start.advanced_by(keyword))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_for_keyword_only() {
        let source = "#!/bin/sh\nfor ((i = 0; i < 5; i++)); do echo \"$i\"; done\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CStyleForInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "for");
    }

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\nfor ((i = 0; i < 5; i++)); do echo \"$i\"; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}

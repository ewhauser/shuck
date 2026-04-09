use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct SelectLoop;

impl Violation for SelectLoop {
    fn rule() -> Rule {
        Rule::SelectLoop
    }

    fn message(&self) -> String {
        "`select` loops are not portable in `sh` scripts".to_owned()
    }
}

pub fn select_loop(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .select_headers()
        .iter()
        .map(|header| keyword_span(header.span(), "select"))
        .collect::<Vec<_>>();

    checker.report_all(spans, || SelectLoop);
}

fn keyword_span(span: Span, keyword: &str) -> Span {
    Span::from_positions(span.start, span.start.advanced_by(keyword))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_select_keyword_only() {
        let source = "#!/bin/sh\nselect item in a b; do echo \"$item\"; break; done\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SelectLoop));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "select");
    }

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\nselect item in a b; do break; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SelectLoop).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}

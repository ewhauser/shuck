use shuck_ast::{RedirectKind, Span};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct HereString;

impl Violation for HereString {
    fn rule() -> Rule {
        Rule::HereString
    }

    fn message(&self) -> String {
        "here-strings are not portable in `sh`".to_owned()
    }
}

pub fn here_string(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirect_facts().iter())
        .filter(|redirect| redirect.kind() == RedirectKind::HereString)
        .map(|redirect| {
            let span = redirect.span();
            Span::from_positions(span.start, span.start.advanced_by("<<<"))
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || HereString);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_here_string_operator() {
        let source = "\
#!/bin/sh
cat <<< hi
printf '%s\n' \"$(
  wc -c <<< \"$value\"
)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::HereString));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["<<<", "<<<"]
        );
    }

    #[test]
    fn ignores_here_strings_in_bash() {
        let source = "cat <<< hi\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::HereString).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}

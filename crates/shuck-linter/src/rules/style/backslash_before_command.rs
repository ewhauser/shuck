use shuck_ast::{Command, Span};

use crate::{Checker, Rule, Violation};

pub struct BackslashBeforeCommand;

impl Violation for BackslashBeforeCommand {
    fn rule() -> Rule {
        Rule::BackslashBeforeCommand
    }

    fn message(&self) -> String {
        "a leading backslash before command is unnecessary".to_owned()
    }
}

pub fn backslash_before_command(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| backslash_before_command_span(fact.command(), source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BackslashBeforeCommand);
}

fn backslash_before_command_span(command: &Command, source: &str) -> Option<Span> {
    let Command::Simple(simple) = command else {
        return None;
    };

    (simple.name.span.slice(source) == "\\command").then_some(Span::from_positions(
        simple.name.span.start,
        simple.name.span.start,
    ))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_backslash_before_command() {
        let source = "\
#!/bin/bash
\\command echo hi
\\command \\rm tmp.txt || echo fail
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BackslashBeforeCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["", ""]
        );
    }
}

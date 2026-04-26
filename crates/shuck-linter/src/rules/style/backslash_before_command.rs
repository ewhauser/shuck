use shuck_ast::Span;

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
        .filter_map(|fact| {
            let word = fact.arena_body_name_word(source)?;
            backslash_before_command_span(word.span(), source)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BackslashBeforeCommand);
}

fn backslash_before_command_span(name_span: Span, source: &str) -> Option<Span> {
    (name_span.slice(source) == "\\command").then_some(Span::from_positions(
        name_span.start,
        name_span.start,
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

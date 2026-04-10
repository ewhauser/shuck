use crate::{Checker, Rule, Violation};

pub struct RedundantSpacesInEcho;

impl Violation for RedundantSpacesInEcho {
    fn rule() -> Rule {
        Rule::RedundantSpacesInEcho
    }

    fn message(&self) -> String {
        "quote repeated spaces to avoid them collapsing into one".to_owned()
    }
}

pub fn redundant_spaces_in_echo(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("echo"))
        .filter(|fact| fact.wrappers().is_empty())
        .filter_map(|fact| {
            if has_repeated_argument_spaces(fact.body_args(), source) {
                fact.body_name_word()
                    .and_then(|name| command_span(name, fact.body_args()))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || RedundantSpacesInEcho);
}

fn has_repeated_argument_spaces(words: &[&shuck_ast::Word], source: &str) -> bool {
    words.windows(2).any(|pair| repeated_space_gap(pair[0].span, pair[1].span, source))
}

fn repeated_space_gap(left: shuck_ast::Span, right: shuck_ast::Span, source: &str) -> bool {
    if left.end.line != right.start.line {
        return false;
    }

    let Some(gap) = source.get(left.end.offset..right.start.offset) else {
        return false;
    };

    gap.len() >= 4 && gap.chars().all(|ch| ch == ' ')
}

fn command_span(name: &shuck_ast::Word, args: &[&shuck_ast::Word]) -> Option<shuck_ast::Span> {
    let last = args.last()?;
    Some(shuck_ast::Span::from_positions(name.span.start, last.span.end))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_repeated_spaces_between_echo_arguments() {
        let source = "\
#!/bin/bash
echo foo    bar
echo -n    \"foo\"
echo \"foo\"    bar
echo foo    \"bar\"
echo foo  bar
echo    foo
command echo foo    bar
builtin echo foo    bar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo foo    bar", "echo -n    \"foo\"", "echo \"foo\"    bar", "echo foo    \"bar\""]
        );
    }

    #[test]
    fn ignores_single_argument_and_wrapped_echoes() {
        let source = "\
#!/bin/sh
echo    foo
command echo foo    bar
builtin echo foo    bar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho).with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty());
    }
}

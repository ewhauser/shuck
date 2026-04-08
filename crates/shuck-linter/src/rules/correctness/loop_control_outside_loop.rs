use shuck_ast::{BuiltinCommand, Command, Span};

use crate::{Checker, Rule, Violation};

pub struct LoopControlOutsideLoop {
    pub keyword: &'static str,
}

impl Violation for LoopControlOutsideLoop {
    fn rule() -> Rule {
        Rule::LoopControlOutsideLoop
    }

    fn message(&self) -> String {
        format!("`{}` is only valid inside a loop", self.keyword)
    }
}

pub fn loop_control_outside_loop(checker: &mut Checker) {
    let violations = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| match fact.command() {
            Command::Builtin(BuiltinCommand::Break(command)) => {
                Some((command.span, keyword_span(command.span, "break"), "break"))
            }
            Command::Builtin(BuiltinCommand::Continue(command)) => {
                Some((command.span, keyword_span(command.span, "continue"), "continue"))
            }
            _ => None,
        })
        .filter(|(command_span, _, _)| {
            checker
                .semantic()
                .flow_context_at(command_span)
                .map(|context| context.loop_depth == 0)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    for (_, report_span, keyword) in violations {
        checker.report(LoopControlOutsideLoop { keyword }, report_span);
    }
}

fn keyword_span(span: Span, keyword: &str) -> Span {
    Span::from_positions(span.start, span.start.advanced_by(keyword))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_loop_control_keyword_only() {
        let source = "\
#!/bin/sh
termux_step_make() {
\tcontinue 2
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LoopControlOutsideLoop));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["continue"]
        );
    }

    #[test]
    fn ignores_loop_control_inside_a_loop() {
        let source = "\
#!/bin/sh
while true; do
\tbreak
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LoopControlOutsideLoop));

        assert!(diagnostics.is_empty());
    }
}

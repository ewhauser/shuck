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
    let violations = loop_control_violations(checker, false, false);

    for (_, report_span, keyword) in violations {
        checker.report(LoopControlOutsideLoop { keyword }, report_span);
    }
}

pub(crate) fn loop_control_violations(
    checker: &Checker<'_>,
    inside_function_only: bool,
    continue_only: bool,
) -> Vec<(Span, Span, &'static str)> {
    checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| match fact.command() {
            Command::Builtin(BuiltinCommand::Break(command)) if !continue_only => {
                Some((command.span, keyword_span(command.span, "break"), "break"))
            }
            Command::Builtin(BuiltinCommand::Continue(command)) => Some((
                command.span,
                keyword_span(command.span, "continue"),
                "continue",
            )),
            _ => None,
        })
        .filter(|(command_span, _, keyword)| {
            let inside_function = checker
                .semantic_analysis()
                .enclosing_function_scope_at(command_span.start.offset)
                .is_some();
            let in_subshell = checker
                .semantic()
                .flow_context_at(command_span)
                .map(|context| context.in_subshell)
                .unwrap_or(false);
            if inside_function_only && !inside_function {
                return false;
            }
            if inside_function_only && in_subshell {
                return false;
            }
            if !inside_function_only && inside_function && *keyword == "continue" && !in_subshell {
                return false;
            }
            checker
                .semantic()
                .flow_context_at(command_span)
                .map(|context| context.loop_depth == 0)
                .unwrap_or(true)
        })
        .collect()
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
continue 2
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LoopControlOutsideLoop),
        );

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
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LoopControlOutsideLoop),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_loop_control_inside_function_loop_brace_group() {
        let source = "\
#!/bin/bash
f() {
  for op in a; do
    if [[ \"$op\" == a ]]; then { echo ok; break; }; fi
  done
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LoopControlOutsideLoop),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_continue_inside_functions() {
        let source = "\
#!/bin/sh
f() {
\tcontinue
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LoopControlOutsideLoop),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_continue_inside_function_subshells() {
        let source = "\
#!/bin/sh
f() {
\t(
\t\tcontinue
\t)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LoopControlOutsideLoop),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "continue");
    }

    #[test]
    fn ignores_continue_inside_functions_when_specific_rule_handles_it() {
        let source = "\
#!/bin/sh
f() {
\tcontinue
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([
                Rule::LoopControlOutsideLoop,
                Rule::ContinueOutsideLoopInFunction,
            ]),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ContinueOutsideLoopInFunction);
        assert_eq!(diagnostics[0].span.slice(source), "continue");
    }

    #[test]
    fn still_reports_break_inside_functions() {
        let source = "\
#!/bin/sh
f() {
\tbreak
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LoopControlOutsideLoop),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "break");
    }
}

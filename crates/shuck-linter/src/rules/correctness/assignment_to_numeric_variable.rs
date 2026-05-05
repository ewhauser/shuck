use shuck_ast::{DeclOperand, Position, Span};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct AssignmentToNumericVariable;

impl Violation for AssignmentToNumericVariable {
    fn rule() -> Rule {
        Rule::AssignmentToNumericVariable
    }

    fn message(&self) -> String {
        "assignment target is numeric".to_owned()
    }
}

pub fn assignment_to_numeric_variable(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| command_assignment_spans(fact, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AssignmentToNumericVariable);
}

fn command_assignment_spans(fact: crate::facts::CommandFactRef<'_, '_>, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();

    if let Some(span) = command_numeric_assignment_span(fact, source) {
        spans.push(span);
    }

    if let Some(declaration) = fact.declaration() {
        spans.extend(
            declaration
                .operands
                .iter()
                .filter_map(|operand| match operand {
                    DeclOperand::Dynamic(word) => {
                        numeric_assignment_target_span(word.span.slice(source), word.span.start)
                    }
                    DeclOperand::Flag(_) | DeclOperand::Name(_) | DeclOperand::Assignment(_) => {
                        None
                    }
                }),
        );
    }

    spans
}

fn command_numeric_assignment_span(
    fact: crate::facts::CommandFactRef<'_, '_>,
    source: &str,
) -> Option<Span> {
    let text = fact.span().slice(source);
    let first_word = text.split_whitespace().next()?;
    numeric_assignment_target_span(first_word, fact.span().start)
}

fn numeric_assignment_target_span(text: &str, start: Position) -> Option<Span> {
    let target_end = text.find("+=").or_else(|| text.find('='))?;
    let target = &text[..target_end];
    if !target.is_empty() && target.chars().all(|character| character.is_ascii_digit()) {
        Some(Span::from_positions(start, start.advanced_by(target)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_snippet, test_snippet_at_path};
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_numeric_assignment_targets() {
        let source = "\
#!/bin/sh
# shellcheck disable=2288
test \"$2\" || 2=\".\"
export 3=foo
local 4=bar
declare 5=baz
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AssignmentToNumericVariable),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["2", "3", "4", "5"]
        );
    }

    #[test]
    fn ignores_non_numeric_assignment_targets() {
        let source = "\
#!/bin/sh
foo=1
_2=1
a2=1
2foo=1
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AssignmentToNumericVariable),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_numeric_parameter_assignments() {
        let source = "\
#!/bin/zsh
0=${(%):-%N}
1=value
2+=more
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AssignmentToNumericVariable)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_plugin_numeric_zero_assignments_despite_bash_compat_shebang() {
        let source = r#"#!/usr/bin/bash
# shellcheck disable=SC1090,SC2154
0="${${ZERO:-${0:#$ZSH_ARGZERO}}:-${(%):-%N}}"
0="${${(M)0:#/*}:-$PWD/$0}"
"#;
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/ohmyzsh/plugins/shell-proxy/shell-proxy.plugin.zsh"),
            source,
            &LinterSettings::for_rule(Rule::AssignmentToNumericVariable),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}

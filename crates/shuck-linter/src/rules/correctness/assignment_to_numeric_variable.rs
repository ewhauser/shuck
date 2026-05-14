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
    let suppress_assign_special_zero_overlap =
        checker.shell() == ShellDialect::Sh && checker.is_rule_enabled(Rule::AssignSpecialZero);
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            command_assignment_spans(checker, fact, source, suppress_assign_special_zero_overlap)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || AssignmentToNumericVariable);
}

fn command_assignment_spans(
    checker: &Checker<'_>,
    fact: crate::facts::CommandFactRef<'_, '_>,
    source: &str,
    suppress_assign_special_zero_overlap: bool,
) -> Vec<Span> {
    let mut spans = Vec::new();

    if let Some(span) =
        command_numeric_assignment_span(checker, fact, source, suppress_assign_special_zero_overlap)
    {
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
    checker: &Checker<'_>,
    fact: crate::facts::CommandFactRef<'_, '_>,
    source: &str,
    suppress_assign_special_zero_overlap: bool,
) -> Option<Span> {
    let text = fact.span().slice(source);
    let first_word = text.split_whitespace().next()?;
    if suppress_assign_special_zero_overlap
        && first_word.starts_with("0=")
        && !checker.is_suppressed_at(Rule::AssignSpecialZero, fact.span())
    {
        return None;
    }
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

    use shuck_parser::parser::Parser;

    use crate::suppression::parse_directives;
    use crate::test::{test_snippet, test_snippet_at_path};
    use crate::{AnalysisRequest, Indexer, LinterSettings, Rule, ShellCheckCodeMap, ShellDialect};

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

    #[test]
    fn defers_plain_zero_assignment_to_assign_special_zero_when_enabled() {
        let source = "\
#!/bin/sh
0=demo
0+=demo
+0=demo
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([
                Rule::AssignmentToNumericVariable,
                Rule::AssignSpecialZero,
            ]),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.rule, diagnostic.span.slice(source)))
                .collect::<Vec<_>>(),
            vec![
                (Rule::AssignSpecialZero, "0=demo"),
                (Rule::AssignmentToNumericVariable, "0"),
                (Rule::AssignSpecialZero, "0=demo"),
            ]
        );
    }

    #[test]
    fn reports_numeric_assignment_when_assign_special_zero_is_suppressed() {
        let source = "\
#!/bin/sh
# shellcheck disable=SC2280
0=demo
";
        let parse_result = Parser::new(source).parse().unwrap();
        let indexer = Indexer::new(source, &parse_result);
        let directives = parse_directives(
            source,
            indexer.comment_index(),
            &ShellCheckCodeMap::default(),
        );
        let settings =
            LinterSettings::for_rules([Rule::AssignmentToNumericVariable, Rule::AssignSpecialZero]);
        let diagnostics = AnalysisRequest::from_parse_result(&parse_result, source, &settings)
            .with_directives(&directives)
            .lint();

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::AssignmentToNumericVariable);
        assert_eq!(diagnostics[0].span.slice(source), "0");
    }
}

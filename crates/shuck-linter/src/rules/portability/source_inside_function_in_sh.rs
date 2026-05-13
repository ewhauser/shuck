use shuck_ast::Span;
use shuck_semantic::{SourceRef, SourceRefKind};

use crate::{
    Checker, CommandFactRef, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation,
};

use super::source_common::source_anchor_span_for_command_fact;

pub struct SourceInsideFunctionInSh;

impl Violation for SourceInsideFunctionInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SourceInsideFunctionInSh
    }

    fn message(&self) -> String {
        "`source` inside a function is not portable in `sh` scripts".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace `source` with `.`".to_owned())
    }
}

pub fn source_inside_function_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let diagnostics = checker
        .semantic()
        .source_refs()
        .iter()
        .filter(|source_ref| {
            matches!(
                source_ref.kind,
                SourceRefKind::Directive(_) | SourceRefKind::DirectiveDevNull
            )
        })
        .filter_map(|source_ref| source_command_for_ref(checker, source_ref))
        .filter(|command| inside_function(checker, command.span()))
        .filter_map(|command| {
            let diagnostic_span = source_anchor_span_for_command_fact(command, checker.source());
            let fix_span = command.body_name_word()?.span;
            Some(
                Diagnostic::new(SourceInsideFunctionInSh, diagnostic_span)
                    .with_fix(Fix::unsafe_edit(Edit::replacement(".", fix_span))),
            )
        })
        .collect::<Vec<_>>();
    for diagnostic in diagnostics {
        checker.report_diagnostic(diagnostic);
    }
}

fn source_command_for_ref<'checker, 'ast>(
    checker: &'checker Checker<'ast>,
    source_ref: &SourceRef,
) -> Option<CommandFactRef<'checker, 'ast>> {
    checker
        .facts()
        .commands()
        .iter()
        .find(|fact| fact.effective_name_is("source") && same_start(fact.span(), source_ref.span))
}

fn same_start(left: Span, right: Span) -> bool {
    left.start.offset == right.start.offset
}

fn inside_function(checker: &Checker<'_>, span: Span) -> bool {
    checker
        .semantic_analysis()
        .enclosing_function_scope_at(span.start.offset)
        .is_some()
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_source_inside_function_in_sh() {
        let source = "#!/bin/sh\nf() {\n  # shellcheck source=/dev/null\n  source ./lib.sh\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SourceInsideFunctionInSh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "source ./lib.sh");
    }

    #[test]
    fn applies_unsafe_fix_to_source_inside_function_in_sh() {
        let source = "#!/bin/sh\nf() {\n  # shellcheck source=/dev/null\n  source ./lib.sh\n}\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SourceInsideFunctionInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nf() {\n  # shellcheck source=/dev/null\n  . ./lib.sh\n}\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}

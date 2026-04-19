use shuck_ast::AssignmentValue;
use shuck_semantic::ScopeKind;

use crate::{
    Checker, DeclarationKind, ExpansionContext, Rule, Violation, WordFactContext,
    assignment_name_span,
};

pub struct ExportCommandSubstitution {
    pub name: String,
}

impl Violation for ExportCommandSubstitution {
    fn rule() -> Rule {
        Rule::ExportCommandSubstitution
    }

    fn message(&self) -> String {
        format!("assign command output before declaring `{}`", self.name)
    }
}

pub fn export_command_substitution(checker: &mut Checker) {
    let findings = checker
        .facts()
        .structural_commands()
        .filter_map(|fact| {
            let declaration = fact.declaration()?;
            should_report_s010_declaration(
                checker,
                &declaration.kind,
                declaration.readonly_flag,
                fact.span(),
            )
            .then_some(declaration)
        })
        .flat_map(|declaration| declaration.assignment_operands.iter().copied())
        .filter_map(|assignment| {
            let AssignmentValue::Scalar(word) = &assignment.value else {
                return None;
            };

            checker
                .facts()
                .word_fact(
                    word.span,
                    WordFactContext::Expansion(ExpansionContext::DeclarationAssignmentValue),
                )
                .filter(|fact| fact.classification().has_command_substitution())
                .map(|_| {
                    (
                        assignment.target.name.to_string(),
                        assignment_name_span(assignment),
                    )
                })
        })
        .collect::<Vec<_>>();

    for (name, span) in findings {
        checker.report_dedup(ExportCommandSubstitution { name }, span);
    }
}

fn matches_s010_declaration_kind(kind: &DeclarationKind) -> bool {
    matches!(
        kind,
        DeclarationKind::Export
            | DeclarationKind::Local
            | DeclarationKind::Declare
            | DeclarationKind::Typeset
    ) || matches!(kind, DeclarationKind::Other(name) if name == "readonly")
}

fn should_report_s010_declaration(
    checker: &Checker,
    kind: &DeclarationKind,
    readonly_flag: bool,
    span: shuck_ast::Span,
) -> bool {
    if !matches_s010_declaration_kind(kind) {
        return false;
    }

    if !readonly_flag || matches!(kind, DeclarationKind::Export) {
        return true;
    }

    if matches!(kind, DeclarationKind::Other(name) if name == "readonly") {
        return true;
    }

    let scope = checker.semantic().scope_at(span.start.offset);
    let inside_function = checker
        .semantic()
        .ancestor_scopes(scope)
        .any(|scope| matches!(checker.semantic().scope_kind(scope), ScopeKind::Function(_)));

    !inside_function
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_declaration_assignment_names() {
        let source = "\
#!/bin/bash
export greeting=$(printf '%s\\n' hi)
demo() {
  local temp=\"$(date)\"
  readonly keep_me=$(date)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["greeting", "temp", "keep_me"]
        );
    }

    #[test]
    fn ignores_function_local_readonly_modifier_declaration_assignments() {
        let source = "\
#!/bin/bash
demo() {
  local -r temp=\"$(date)\"
  declare -r other=$(date)
  typeset -r home=$(pwd)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_top_level_readonly_declaration_assignments() {
        let source = "\
#!/bin/bash
readonly temp=\"$(mktemp)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["temp"]
        );
    }

    #[test]
    fn reports_top_level_readonly_declare_variants() {
        let source = "\
#!/bin/bash
declare -r archive_dir_create=\"$(mktemp -dt archive_dir_create.XXXXXX)\"
typeset -r n_version=\"$(./bin/n --version)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["archive_dir_create", "n_version"]
        );
    }

    #[test]
    fn reports_top_level_readonly_variants_from_corpus() {
        let source = "\
#!/bin/bash
readonly archive_dir_create=\"$(mktemp -dt archive_dir_create.XXXXXX)\"
readonly n_version=\"$(./bin/n --version)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["archive_dir_create", "n_version"]
        );
    }
}

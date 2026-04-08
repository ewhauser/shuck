use crate::SubstitutionHostKind;
use crate::rules::common::query::CommandSubstitutionKind;
use crate::{Checker, Rule, Violation};

pub struct UnquotedCommandSubstitution;

impl Violation for UnquotedCommandSubstitution {
    fn rule() -> Rule {
        Rule::UnquotedCommandSubstitution
    }

    fn message(&self) -> String {
        "quote command substitutions in arguments to avoid word splitting".to_owned()
    }
}

pub fn unquoted_command_substitution(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            fact.substitution_facts()
                .iter()
                .filter(|substitution| substitution.kind() == CommandSubstitutionKind::Command)
                .filter(|substitution| substitution.unquoted_in_host())
                .filter(|substitution| {
                    matches!(
                        substitution.host_kind(),
                        SubstitutionHostKind::CommandArgument
                            | SubstitutionHostKind::AssignmentTargetSubscript
                            | SubstitutionHostKind::DeclarationNameSubscript
                            | SubstitutionHostKind::ArrayKeySubscript
                    )
                })
                .map(|substitution| substitution.span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedCommandSubstitution);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_inner_command_substitution_spans() {
        let source = "printf '%s\\n' prefix$(date)suffix $(uname)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(date)", "$(uname)"]
        );
    }

    #[test]
    fn ignores_redirect_and_here_string_contexts() {
        let source = "\
#!/bin/bash
cat <<< $(printf here) >$(printf out)
printf '%s\\n' $(printf arg)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf arg)"]
        );
    }

    #[test]
    fn reports_subscript_command_substitutions_without_flagging_assignment_rhs() {
        let source = "\
declare arr[$(printf hi)]=1
stamp=$(printf ok)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf hi)"]
        );
    }

    #[test]
    fn reports_declaration_name_and_array_key_subscript_substitutions() {
        let source = "\
declare arr[$(printf decl-name)]
declare -A map=([$(printf key)]=1)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf decl-name)", "$(printf key)"]
        );
    }
}

use crate::SubstitutionHostKind;
use crate::{Checker, CommandSubstitutionKind, Rule, ShellDialect, Violation};

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
                            | SubstitutionHostKind::HereStringOperand
                            | SubstitutionHostKind::AssignmentTargetSubscript
                            | SubstitutionHostKind::DeclarationNameSubscript
                            | SubstitutionHostKind::ArrayKeySubscript
                    ) || (substitution.host_kind()
                        == SubstitutionHostKind::DeclarationAssignmentValue
                        && checker.shell() == ShellDialect::Sh)
                })
                .map(|substitution| substitution.span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedCommandSubstitution);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

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
    fn reports_here_strings_but_ignores_other_redirect_contexts() {
        let source = "\
#!/bin/bash
cat <<< $(printf here) <<< \"$(printf quoted-here)\" >$(printf out)
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
            vec!["$(printf here)", "$(printf arg)"]
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

    #[test]
    fn ignores_declaration_assignment_value_substitutions() {
        let source = "\
local name=$(printf local)
declare other=$(printf declare)
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
    fn reports_declaration_assignment_value_substitutions_in_sh() {
        let source = "\
local name=$(printf local)
declare other=$(printf declare)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution)
                .with_shell(ShellDialect::Sh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf local)", "$(printf declare)"]
        );
    }
}

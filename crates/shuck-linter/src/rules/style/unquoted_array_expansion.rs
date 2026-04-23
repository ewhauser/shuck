use crate::{Checker, ExpansionContext, Rule, ShellDialect, Violation};

pub struct UnquotedArrayExpansion;

impl Violation for UnquotedArrayExpansion {
    fn rule() -> Rule {
        Rule::UnquotedArrayExpansion
    }

    fn message(&self) -> String {
        "quote array expansions to preserve element boundaries".to_owned()
    }
}

pub fn unquoted_array_expansion(checker: &mut Checker) {
    let mut spans = [
        ExpansionContext::CommandName,
        ExpansionContext::CommandArgument,
        ExpansionContext::HereString,
        ExpansionContext::ForList,
        ExpansionContext::SelectList,
    ]
    .into_iter()
    .flat_map(|context| checker.facts().expansion_word_facts(context))
    .flat_map(|fact| {
        fact.unquoted_all_elements_array_expansion_spans()
            .iter()
            .copied()
    })
    .collect::<Vec<_>>();

    if checker.shell() == ShellDialect::Sh {
        spans.extend(
            checker
                .facts()
                .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue)
                .flat_map(|fact| {
                    fact.unquoted_all_elements_array_expansion_spans()
                        .iter()
                        .copied()
                }),
        );
    }

    checker.report_all_dedup(spans, || UnquotedArrayExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_inner_array_expansion_spans() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix${arr[@]}suffix ${arr[0]} ${names[*]}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}"]
        );
    }

    #[test]
    fn ignores_redirect_targets_but_reports_here_strings() {
        let source = "\
#!/bin/bash
arr=(a b)
printf '%s\\n' ok >${paths[@]}
cat <<< ${items[@]}
cat <<< ${items[@]:+fallback}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${items[@]}"]
        );
    }

    #[test]
    fn ignores_array_values_that_stay_single_field_when_quoted() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${names[*]}\" \"${arr[0]}\" \"$@\" \"${@:2}\" \"${items[@]}\" \"${items[@]:1}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_for_and_select_list_array_expansions() {
        let source = "\
#!/bin/bash
arr=(a b)
for item in ${arr[@]}; do
  :
done
select item in ${arr[@]}; do
  break
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}", "${arr[@]}"]
        );
    }

    #[test]
    fn reports_at_style_positional_parameter_expansions_only() {
        let source = "\
#!/bin/bash
printf '%s\\n' $@ ${@:2} $*
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@", "${@:2}"]
        );
    }

    #[test]
    fn reports_command_name_and_array_slice_at_expansions() {
        let source = "\
#!/bin/bash
arr=(a b c)
${arr[@]:0:1} --flag
printf '%s\\n' ${arr[@]:1} ${arr[*]:1} ${!arr[@]} ${arr[@]/#/#} ${arr[@]@Q} ${arr[@]:-fallback} ${arr[@]:+fallback}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${arr[@]:0:1}",
                "${arr[@]:1}",
                "${!arr[@]}",
                "${arr[@]/#/#}",
                "${arr[@]@Q}",
                "${arr[@]:-fallback}",
            ]
        );
    }

    #[test]
    fn reports_trap_signal_array_expansions() {
        let source = "\
#!/bin/bash
trap \"trap - ${sig[*]}; kill $pid\" ${sig[@]}
trap - ${sig[@]}
trap 'echo $@' EXIT
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${sig[@]}", "${sig[@]}"]
        );
    }

    #[test]
    fn reports_sh_declaration_assignment_splats() {
        let source = "\
local dst=${src[@]}
export args=${argv[@]}
declare copy=(${src[@]})
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion).with_shell(ShellDialect::Sh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${src[@]}", "${argv[@]}", "${src[@]}"]
        );
    }

    #[test]
    fn ignores_declaration_assignment_splats_in_bash_mode() {
        let source = "\
#!/bin/bash
local dst=${src[@]}
declare copy=(${src[@]})
printf '%s\\n' ${src[@]}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion).with_shell(ShellDialect::Bash),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${src[@]}"]
        );
    }

    #[test]
    fn ignores_star_splats_that_have_their_own_rule() {
        let source = "\
#!/bin/bash
arr=(a b c)
${arr[*]:0:1} --flag
printf '%s\\n' $* ${arr[*]} ${arr[*]:1}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {diagnostics:#?}"
        );
    }
}

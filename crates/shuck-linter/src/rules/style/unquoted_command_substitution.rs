use crate::{
    Checker, ExpansionContext, Rule, ShellDialect, Violation, WordFactHostKind, WordOccurrenceRef,
};

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
    let arithmetic_spans = checker.facts().arithmetic_command_substitution_spans();
    let pgrep_spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.substitution_facts().iter())
        .filter(|fact| fact.body_is_pgrep_lookup())
        .map(|fact| fact.span())
        .collect::<Vec<_>>();
    let seq_spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.substitution_facts().iter())
        .filter(|fact| fact.body_is_seq_utility())
        .map(|fact| fact.span())
        .collect::<Vec<_>>();
    let inert_spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.substitution_facts().iter())
        .filter(|fact| !fact.body_has_commands())
        .map(|fact| fact.span())
        .collect::<Vec<_>>();
    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .flat_map(|fact| {
            fact.unquoted_command_substitution_spans()
                .iter()
                .copied()
                .filter(|span| {
                    should_report_unquoted_command_substitution(
                        checker,
                        fact,
                        *span,
                        arithmetic_spans,
                        &pgrep_spans,
                        &seq_spans,
                        &inert_spans,
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedCommandSubstitution);
}

fn should_report_unquoted_command_substitution(
    checker: &Checker,
    fact: WordOccurrenceRef<'_, '_>,
    span: shuck_ast::Span,
    arithmetic_spans: &[shuck_ast::Span],
    pgrep_spans: &[shuck_ast::Span],
    seq_spans: &[shuck_ast::Span],
    inert_spans: &[shuck_ast::Span],
) -> bool {
    if arithmetic_spans.contains(&span) || inert_spans.contains(&span) {
        return false;
    }

    let Some(context) = fact.expansion_context() else {
        return false;
    };

    match (context, fact.host_kind()) {
        (ExpansionContext::CommandName, WordFactHostKind::Direct) => fact.has_literal_affixes(),
        (ExpansionContext::HereString, WordFactHostKind::Direct)
        | (ExpansionContext::RedirectTarget(_), WordFactHostKind::Direct)
        | (ExpansionContext::DescriptorDupTarget(_), WordFactHostKind::Direct)
        | (
            ExpansionContext::DeclarationAssignmentValue,
            WordFactHostKind::DeclarationNameSubscript,
        ) => true,
        (
            ExpansionContext::DeclarationAssignmentValue,
            WordFactHostKind::ArrayKeySubscript,
        ) => checker.shell() == ShellDialect::Sh,
        (ExpansionContext::DeclarationAssignmentValue, WordFactHostKind::Direct) => {
            checker.shell() == ShellDialect::Sh
        }
        (ExpansionContext::CommandArgument, WordFactHostKind::Direct) => {
            !(pgrep_spans.contains(&span) || seq_spans.contains(&span))
        }
        _ => false,
    }
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
    fn reports_command_names_and_redirect_targets() {
        let source = "\
#!/bin/bash
$(pwd)/tool --flag
cat <<< $(printf here) <<< \"$(printf quoted-here)\" >$(printf out) >&$(printf fd)
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
            vec![
                "$(pwd)",
                "$(printf here)",
                "$(printf out)",
                "$(printf fd)",
                "$(printf arg)",
            ]
        );
    }

    #[test]
    fn ignores_assignment_target_subscript_command_substitutions() {
        let source = "\
declare arr[$(printf hi)]=1
arr[$(printf '1 + 1')]=2
stamp=$(printf ok)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_declaration_name_subscript_but_ignores_array_keys() {
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
            vec!["$(printf decl-name)"]
        );
    }

    #[test]
    fn reports_array_key_subscript_in_sh_mode() {
        let source = "\
#!/bin/sh
declare arr[$(printf decl-name)]
declare -A map=([$(printf key)]=1)
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

    #[test]
    fn ignores_bare_command_names_and_arithmetic() {
        let source = "\
#!/bin/sh
$(printf helper) --flag
printf '%s\\n' $(($(date +%s) - $(date +%s)))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_pgrep_seq_and_comment_only_backticks_in_command_arguments() {
        let source = "\
#!/bin/bash
if [ $(pgrep -f service) ]; then
  :
fi
echo $(pgrep service)
readlink /proc/$(pgrep -x service)/exe >/dev/null
printf '%0.s-' $(seq 1 3)
cmake \\
  `# comment` \\
  -DFOO=1
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_pidof_in_truthy_tests_and_kill() {
        let source = "\
#!/bin/sh
if [ $(pidof service) ]; then
  :
fi
kill $(pidof service)
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
            vec!["$(pidof service)", "$(pidof service)"]
        );
    }

    #[test]
    fn reports_non_truthy_test_operands_and_nested_parameter_expansion_commands() {
        let source = "\
#!/bin/sh
if [ $(printf one) = one ]; then
  :
fi
if [ $(command -v pigz) ]; then
  :
fi
NUMJOBS=${NUMJOBS:-\" -j $(expr $(nproc) + 1) \"}
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
            vec!["$(printf one)", "$(command -v pigz)", "$(nproc)"]
        );
    }

    #[test]
    fn reports_filename_builder_command_substitutions() {
        let source = "\
#!/bin/bash
/sbin/makepkg -l y -c n $OUTPUT/$PRGNAM-$VERSION\\_$(echo ${KERNEL} | tr '-' '_')-$ARCH-$BUILD$TAG.$PKGTYPE
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
            vec!["$(echo ${KERNEL} | tr '-' '_')"]
        );
    }

    #[test]
    fn reports_docker_ps_command_substitutions_in_arguments() {
        let source = "\
#!/bin/bash
docker inspect -f '{{ if ne \"true\" (index .Config.Labels \"com.dokku.devcontainer\") }}{{.ID}} {{ end }}' $(docker ps -q)
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
            vec!["$(docker ps -q)"]
        );
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 105);
        assert_eq!(diagnostics[0].span.end.column, 120);
    }

    #[test]
    fn ignores_kill_pid_lists() {
        let source = "\
#!/bin/sh
kill $(pgrep service)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_kill_pidfile_command_substitutions() {
        let source = "\
#!/bin/sh
kill $(cat \"$pidfile\")
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
            vec!["$(cat \"$pidfile\")"]
        );
    }

    #[test]
    fn reports_inner_command_substitution_inside_heredoc_eval_wrapper() {
        let source = "\
#!/bin/bash
cfgtest=true
name=QtCore
cfgtest_QtCore=\"shared\"
if test \"${cfgtest}\"; then
\tcat <<-EOF > \"${name}\"
\t\t#!/bin/sh
\t\ttest \"\\$#\" -ge 1 || exit 1
\t\techo $(eval echo \\$$(echo cfgtest_${name})) | tr ' ' '\\n' > \\$1
\tEOF
fi
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
            vec!["$(echo cfgtest_${name})"]
        );
        assert_eq!(diagnostics[0].span.start.line, 9);
        assert_eq!(diagnostics[0].span.start.column, 22);
        assert_eq!(diagnostics[0].span.end.column, 45);
    }
}

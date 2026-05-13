use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct CStyleForArithmeticInSh;

impl Violation for CStyleForArithmeticInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::CStyleForArithmeticInSh
    }

    fn message(&self) -> String {
        "arithmetic `++` and `--` operators are not portable in `sh` scripts".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite the arithmetic update explicitly".to_owned())
    }
}

pub fn c_style_for_arithmetic_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    checker.report_fact_diagnostics(|facts, report| {
        let fix_facts = facts.arithmetic_update_operator_fix_facts();
        for span in facts.arithmetic_update_operator_spans().iter().copied() {
            let diagnostic = Diagnostic::new(CStyleForArithmeticInSh, span);
            let diagnostic = match fix_facts.iter().find(|fact| fact.diagnostic_span() == span) {
                Some(fact) => diagnostic.with_fix(Fix::unsafe_edit(Edit::replacement(
                    fact.replacement(),
                    fact.replacement_span(),
                ))),
                None => diagnostic,
            };
            report(diagnostic);
        }
    });
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_update_operators_inside_c_style_for() {
        let source = "#!/bin/sh\nfor ((++i; j < 3; k--)); do :; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_standalone_arithmetic() {
        let source = "#!/bin/sh\n((++i))\n((j--))\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_arithmetic_update_operators() {
        let source = "#!/bin/sh\n((++i))\necho \"$((j--))\"\narr[i++]=x\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\n(((i = i + 1)))\necho \"$(((j = j - 1)))\"\narr[(i = i + 1)]=x\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn anchors_on_update_operators_inside_arithmetic_expansions() {
        let source = "#!/bin/sh\necho \"$((++i)) $((j--))\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_expanding_heredoc_bodies() {
        let source = "#!/bin/sh\ncat <<EOF\n$((++i))\n$(printf '%s' \"$((j--))\")\nEOF\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_heredoc_parameter_command_substitutions() {
        let source = "#!/bin/sh\ncat <<EOF\n${value:-$(printf '%s' \"$((i++))\")}\nEOF\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_assignment_target_subscripts() {
        let source = "#!/bin/sh\narr[i++]=x\narr[--j]=y\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_compound_assignment_key_words() {
        let source = "#!/bin/sh\narr=([$((i++))]=x [$(printf '%s' \"$((j--))\")]=y)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn anchors_on_update_operators_inside_double_bracket_operands() {
        let source = "#!/bin/sh\n[[ \"$((i++))\" -gt 0 && \"$((j--))\" -lt 3 ]]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++", "--"]
        );
    }

    #[test]
    fn ignores_associative_array_keys_that_look_like_updates() {
        let source = "#!/bin/sh\nlocal -A tools=([c++]=CXX)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_associative_assignment_target_subscripts_that_look_like_updates() {
        let source = "#!/bin/sh\nlocal -A tools[c++]=CXX\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_contextual_associative_assignment_target_subscripts_that_look_like_updates() {
        let source = "#!/bin/sh\ndeclare -A tools\ntools[c++]=CXX\ntools=([d--]=DASH)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_contextual_associative_reference_subscripts_that_look_like_updates() {
        let source = "#!/bin/sh\ndeclare -A tools\necho \"${tools[c++]}\"\n[[ ${tools[d--]} ]]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_caller_associative_reference_subscripts_that_look_like_updates() {
        let source = "\
#!/bin/sh
helper() {
  echo \"${tools[c++]}\"
  [[ ${tools[d--]} ]]
}
main() {
  declare -A tools
  helper
}
main
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_shadowed_caller_associative_reference_subscripts_that_look_like_updates() {
        let source = "\
#!/bin/sh
helper() {
  local tools
  echo \"${tools[c++]}\"
}
main() {
  declare -A tools
  helper
}
main
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["++"]
        );
    }

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\nfor ((++i; j < 3; k--)); do :; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForArithmeticInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}

use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct EvalOnArray;

impl Violation for EvalOnArray {
    fn rule() -> Rule {
        Rule::EvalOnArray
    }

    fn message(&self) -> String {
        "array expansion passed to `eval` is reinterpreted as shell text".to_owned()
    }
}

pub fn eval_on_array(checker: &mut Checker) {
    let command_arg_facts = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .collect::<Vec<_>>();

    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("eval"))
        .flat_map(|command| command.body_args())
        .flat_map(|word| {
            command_arg_facts
                .iter()
                .copied()
                .filter(move |fact| fact.span() == word.span)
                .flat_map(|fact| {
                    fact.direct_all_elements_array_expansion_spans()
                        .iter()
                        .copied()
                })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || EvalOnArray);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_pyenv_style_eval_string_array_expansion() {
        let source = "\
#!/bin/bash
shims=(a)
eval \"conda_shim() { case \\\"\\${1##*/}\\\" in ${shims[@]} *) return 1;; esac }\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EvalOnArray));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "${shims[@]}");
    }

    #[test]
    fn anchors_multiline_eval_string_array_expansions_at_the_dollar() {
        let source = "\
#!/bin/bash
shims=(a)
eval \\
\"conda_shim() {
  case \\\"\\${1##*/}\\\" in
    ${shims[@]}
    *) return 1;;
  esac
}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EvalOnArray));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "${shims[@]}");
        assert_eq!(diagnostics[0].span.start.line, 6);
        assert_eq!(diagnostics[0].span.start.column, 5);
    }

    #[test]
    fn ignores_escaped_array_text_built_for_later_eval() {
        let source = "\
#!/bin/bash
eval command sudo \\\"\\${sudo_args[@]}\\\" $(printf '%s\\n' env=1) \\\"\\$@\\\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EvalOnArray));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_scalar_parameter_expansions_with_literal_array_selector_text() {
        let source = "\
#!/bin/bash
eval \"${name:-safe[@]}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EvalOnArray));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_posix_forwarding_idioms_that_only_embed_quoted_positional_params() {
        let source = "\
#!/bin/sh
eval shellspec_join SHELLSPEC_EXPECTATION '\" \"' The ${1+'\"$@\"'}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EvalOnArray));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_replacement_forms_that_do_not_expand_the_positional_splat_itself() {
        let source = "\
#!/bin/bash
eval \"${@:+ok}\"
eval \"${args[@]:+ok}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EvalOnArray));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_nested_positional_splats_inside_escaped_parameter_text() {
        let source = "\
#!/bin/bash
eval \"\\${1+'\\\"$@\\\"'}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EvalOnArray));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_quoted_braces_inside_escaped_parameter_text() {
        let source = "\
#!/bin/bash
eval \"\\${1+'} \\\"$@\\\"'}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EvalOnArray));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_direct_positional_splats_after_escaped_parameter_text_in_eval_strings() {
        let source = "\
#!/bin/bash
eval \"echo \\${1##*/} $@\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EvalOnArray));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "$@");
    }
}

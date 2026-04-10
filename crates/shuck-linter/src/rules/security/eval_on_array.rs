use crate::{Checker, ExpansionContext, Rule, Violation, WordFactContext};

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
    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("eval"))
        .flat_map(|fact| fact.body_args())
        .filter_map(|word| {
            checker.facts().word_fact(
                word.span,
                WordFactContext::Expansion(ExpansionContext::CommandArgument),
            )
        })
        .flat_map(|fact| fact.all_elements_array_expansion_spans().iter().copied())
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
}

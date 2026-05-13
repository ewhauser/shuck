use crate::{Checker, Rule, S083FunctionDocRequirement, Violation};

pub struct MissingFunctionDoc {
    name: String,
}

impl Violation for MissingFunctionDoc {
    fn rule() -> Rule {
        Rule::MissingFunctionDoc
    }

    fn message(&self) -> String {
        format!("add a leading comment block for function `{}`", self.name)
    }
}

pub fn missing_function_doc(checker: &mut Checker) {
    let options = checker.rule_options().s083.clone();
    let violations = checker
        .facts()
        .command_facts()
        .function_doc_content()
        .iter()
        .filter(|fact| !fact.has_leading_comment())
        .filter(|fact| function_requires_doc(fact, &options))
        .map(|fact| {
            (
                fact.name_span(),
                MissingFunctionDoc {
                    name: fact.name().as_str().to_owned(),
                },
            )
        })
        .collect::<Vec<_>>();

    for (span, violation) in violations {
        checker.report(violation, span);
    }
}

fn function_requires_doc(
    fact: &crate::facts::FunctionDocContentFact,
    options: &crate::S083RuleOptions,
) -> bool {
    match options.require_for {
        S083FunctionDocRequirement::All => true,
        S083FunctionDocRequirement::Exported => !fact.name().as_str().starts_with('_'),
        S083FunctionDocRequirement::Long => {
            fact.body_line_count() > options.long_function_line_threshold
        }
        S083FunctionDocRequirement::Parameterized => fact.uses_any_positional_parameters(),
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, S083FunctionDocRequirement};

    #[test]
    fn reports_long_function_without_leading_comment_by_default() {
        let source = "#!/bin/bash\ndo_work() {\n  echo one\n  echo two\n  echo three\n  echo four\n  echo five\n  echo six\n  echo seven\n  echo eight\n  echo nine\n  echo ten\n}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::MissingFunctionDoc));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_work");
    }

    #[test]
    fn all_mode_reports_short_functions() {
        let source = "#!/bin/bash\ndo_work() {\n  echo hi\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::All),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "do_work");
    }

    #[test]
    fn accepts_immediate_leading_comment() {
        let source = "#!/bin/bash\n# Does the work.\ndo_work() {\n  echo hi\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::All),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_comment_separated_by_blank_line() {
        let source = "#!/bin/bash\n# Does the work.\n\ndo_work() {\n  echo hi\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::All),
        );

        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn ignores_directive_comments_as_documentation() {
        let source = "#!/bin/bash\n# shellcheck disable=SC2034\ndo_work() {\n  echo hi\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::All),
        );

        assert_eq!(diagnostics.len(), 1);
    }

    #[test]
    fn exported_mode_ignores_private_functions() {
        let source = "#!/bin/bash\n_private() {\n  echo hi\n}\npublic() {\n  echo hi\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::Exported),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "public");
    }

    #[test]
    fn long_mode_uses_body_line_threshold() {
        let source = "#!/bin/bash\nshort() {\n  echo hi\n}\nlong() {\n  echo one\n  echo two\n  echo three\n  echo four\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::Long)
                .with_s083_long_function_line_threshold(3),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "long");
    }

    #[test]
    fn long_mode_accepts_body_at_threshold() {
        let source = "#!/bin/bash\nexact() {\n  echo one\n  echo two\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::Long)
                .with_s083_long_function_line_threshold(4),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn parameterized_mode_checks_positional_parameter_use() {
        let source = "#!/bin/bash\nplain() {\n  echo hi\n}\nwith_arg() {\n  echo \"$1\"\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::Parameterized),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "with_arg");
    }

    #[test]
    fn parameterized_mode_counts_guarded_positional_parameter_use() {
        let source = "#!/bin/bash\nplain() {\n  echo hi\n}\nwith_default() {\n  printf '%s\\n' \"${1:-default}\"\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::Parameterized),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "with_default");
    }

    #[test]
    fn parameterized_mode_ignores_guarded_positional_parameter_after_local_reset() {
        let source = "#!/bin/bash\nlocal_args() {\n  (\n    set -- inner\n    printf '%s\\n' \"${1:-default}\"\n  )\n}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MissingFunctionDoc)
                .with_s083_require_for(S083FunctionDocRequirement::Parameterized),
        );

        assert!(diagnostics.is_empty());
    }
}

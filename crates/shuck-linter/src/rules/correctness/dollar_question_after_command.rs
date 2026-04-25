use crate::{Checker, Rule, Violation};

pub struct DollarQuestionAfterCommand;

impl Violation for DollarQuestionAfterCommand {
    fn rule() -> Rule {
        Rule::DollarQuestionAfterCommand
    }

    fn message(&self) -> String {
        "test the command directly instead of reading its status back from `$?`".to_owned()
    }
}

pub fn dollar_question_after_command(checker: &mut Checker) {
    checker.report_fact_slice(
        |facts| facts.dollar_question_after_command_spans(),
        || DollarQuestionAfterCommand,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_status_checks_in_test_contexts() {
        let source = "\
#!/bin/bash
run
if [ $? -ne 0 ]; then :; fi
run
[ $? -ne 0 ]
run && [ $? -eq 0 ]
run || [ $? -ne 0 ]
if (( $? != 0 )); then :; fi
while [[ $? -ne 0 ]]; do break; done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarQuestionAfterCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?", "$?", "$?", "$?", "$?"]
        );
    }

    #[test]
    fn ignores_non_test_uses_and_saved_status() {
        let source = "\
#!/bin/bash
run
saved=$?
if [ \"$saved\" -ne 0 ]; then :; fi
case $? in 0) : ;; esac
test $? -ne 0
exit $?
[ $? -eq 1 ]
[[ \"$name\" = ok || $? -eq 1 ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarQuestionAfterCommand),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_leading_function_body_checks_without_a_prior_command() {
        let source = "\
#!/bin/bash
check_status() {
  if [ $? -ne 0 ]; then :; fi
  [ $? -ne 0 ]
  run && [ $? -ne 0 ]
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarQuestionAfterCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?"]
        );
    }

    #[test]
    fn reports_group_and_branch_followups() {
        let source = "\
#!/bin/bash
{ [ $? -ne 0 ]; }
if [ \"$x\" = y ]; then
  [ $? -ne 0 ]
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarQuestionAfterCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?"]
        );
    }

    #[test]
    fn reports_pipeline_rhs_status_checks_even_at_function_entry() {
        let source = "\
#!/bin/bash
check_status() {
  run | [ $? -eq 0 ]
  run |& [ $? -eq 0 ]
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarQuestionAfterCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?"]
        );
    }

    #[test]
    fn ignores_noncanonical_zero_spellings() {
        let source = "\
#!/bin/bash
run
[ $? -eq 00 ]
[ $? -ne 000 ]
[ $? -gt +0 ]
[[ $? == 00 ]]
(( $? == 00 ))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarQuestionAfterCommand),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_arithmetic_for_header_checks() {
        let source = "\
#!/bin/bash
run
for (( $? == 0; ; )); do break; done
run
for (( ; $? == 0; )); do break; done
run
for (( ; ; $? == 0 )); do break; done
check_loop_status() {
  for (( $? == 0; ; )); do break; done
  for (( ; $? == 0; )); do break; done
  for (( ; ; $? == 0 )); do break; done
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarQuestionAfterCommand),
        );

        assert!(diagnostics.is_empty());
    }
}

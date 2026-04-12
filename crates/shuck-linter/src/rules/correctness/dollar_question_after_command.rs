use crate::{Checker, Rule, Violation};

pub struct DollarQuestionAfterCommand;

impl Violation for DollarQuestionAfterCommand {
    fn rule() -> Rule {
        Rule::DollarQuestionAfterCommand
    }

    fn message(&self) -> String {
        "`$?` here reflects a later command, not the status you likely meant to keep".to_owned()
    }
}

pub fn dollar_question_after_command(checker: &mut Checker) {
    checker.report_all(
        checker
            .facts()
            .dollar_question_after_command_spans()
            .to_vec(),
        || DollarQuestionAfterCommand,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_status_uses_after_output_commands() {
        let source = "\
#!/bin/bash
run
echo status
if [ $? -ne 0 ]; then :; fi
run
printf '%s\\n' status
case $? in 0) : ;; esac
check_status() {
  run
  printf '%s\\n' status
  return $?
}
run
echo status
saved=$?
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
            vec!["$?", "$?", "$?", "$?"]
        );
    }

    #[test]
    fn ignores_immediate_checks_and_saved_status() {
        let source = "\
#!/bin/bash
run
if [ $? -ne 0 ]; then :; fi
run
saved=$?
echo status
if [ \"$saved\" -ne 0 ]; then :; fi
run
pwd >/dev/null
if [ $? -ne 0 ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarQuestionAfterCommand),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_case_subjects_without_a_qualifying_intervening_output_command() {
        let source = "\
#!/bin/bash
awk 'BEGIN { exit 0 }'
case $? in
  0) : ;;
esac
groupadd demo
case $? in
  0) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DollarQuestionAfterCommand),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_short_circuit_followups_after_output_commands() {
        let source = "\
#!/bin/bash
run
echo status && [ $? -ne 0 ]
run
printf '%s\\n' status || return $?
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
}

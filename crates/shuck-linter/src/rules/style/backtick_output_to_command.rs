use crate::{
    Checker, CommandSubstitutionKind, Rule, SubstitutionHostKind, Violation,
    unescaped_backtick_command_substitution_span,
};

pub struct BacktickOutputToCommand;

impl Violation for BacktickOutputToCommand {
    fn rule() -> Rule {
        Rule::BacktickOutputToCommand
    }

    fn message(&self) -> String {
        "avoid passing unquoted backtick output directly as command arguments".to_owned()
    }
}

pub fn backtick_output_to_command(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.substitution_facts().iter().copied())
        .filter(|substitution| substitution.kind() == CommandSubstitutionKind::Command)
        .filter(|substitution| substitution.host_kind() == SubstitutionHostKind::CommandArgument)
        .filter(|substitution| substitution.unquoted_in_host())
        .filter(|substitution| substitution.uses_backtick_syntax())
        .filter_map(|substitution| {
            unescaped_backtick_command_substitution_span(substitution.span(), checker.source())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BacktickOutputToCommand);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_backtick_output_in_command_arguments() {
        let source = "\
#!/bin/sh
git branch -d `git branch --merged | grep -v '^*' | grep -v master | tr -d '\\n'`
printf '%s\\n' prefix`uname`suffix `date`
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BacktickOutputToCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "`git branch --merged | grep -v '^*' | grep -v master | tr -d '\\n'`",
                "`uname`",
                "`date`"
            ]
        );
    }

    #[test]
    fn ignores_non_argument_or_non_backtick_substitutions() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"`uname`\" \"$(date)\" $(date)
printf '%s\\n' \\`escaped\\`
stamp=`date`
arr=(`printf '%s\\n' one two`)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BacktickOutputToCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            Vec::<&str>::new()
        );
    }
}

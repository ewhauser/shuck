use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct CommandSubstitutionInAlias;

impl Violation for CommandSubstitutionInAlias {
    fn rule() -> Rule {
        Rule::CommandSubstitutionInAlias
    }

    fn message(&self) -> String {
        "avoid command substitutions in alias definitions".to_owned()
    }
}

pub fn command_substitution_in_alias(checker: &mut Checker) {
    let source = checker.source();
    let alias_definition_spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| {
            fact.effective_name_is("alias")
                && fact
                    .body_args()
                    .iter()
                    .any(|word| word.span.slice(source).contains('='))
        })
        .flat_map(|fact| {
            fact.body_args()
                .iter()
                .filter(|word| word.span.slice(source).contains('='))
                .map(|word| word.span)
        })
        .collect::<Vec<_>>();

    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .filter(|fact| fact.expansion_context() == Some(ExpansionContext::CommandArgument))
        .filter(|fact| alias_definition_spans.contains(&fact.span()))
        .flat_map(|fact| fact.command_substitution_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CommandSubstitutionInAlias);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_command_substitutions_inside_alias_definitions() {
        let source = "\
#!/bin/sh
alias printf=$(command -v printf)
alias a=$(command -v printf) b=$(command -v cat)
alias plain=printf
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommandSubstitutionInAlias),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$(command -v printf)",
                "$(command -v printf)",
                "$(command -v cat)"
            ]
        );
    }

    #[test]
    fn ignores_aliases_without_command_substitutions() {
        let source = "\
#!/bin/sh
alias printf=printf
alias foo=$BAR
alias plain='$(command -v printf)'
\\alias \"${1-}\" >/dev/null 2>&1
alias -p
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommandSubstitutionInAlias),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_command_substitutions_outside_alias_operands() {
        let source = "\
#!/bin/sh
X=$(date) alias ll='ls -l'
FOO=$(date) BAR=$(uname) alias ll='ls -l'
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommandSubstitutionInAlias),
        );

        assert!(diagnostics.is_empty());
    }
}

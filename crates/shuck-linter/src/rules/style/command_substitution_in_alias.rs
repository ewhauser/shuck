use crate::{Checker, Rule, Violation};

pub struct CommandSubstitutionInAlias;

impl Violation for CommandSubstitutionInAlias {
    fn rule() -> Rule {
        Rule::CommandSubstitutionInAlias
    }

    fn message(&self) -> String {
        "avoid expansions in alias definitions".to_owned()
    }
}

pub fn command_substitution_in_alias(checker: &mut Checker) {
    checker.report_all_dedup(
        checker
            .facts()
            .alias_definition_expansion_spans()
            .iter()
            .copied()
            .collect(),
        || CommandSubstitutionInAlias,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_active_expansions_inside_alias_definitions() {
        let source = "\
#!/bin/bash
alias home=$HOME
alias icloud=\"cd '$HOME'\"
alias printf=$(command -v printf)
alias math=\"$((1+2))\"
alias list=${arr[@]}
alias proc=<(printf hi)
alias brace={a,b}
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
                "$HOME",
                "$HOME",
                "$(command -v printf)",
                "$((1+2))",
                "${arr[@]}",
                "<(printf hi)",
                "{a,b}",
            ]
        );
    }

    #[test]
    fn ignores_aliases_without_active_expansions() {
        let source = "\
#!/bin/bash
alias printf=printf
alias plain='$(command -v printf)'
alias param='${HOME}'
alias brace='{a,b}'
alias ansi=$'\\n'
alias tilde=~
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

    #[test]
    fn ignores_alias_lookups_with_equals_only_inside_expansions() {
        let source = "\
#!/bin/bash
alias \"${cur%=}\" 2>/dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommandSubstitutionInAlias),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_only_the_first_active_expansion_per_alias_definition() {
        let source = "\
#!/bin/bash
alias \"$a=$b\"
alias \"${method}\"=\"lwp-request -m '${method}'\"
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
            vec!["$a", "${method}"]
        );
    }
}

use crate::{Checker, ExpansionContext, Rule, Violation};

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
    let source = checker.source();
    let word_facts = checker.facts().word_facts();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("alias"))
        .flat_map(|fact| {
            let body_args = fact.body_args();
            let mut definition_spans = Vec::new();
            let mut index = 0usize;

            while let Some(word) = body_args.get(index).copied() {
                let text = word.span.slice(source);
                if !text.contains('=') {
                    index += 1;
                    continue;
                }

                let mut definition_len = 1usize;
                let mut last_word = word;
                while last_word.span.slice(source).ends_with('=')
                    && let Some(next_word) = body_args.get(index + definition_len).copied()
                    && last_word.span.end.offset == next_word.span.start.offset
                {
                    last_word = next_word;
                    definition_len += 1;
                }

                let first_expansion = body_args[index..index + definition_len]
                    .iter()
                    .filter_map(|candidate| {
                        word_facts.iter().find(|fact| {
                            fact.expansion_context() == Some(ExpansionContext::CommandArgument)
                                && fact.span() == candidate.span
                        })
                    })
                    .flat_map(|fact| fact.active_expansion_spans().iter().copied())
                    .min_by_key(|span| (span.start.offset, span.end.offset));
                if let Some(first_expansion) = first_expansion {
                    definition_spans.push(first_expansion);
                }

                index += definition_len;
            }

            definition_spans
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CommandSubstitutionInAlias);
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

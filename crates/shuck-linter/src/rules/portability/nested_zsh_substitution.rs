use shuck_ast::{ParameterExpansionSyntax, WordPart, ZshExpansionTarget};

use super::targets_non_zsh_shell;
use crate::{Checker, Rule, Violation};

pub struct NestedZshSubstitution;

impl Violation for NestedZshSubstitution {
    fn rule() -> Rule {
        Rule::NestedZshSubstitution
    }

    fn message(&self) -> String {
        "nested zsh substitutions are not portable to this shell".to_owned()
    }
}

pub fn nested_zsh_substitution(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .flat_map(|fact| {
            fact.word()
                .parts
                .iter()
                .filter_map(|part| {
                    let WordPart::Parameter(parameter) = &part.kind else {
                        return None;
                    };
                    let ParameterExpansionSyntax::Zsh(syntax) = &parameter.syntax else {
                        return None;
                    };
                    matches!(syntax.target, ZshExpansionTarget::Nested(_))
                        .then_some(syntax.operation.as_ref())
                        .flatten()
                        .map(|_| parameter.span)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || NestedZshSubstitution);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_nested_targets_without_outer_operation() {
        let source = "#!/bin/sh\nversions=(${${(f)\"$(echo test)\"}})\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NestedZshSubstitution),
        );

        assert!(diagnostics.is_empty());
    }
}

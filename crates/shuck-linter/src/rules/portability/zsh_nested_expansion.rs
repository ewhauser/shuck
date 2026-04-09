use shuck_ast::{ParameterExpansionSyntax, WordPart, ZshExpansionTarget};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ZshNestedExpansion;

impl Violation for ZshNestedExpansion {
    fn rule() -> Rule {
        Rule::ZshNestedExpansion
    }

    fn message(&self) -> String {
        "nested zsh parameter expansions are not portable to this shell".to_owned()
    }
}

pub fn zsh_nested_expansion(checker: &mut Checker) {
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
                        .then_some(syntax.operation.is_none())
                        .filter(|is_none| *is_none)
                        .map(|_| parameter.span)
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshNestedExpansion);
}

fn targets_non_zsh_shell(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_nested_targets_with_outer_operations() {
        let source = "#!/bin/sh\nx=${${(M)path:#/*}:-$PWD/$path}\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::ZshNestedExpansion));

        assert!(diagnostics.is_empty());
    }
}

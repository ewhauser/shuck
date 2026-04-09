use super::targets_non_zsh_shell;
use crate::{Checker, Rule, Violation, word_zsh_nested_expansion_spans};

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
        .flat_map(|fact| word_zsh_nested_expansion_spans(fact.word()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshNestedExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_nested_targets_with_outer_operations() {
        let source = "#!/bin/sh\nx=${${(M)path:#/*}:-$PWD/$path}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshNestedExpansion));

        assert!(diagnostics.is_empty());
    }
}

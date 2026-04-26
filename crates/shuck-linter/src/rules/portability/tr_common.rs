use shuck_ast::Span;

use crate::{Checker, ShellDialect};

pub(super) fn tr_exact_operand_spans(checker: &Checker<'_>, exact_text: &str) -> Vec<Span> {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return Vec::new();
    }

    checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("tr") && fact.wrappers().is_empty())
        .flat_map(|fact| {
            fact.options()
                .tr()
                .into_iter()
                .flat_map(|tr| tr.exact_operand_spans(exact_text))
        })
        .collect()
}

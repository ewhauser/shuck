use shuck_ast::DeclOperand;

use crate::{Checker, DeclarationKind, Rule, ShellDialect, Violation};

pub struct LocalDeclareCombined;

impl Violation for LocalDeclareCombined {
    fn rule() -> Rule {
        Rule::LocalDeclareCombined
    }

    fn message(&self) -> String {
        "mix either `local` or `declare`, not both in the same statement".to_owned()
    }
}

pub fn local_declare_combined(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| declaration_combination_span(fact))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || LocalDeclareCombined);
}

fn declaration_combination_span(fact: crate::CommandFactRef<'_, '_>) -> Option<shuck_ast::Span> {
    let declaration = fact.declaration()?;
    let expected = match declaration.kind {
        DeclarationKind::Local => "declare",
        DeclarationKind::Declare => "local",
        DeclarationKind::Export | DeclarationKind::Typeset | DeclarationKind::Other(_) => {
            return None;
        }
    };

    declaration
        .operands
        .iter()
        .find_map(|operand| match operand {
            DeclOperand::Name(name) if name.name.as_str() == expected => Some(name.span),
            DeclOperand::Flag(_) | DeclOperand::Dynamic(_) | DeclOperand::Assignment(_) => None,
            DeclOperand::Name(_) => None,
        })
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_combined_local_and_declare_words() {
        let source = "\
#!/bin/sh
f() {
  local declare hard_list
  declare local other_list
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LocalDeclareCombined),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["declare", "local"]
        );
    }

    #[test]
    fn ignores_plain_declaration_commands_and_unsupported_shells() {
        let source = "\
#!/bin/sh
f() {
  local hard_list
  declare other_list
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LocalDeclareCombined),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");

        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LocalDeclareCombined).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

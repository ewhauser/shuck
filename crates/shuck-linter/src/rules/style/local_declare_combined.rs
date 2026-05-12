use shuck_ast::DeclOperand;

use crate::{
    Checker, DeclarationKind, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation,
};

pub struct LocalDeclareCombined;

impl Violation for LocalDeclareCombined {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LocalDeclareCombined
    }

    fn message(&self) -> String {
        "mix either `local` or `declare`, not both in the same statement".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete the extra declaration word".to_owned())
    }
}

pub fn local_declare_combined(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| declaration_combination_span(fact))
        .map(|span| {
            Diagnostic::new(LocalDeclareCombined, span).with_fix(Fix::unsafe_edit(Edit::deletion(
                operand_deletion_span(span, source),
            )))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
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

fn operand_deletion_span(span: shuck_ast::Span, source: &str) -> shuck_ast::Span {
    let bytes = source.as_bytes();
    let mut end_offset = span.end.offset;
    while end_offset < bytes.len() && matches!(bytes[end_offset], b' ' | b'\t') {
        end_offset += 1;
    }
    if end_offset > span.end.offset {
        return shuck_ast::Span::from_positions(
            span.start,
            span.end.advanced_by(&source[span.end.offset..end_offset]),
        );
    }

    let mut start_offset = span.start.offset;
    while start_offset > 0 && matches!(bytes[start_offset - 1], b' ' | b'\t') {
        start_offset -= 1;
    }
    shuck_ast::Span::from_positions(
        shuck_ast::Position {
            offset: start_offset,
            line: span.start.line,
            column: span
                .start
                .column
                .saturating_sub(span.start.offset.saturating_sub(start_offset)),
        },
        span.end,
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

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

    #[test]
    fn applies_unsafe_fix_by_deleting_extra_declaration_word() {
        let source = "\
#!/bin/sh
f() {
  local declare hard_list
  declare local other_list
}
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LocalDeclareCombined),
            Applicability::Unsafe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
f() {
  local hard_list
  declare other_list
}
"
        );
        assert_eq!(result.fixes_applied, 2);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_combined_declarations_unchanged() {
        let source = "\
#!/bin/sh
local declare x
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LocalDeclareCombined),
            Applicability::Safe,
        );

        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixes_applied, 0);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S066.sh").as_path(),
            &LinterSettings::for_rule(Rule::LocalDeclareCombined),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("S066_fix_S066.sh", result);
        Ok(())
    }
}

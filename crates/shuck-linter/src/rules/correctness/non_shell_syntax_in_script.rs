use shuck_ast::{Command, StmtTerminator, static_word_text};

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct NonShellSyntaxInScript;

impl Violation for NonShellSyntaxInScript {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::NonShellSyntaxInScript
    }

    fn message(&self) -> String {
        "line looks like non-shell declaration syntax".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("delete the non-shell declaration".to_owned())
    }
}

pub fn non_shell_syntax_in_script(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| non_shell_syntax_spans(command, checker.source()))
        .collect::<Vec<_>>();

    for (span, fix_span) in diagnostics {
        checker.report_diagnostic_dedup(
            Diagnostic::new(NonShellSyntaxInScript, span)
                .with_fix(Fix::unsafe_edit(Edit::deletion(fix_span))),
        );
    }
}

fn non_shell_syntax_spans(
    command: crate::CommandFactRef<'_, '_>,
    source: &str,
) -> Option<(shuck_ast::Span, shuck_ast::Span)> {
    if command.stmt().terminator != Some(StmtTerminator::Semicolon) {
        return None;
    }

    let Command::Simple(simple) = command.command() else {
        return None;
    };
    if !simple.assignments.is_empty() {
        return None;
    }

    let name = static_word_text(&simple.name, source)?;
    if !looks_like_c_declaration_keyword(name.as_ref()) || simple.args.is_empty() {
        return None;
    }

    Some((simple.name.span, command.stmt().span))
}

fn looks_like_c_declaration_keyword(text: &str) -> bool {
    matches!(
        text,
        "int"
            | "char"
            | "float"
            | "double"
            | "long"
            | "short"
            | "unsigned"
            | "signed"
            | "struct"
            | "enum"
            | "typedef"
            | "static"
            | "const"
            | "void"
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_c_style_declaration_like_lines() {
        let source = "#!/bin/sh\nint value;\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NonShellSyntaxInScript),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "int");
    }

    #[test]
    fn ignores_regular_shell_commands() {
        let source = "#!/bin/sh\necho value;\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::NonShellSyntaxInScript),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_declaration_like_statements() {
        let source = "#!/bin/sh\nint value;\necho ok\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::NonShellSyntaxInScript),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\n\necho ok\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_regular_shell_commands_unchanged_when_fixing() {
        let source = "#!/bin/sh\necho value;\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::NonShellSyntaxInScript),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C104.sh").as_path(),
            &LinterSettings::for_rule(Rule::NonShellSyntaxInScript),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C104_fix_C104.sh", result);
        Ok(())
    }
}

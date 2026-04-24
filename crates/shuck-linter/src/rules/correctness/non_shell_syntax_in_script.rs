use shuck_ast::{Command, StmtTerminator, static_word_text};

use crate::context::FileContextTag;
use crate::{Checker, Rule, Violation};

pub struct NonShellSyntaxInScript;

impl Violation for NonShellSyntaxInScript {
    fn rule() -> Rule {
        Rule::NonShellSyntaxInScript
    }

    fn message(&self) -> String {
        "line looks like non-shell declaration syntax".to_owned()
    }
}

pub fn non_shell_syntax_in_script(checker: &mut Checker) {
    if checker.file_context().has_tag(FileContextTag::PatchFile) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| non_shell_syntax_span(command, checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || NonShellSyntaxInScript);
}

fn non_shell_syntax_span(
    command: crate::CommandFactRef<'_, '_>,
    source: &str,
) -> Option<shuck_ast::Span> {
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

    Some(simple.name.span)
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

    use crate::test::test_snippet;
    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};

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
    fn ignores_patch_file_context() {
        let source = "int value;\n";
        let diagnostics = test_snippet_at_path(
            Path::new("change.patch"),
            source,
            &LinterSettings::for_rule(Rule::NonShellSyntaxInScript),
        );

        assert!(diagnostics.is_empty());
    }
}

use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct HyphenatedFunctionName;

impl Violation for HyphenatedFunctionName {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::HyphenatedFunctionName
    }

    fn message(&self) -> String {
        "function names cannot contain hyphens in `sh` scripts".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rename the function with underscores".to_owned())
    }
}

pub fn hyphenated_function_name(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .command_facts()
        .function_headers()
        .iter()
        .flat_map(|header| {
            header.function().header.entries.iter().filter_map(|entry| {
                let name = entry
                    .static_name
                    .as_ref()
                    .map(|name| name.as_str())
                    .unwrap_or_else(|| entry.word.span.slice(source));
                name.contains('-').then(|| {
                    let replacement = name.replace('-', "_");
                    let diagnostic_span = header.function_span_in_source(source);
                    Diagnostic::new(HyphenatedFunctionName, diagnostic_span).with_fix(
                        Fix::unsafe_edits(rename_function_edits(
                            checker,
                            name,
                            &replacement,
                            entry.word.span,
                            header.binding_id(),
                        )),
                    )
                })
            })
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn rename_function_edits(
    checker: &Checker<'_>,
    old_name: &str,
    new_name: &str,
    definition_span: Span,
    binding_id: Option<shuck_semantic::BindingId>,
) -> Vec<Edit> {
    let mut edits = vec![Edit::replacement(new_name, definition_span)];
    let Some(binding_id) = binding_id else {
        return edits;
    };

    for command in checker.facts().commands() {
        if command.effective_or_literal_name() != Some(old_name) {
            continue;
        }
        let Some(name_span) = command.body_word_span() else {
            continue;
        };
        if name_span == definition_span {
            continue;
        }
        if checker
            .semantic_analysis()
            .visible_function_binding_at_call(&shuck_ast::Name::from(old_name), name_span)
            == Some(binding_id)
        {
            edits.push(Edit::replacement(new_name, name_span));
        }
    }

    edits.sort_by_key(|edit| {
        (
            usize::from(edit.range().start()),
            usize::from(edit.range().end()),
        )
    });
    edits.dedup_by_key(|edit| {
        (
            usize::from(edit.range().start()),
            usize::from(edit.range().end()),
        )
    });
    edits
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet_at_path, test_snippet_at_path_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_hyphenated_function_names_in_sh() {
        let source = "\
#!/bin/sh
my-func() { :; }
function other-func { :; }
function third-func() { :; }
my-func
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::HyphenatedFunctionName),
        );

        assert_eq!(diagnostics.len(), 3);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "my-func() { :; }",
                "function other-func { :; }",
                "function third-func() { :; }"
            ]
        );
    }

    #[test]
    fn ignores_non_hyphenated_function_names() {
        let source = "\
#!/bin/sh
my_func() { :; }
function other_func { :; }
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::HyphenatedFunctionName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_posix_hyphenated_function_names() {
        let source = "\
#!/bin/sh
my-func() { :; }
termux_run_build-package() { :; }
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::HyphenatedFunctionName),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["my-func() { :; }", "termux_run_build-package() { :; }"]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_rename_function_and_call_sites() {
        let source = "\
#!/bin/sh
my-func() { :; }
my-func
other-func() { my-func; }
other-func
";
        let result = test_snippet_at_path_with_fix(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::HyphenatedFunctionName),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
my_func() { :; }
my_func
other_func() { my_func; }
other_func
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\nmy-func() { :; }\n";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::HyphenatedFunctionName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

use crate::{
    Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, ShellDialect,
    Violation, WordFactHostKind,
};

pub struct BareGlobCommandPath;

impl Violation for BareGlobCommandPath {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::BareGlobCommandPath
    }

    fn message(&self) -> String {
        "quote this wildcard command path".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the wildcard command path".to_owned())
    }
}

pub fn bare_glob_command_path(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .command_facts()
        .structural_commands()
        .filter(|fact| {
            !fact
                .command_name_word_active_glob_spans_outside_brace_expansion(source)
                .is_empty()
        })
        .filter_map(|fact| {
            let word = fact.command_name_word()?;
            let replacement = fact.command_name_word_single_double_quoted_replacement(source)?;

            Some(
                Diagnostic::new(BareGlobCommandPath, word.span)
                    .with_fix(Fix::unsafe_edit(Edit::replacement(replacement, word.span))),
            )
        })
        .collect::<Vec<_>>();
    let diagnostics = diagnostics
        .into_iter()
        .chain(
            checker
                .facts()
                .words()
                .expansion_word_facts(ExpansionContext::CommandName)
                .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
                .filter(|fact| {
                    !fact
                        .active_glob_spans_outside_brace_expansion(source)
                        .is_empty()
                })
                .map(|fact| {
                    Diagnostic::new(BareGlobCommandPath, fact.span()).with_fix(Fix::unsafe_edit(
                        Edit::replacement(
                            fact.single_double_quoted_replacement(source),
                            fact.span(),
                        ),
                    ))
                }),
        )
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_wildcards_in_command_position() {
        let source = "\
#!/bin/sh
/opt/tool-*.AppImage --help
./foo?.sh
./[ab].sh
$dir/*.sh
echo $($OUT/bin/python*-config --cflags)
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::BareGlobCommandPath));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "/opt/tool-*.AppImage",
                "./foo?.sh",
                "./[ab].sh",
                "$dir/*.sh",
                "$OUT/bin/python*-config"
            ]
        );
    }

    #[test]
    fn ignores_quoted_words_arguments_and_brace_expansion() {
        let source = "\
#!/bin/bash
\"/opt/tool-*.AppImage\" --help
cmd /opt/tool-*.AppImage
./{foo,bar}.sh
$dir/{tool*,tool}.sh
/opt/tool-star.AppImage --help
exec ./foo?.sh
command ./foo?.sh
env FOO=1 ./foo?.sh
noglob ./disabled*.sh
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::BareGlobCommandPath));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn skips_zsh() {
        let source = "#!/bin/zsh\n./foo?.sh\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BareGlobCommandPath).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_unsafe_fix_to_the_full_command_word() {
        let source = "#!/bin/sh\n/opt/tool-*.AppImage --help\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::BareGlobCommandPath),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\n\"/opt/tool-*.AppImage\" --help\n"
        );
    }
}

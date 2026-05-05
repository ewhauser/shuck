use rustc_hash::FxHashSet;

use crate::{Checker, Edit, ExpansionContext, FactSpan, Fix, FixAvailability, Rule, Violation};

pub struct UnquotedGlobsInFind;

impl Violation for UnquotedGlobsInFind {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::UnquotedGlobsInFind
    }

    fn message(&self) -> String {
        "patterns in `find -exec` arguments expand before `find` runs".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the full `find -exec` argument".to_owned())
    }
}

pub fn unquoted_globs_in_find(checker: &mut Checker) {
    let source = checker.source();
    let find_exec_argument_words = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            fact.options().find_exec().map(|find_exec| {
                find_exec
                    .argument_word_spans()
                    .iter()
                    .copied()
                    .map(move |span| (fact.id(), FactSpan::new(span)))
            })
        })
        .flatten()
        .collect::<FxHashSet<_>>();

    let diagnostics = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| find_exec_argument_words.contains(&(fact.command_id(), fact.key())))
        .flat_map(|fact| diagnostics_for_find_exec_argument(source, fact))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn diagnostics_for_find_exec_argument(
    source: &str,
    fact: crate::facts::words::WordOccurrenceRef<'_, '_>,
) -> Vec<crate::Diagnostic> {
    let word_span = fact.span();
    let replacement = fact.single_double_quoted_replacement(source);

    fact.unquoted_command_substitution_spans()
        .iter()
        .copied()
        .chain(fact.active_literal_glob_spans(source))
        .map(|span| {
            crate::Diagnostic::new(UnquotedGlobsInFind, span).with_fix(Fix::unsafe_edit(
                Edit::replacement(replacement.clone(), word_span),
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_globs_and_unquoted_substitutions_in_find_exec_arguments() {
        let source = "\
#!/bin/bash
find . -exec echo *.txt {} +
find . -exec echo foo[ab]bar {} +
find . -exec echo $(basename \"$dir\") {} +
find . -exec echo $(basename \"$dir\")* {} +
find . -execdir echo \"$prefix\"*.tmp {} \\;
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "*",
                "[ab]",
                "$(basename \"$dir\")",
                "$(basename \"$dir\")",
                "*",
                "*"
            ]
        );
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/bash\nfind . -exec echo *.txt {} +\n";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "*");
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("quote the full `find -exec` argument")
        );
    }

    #[test]
    fn ignores_quoted_or_non_find_exec_arguments() {
        let source = "\
#!/bin/bash
find . -exec echo \"$file\" {} +
find . -exec echo \"*.txt\" {} +
find . -exec echo \"$(basename \"$dir\")\" {} +
find . -name *.txt -print
printf '*.txt'
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_find_exec_arguments_inside_nested_command_substitutions() {
        let source = "\
#!/bin/bash
result=$(find . -type d -name fuzz -exec dirname $(readlink -f {}) \\;)
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(readlink -f {})"]
        );
    }

    #[test]
    fn ignores_find_path_words_and_words_after_exec_terminators() {
        let source = "\
#!/bin/bash
find \"$f\"/*.py -exec echo {} +
find $PKG/$(echo ${DOCROOT} | sed 's|/||')/$PRGNAM -type f -exec chmod 0750 {} \\;
find . -exec echo {} \\; -name *.cfg
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_globs_after_literal_plus_in_semicolon_terminated_find_exec() {
        let source = "\
#!/bin/bash
find . -exec echo + *.tmp {} \\;
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*"]
        );
    }

    #[test]
    fn reports_globs_after_quoted_backslash_semicolon_arguments() {
        let source = "\
#!/bin/bash
find . -exec echo '\\;' *.tmp {} \\;
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*"]
        );
    }

    #[test]
    fn ignores_outer_find_words_after_quoted_semicolon_terminator() {
        let source = "\
#!/bin/bash
find . -exec echo {} ';' -name *.cfg
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_outer_find_words_after_plus_terminated_exec() {
        let source = "\
#!/bin/bash
find . -exec echo {} + -name *.cfg -exec rm {} \\;
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_outer_find_words_between_plus_terminated_exec_clauses() {
        let source = "\
#!/bin/bash
find . -exec echo {} + -name *.cfg -exec rm {} +
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_globs_after_dynamic_find_exec_command_names() {
        let source = "\
#!/bin/bash
find . -exec \"$tool\" *.tmp {} \\;
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*"]
        );
    }

    #[test]
    fn reports_globs_in_later_find_exec_clauses() {
        let source = "\
#!/bin/bash
find . -exec echo {} + -exec rm *.tmp {} +
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*"]
        );
    }

    #[test]
    fn reports_expansions_in_find_exec_command_position() {
        let source = "\
#!/bin/bash
find . -exec *.tool {} +
find . -exec $(pick_cmd) {} \\;
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedGlobsInFind));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*", "$(pick_cmd)"]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_full_find_exec_argument_words() {
        let source = "\
#!/bin/bash
find . -exec echo *.txt {} +
find . -exec echo foo[ab]bar {} +
find . -exec echo $(basename \"$dir\") {} +
find . -exec echo $(basename \"$dir\")* {} +
find . -execdir echo \"$prefix\"*.tmp {} \\;
find . -exec zip -j $OUT/$(basename $dir).zip {} +
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnquotedGlobsInFind),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 6);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
find . -exec echo \"*.txt\" {} +
find . -exec echo \"foo[ab]bar\" {} +
find . -exec echo \"$(basename \"$dir\")\" {} +
find . -exec echo \"$(basename \"$dir\")*\" {} +
find . -execdir echo \"${prefix}*.tmp\" {} \\;
find . -exec zip -j \"${OUT}/$(basename $dir).zip\" {} +
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_quoted_or_non_find_exec_arguments_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
find . -exec echo \"$file\" {} +
find . -exec echo \"*.txt\" {} +
find . -exec echo \"$(basename \"$dir\")\" {} +
find . -name *.txt -print
printf '*.txt'
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnquotedGlobsInFind),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn respects_zsh_glob_activation_in_find_exec_arguments() {
        let source = "\
#!/usr/bin/env zsh
setopt no_glob
find . -exec echo *.txt {} +
setopt glob ksh_glob
find . -exec echo ?(*.txt) {} +
setopt extended_glob
find . -exec echo foo~bar {} +
find . -exec echo foo~bar* {} +
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedGlobsInFind)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["?(*.txt)", "~", "*"]
        );
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C078.sh").as_path(),
            &LinterSettings::for_rule(Rule::UnquotedGlobsInFind),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C078_fix_C078.sh", result);
        Ok(())
    }
}

use crate::{
    Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation,
    WordOccurrenceRef,
};
use shuck_ast::Span;

pub struct GlobWithExpansionInLoop;

impl Violation for GlobWithExpansionInLoop {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::GlobWithExpansionInLoop
    }

    fn message(&self) -> String {
        "quote expansion prefixes when combining them with loop globs".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the expansion prefix".to_owned())
    }
}

pub fn glob_with_expansion_in_loop(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .words()
        .expansion_word_facts(ExpansionContext::ForList)
        .filter(|fact| {
            !fact
                .active_glob_spans_outside_brace_expansion(source)
                .is_empty()
        })
        .flat_map(unquoted_expansion_prefix_spans)
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(Diagnostic::new(GlobWithExpansionInLoop, span).with_fix(
            Fix::unsafe_edits([
                Edit::insertion(span.start.offset, "\""),
                Edit::insertion(span.end.offset, "\""),
            ]),
        ));
    }
}

fn unquoted_expansion_prefix_spans(fact: WordOccurrenceRef<'_, '_>) -> Vec<Span> {
    let quoted = fact.double_quoted_expansion_spans();
    let mut spans = fact
        .scalar_expansion_spans()
        .iter()
        .copied()
        .filter(|span| !quoted.contains(span))
        .collect::<Vec<_>>();
    spans.extend(fact.unquoted_command_substitution_spans().iter().copied());
    spans
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

    #[test]
    fn reports_unquoted_expansion_prefixes_in_for_glob_words() {
        let source = "\
#!/bin/sh
for i in $CWD/file.*pattern*; do :; done
for i in ${CWD}/file.*pattern*; do :; done
for i in $(pwd)/file.*pattern*; do :; done
for i in $DIR/{1..3}*.txt; do :; done
for i in $dir/{exec,grom,ecs}.{rom,bin,int}*; do :; done
for i in $PKG/usr/man/{ja/,}*/*-8.?.?.gz; do :; done
for file in $BINARY_SAMPLES_V2/{linux,windows}/*_DWARF*/*; do :; done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobWithExpansionInLoop),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$CWD",
                "${CWD}",
                "$(pwd)",
                "$DIR",
                "$dir",
                "$PKG",
                "$BINARY_SAMPLES_V2",
            ]
        );
    }

    #[test]
    fn ignores_quoted_prefixes_and_words_without_globs() {
        let source = "\
#!/bin/sh
for i in \"$CWD\"/file.*pattern*; do :; done
for i in file.*pattern*; do :; done
for i in \"$CWD\"/*.txt; do :; done
for i in $CWD/file.txt; do :; done
for i in $DIR/setjmp-aarch64/{setjmp.S,private-*.h}; do :; done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobWithExpansionInLoop),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_zsh_glob_qualifier_words_with_unquoted_expansion_prefixes() {
        let source = "\
#!/bin/zsh
for m in ${plugin_dir}/*.zwc(.N); do :; done
for snip ( ${ZINIT[SNIPPETS_DIR]}/**/(._zinit|._zplugin)/mode(D) ) { :; }
for repo in ${ZINIT[PLUGINS_DIR]}/*; do :; done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobWithExpansionInLoop).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${plugin_dir}",
                "${ZINIT[SNIPPETS_DIR]}",
                "${ZINIT[PLUGINS_DIR]}",
            ]
        );
    }

    #[test]
    fn ignores_quoted_zsh_glob_qualifier_prefixes() {
        let source = "\
#!/bin/zsh
for m in \"${plugin_dir}\"/*.zwc(.N); do :; done
for snip ( \"${ZINIT[SNIPPETS_DIR]}\"/**/(._zinit|._zplugin)/mode(D) ) { :; }
for repo in \"${ZINIT[PLUGINS_DIR]}\"/*; do :; done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobWithExpansionInLoop).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_unsafe_fix_to_unquoted_expansion_prefixes() {
        let source = "\
#!/bin/sh
for i in $CWD/file.*pattern*; do :; done
for i in ${CWD}/file.*pattern*; do :; done
for i in $(pwd)/file.*pattern*; do :; done
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobWithExpansionInLoop),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
for i in \"$CWD\"/file.*pattern*; do :; done
for i in \"${CWD}\"/file.*pattern*; do :; done
for i in \"$(pwd)\"/file.*pattern*; do :; done
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_unquoted_expansion_prefixes_unchanged() {
        let source = "#!/bin/sh\nfor i in $CWD/file.*pattern*; do :; done\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobWithExpansionInLoop),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C114.sh").as_path(),
            &LinterSettings::for_rule(Rule::GlobWithExpansionInLoop),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C114_fix_C114.sh", result);
        Ok(())
    }
}

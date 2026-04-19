use rustc_hash::FxHashSet;

use crate::{
    Checker, ExpansionContext, FactSpan, Rule, Violation, word_unquoted_glob_pattern_spans,
};

pub struct UnquotedGlobsInFind;

impl Violation for UnquotedGlobsInFind {
    fn rule() -> Rule {
        Rule::UnquotedGlobsInFind
    }

    fn message(&self) -> String {
        "patterns in `find -exec` arguments expand before `find` runs".to_owned()
    }
}

pub fn unquoted_globs_in_find(checker: &mut Checker) {
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

    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| find_exec_argument_words.contains(&(fact.command_id(), fact.key())))
        .flat_map(|fact| {
            fact.unquoted_command_substitution_spans()
                .iter()
                .copied()
                .chain(word_unquoted_glob_pattern_spans(
                    fact.word(),
                    checker.source(),
                ))
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedGlobsInFind);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}

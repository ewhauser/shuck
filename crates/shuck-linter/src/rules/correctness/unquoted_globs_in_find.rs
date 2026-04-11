use rustc_hash::FxHashSet;

use crate::{
    Checker, ExpansionContext, Rule, Violation, WrapperKind, word_unquoted_glob_pattern_spans,
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
    let find_exec_command_ids = checker
        .facts()
        .structural_commands()
        .filter(|fact| {
            fact.has_wrapper(WrapperKind::FindExec) || fact.has_wrapper(WrapperKind::FindExecDir)
        })
        .map(|fact| fact.id())
        .collect::<FxHashSet<_>>();

    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| find_exec_command_ids.contains(&fact.command_id()))
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
}

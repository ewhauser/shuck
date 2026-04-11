use crate::{
    Checker, ExpansionContext, Rule, Violation, WordFactHostKind, WordQuote,
    word_unquoted_glob_pattern_spans,
};

pub struct GlobAssignedToVariable;

impl Violation for GlobAssignedToVariable {
    fn rule() -> Rule {
        Rule::GlobAssignedToVariable
    }

    fn message(&self) -> String {
        "quote assigned glob patterns so they stay literal".to_owned()
    }
}

pub fn glob_assigned_to_variable(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::AssignmentValue)
        .chain(
            checker
                .facts()
                .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue),
        )
        .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
        .filter(|fact| !checker.facts().is_compound_assignment_value_word(fact))
        .filter(|fact| fact.classification().quote != WordQuote::FullyQuoted)
        .filter(|fact| !word_unquoted_glob_pattern_spans(fact.word(), source).is_empty())
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GlobAssignedToVariable);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_assignment_values_with_unquoted_globs() {
        let source = "\
#!/bin/bash
LOCS=*.oxt
LOCS=$dir/*.oxt
LOCS=\"$dir\"/*.oxt
LOCS=${dir}/*.oxt
LOCS=$(pwd)/*.oxt
readonly LOCS=foo*bar
export LOCS=$dir/*.txt
LOCS=${disk_info[${#disk_info[@]} - 1]/*\\/}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobAssignedToVariable),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "*.oxt",
                "$dir/*.oxt",
                "\"$dir\"/*.oxt",
                "${dir}/*.oxt",
                "$(pwd)/*.oxt",
                "foo*bar",
                "$dir/*.txt"
            ]
        );
    }

    #[test]
    fn ignores_fully_quoted_or_non_glob_assignment_values() {
        let source = "\
#!/bin/bash
LOCS=\"*.oxt\"
LOCS='*.oxt'
LOCS=\\*.oxt
LOCS=$dir/file.txt
readonly LOCS=\"*.txt\"
export LOCS='$dir/*.txt'
LOCS=(*.oxt)
LOCS=(\"$dir\"/*.oxt)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobAssignedToVariable),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

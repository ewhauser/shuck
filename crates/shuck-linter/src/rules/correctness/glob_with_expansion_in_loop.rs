use crate::{
    Checker, ExpansionContext, Rule, Violation, WordFact, word_has_unquoted_brace_expansion,
    word_unquoted_glob_pattern_spans,
};
use shuck_ast::Span;

pub struct GlobWithExpansionInLoop;

impl Violation for GlobWithExpansionInLoop {
    fn rule() -> Rule {
        Rule::GlobWithExpansionInLoop
    }

    fn message(&self) -> String {
        "quote expansion prefixes when combining them with loop globs".to_owned()
    }
}

pub fn glob_with_expansion_in_loop(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::ForList)
        .filter(|fact| !word_has_unquoted_brace_expansion(fact.word(), source))
        .filter(|fact| !word_unquoted_glob_pattern_spans(fact.word(), source).is_empty())
        .flat_map(unquoted_expansion_prefix_spans)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GlobWithExpansionInLoop);
}

fn unquoted_expansion_prefix_spans(fact: &WordFact<'_>) -> Vec<Span> {
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
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_expansion_prefixes_in_for_glob_words() {
        let source = "\
#!/bin/sh
for i in $CWD/file.*pattern*; do :; done
for i in ${CWD}/file.*pattern*; do :; done
for i in $(pwd)/file.*pattern*; do :; done
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
            vec!["$CWD", "${CWD}", "$(pwd)"]
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
}

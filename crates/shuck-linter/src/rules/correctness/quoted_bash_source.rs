use crate::{Checker, Rule, Violation, WordFactContext};

pub struct QuotedBashSource;

impl Violation for QuotedBashSource {
    fn rule() -> Rule {
        Rule::QuotedBashSource
    }

    fn message(&self) -> String {
        "quoted BASH_SOURCE expansions should use an explicit array index".to_owned()
    }
}

pub fn quoted_bash_source(checker: &mut Checker) {
    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .filter(|fact| matches!(fact.context(), WordFactContext::Expansion(_)))
        .filter_map(|fact| fact.quoted_unindexed_bash_source_span_in_source(checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedBashSource);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_unindexed_bash_source_expansions() {
        let source = "\
#!/bin/bash
x=\"$BASH_SOURCE\"
y=\"${BASH_SOURCE}\"
printf '%s\\n' \"$BASH_SOURCE\" \"${BASH_SOURCE}\"
source \"$(dirname \"$BASH_SOURCE\")/helper.bash\"
if [[ \"$BASH_SOURCE\" == foo ]]; then :; fi
for item in \"$BASH_SOURCE\"; do
  :
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$BASH_SOURCE",
                "${BASH_SOURCE}",
                "$BASH_SOURCE",
                "${BASH_SOURCE}",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
            ]
        );
    }

    #[test]
    fn ignores_unquoted_indexed_and_non_access_forms() {
        let source = "\
#!/bin/bash
x=$BASH_SOURCE
y=${BASH_SOURCE}
z=\"${BASH_SOURCE[0]}\"
q=\"${BASH_SOURCE[@]}\"
r=\"${BASH_SOURCE[*]}\"
s=\"${BASH_SOURCE%/*}\"
t=\"${BASH_SOURCE:-fallback}\"
u=\"\\$BASH_SOURCE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }
}

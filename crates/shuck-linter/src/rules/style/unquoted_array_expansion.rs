use crate::rules::common::expansion::analyze_word;
use crate::rules::common::span;
use crate::rules::common::{
    expansion::ExpansionContext,
    query::{self, CommandWalkOptions},
};
use crate::{Checker, Rule, Violation};

pub struct UnquotedArrayExpansion;

impl Violation for UnquotedArrayExpansion {
    fn rule() -> Rule {
        Rule::UnquotedArrayExpansion
    }

    fn message(&self) -> String {
        "quote array expansions to preserve element boundaries".to_owned()
    }
}

pub fn unquoted_array_expansion(checker: &mut Checker) {
    let source = checker.source();

    query::walk_commands(
        &checker.ast().commands,
        checker.source(),
        CommandWalkOptions {
            descend_nested_word_commands: false,
        },
        &mut |command, _| {
            query::visit_expansion_words(command, source, &mut |word, context| {
                if context != ExpansionContext::CommandArgument {
                    return;
                }

                let analysis = analyze_word(word, source);
                if analysis.array_valued && analysis.can_expand_to_multiple_fields {
                    for span in span::unquoted_array_expansion_part_spans(word, source) {
                        checker.report_dedup(UnquotedArrayExpansion, span);
                    }
                }
            });
        },
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_inner_array_expansion_spans() {
        let source = "\
#!/bin/bash
printf '%s\\n' prefix${arr[@]}suffix ${arr[0]} ${names[*]}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${arr[@]}", "${names[*]}"]
        );
    }

    #[test]
    fn ignores_non_argument_array_contexts() {
        let source = "\
#!/bin/bash
arr=(a b)
printf '%s\\n' ok >${paths[@]}
cat <<< ${items[@]}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_array_values_that_stay_single_field_when_quoted() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"${names[*]}\" \"${arr[0]}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedArrayExpansion),
        );

        assert!(diagnostics.is_empty());
    }
}

use crate::rules::common::expansion::ExpansionContext;
use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

pub struct PatternWithVariable;

impl Violation for PatternWithVariable {
    fn rule() -> Rule {
        Rule::PatternWithVariable
    }

    fn message(&self) -> String {
        "pattern expressions should not expand variables".to_owned()
    }
}

pub fn pattern_with_variable(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            query::visit_expansion_words(command, source, &mut |word, context| {
                if context == ExpansionContext::ParameterPattern && classify_word(word).is_expanded()
                {
                    spans.push(word.span);
                }
            });
        },
    );

    for span in spans {
        checker.report(PatternWithVariable, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_nested_parameter_pattern_groups_and_substitutions() {
        let source = "\
#!/bin/bash
suffix=bc
trimmed=${name%@($suffix|$(printf '%s' zz))}
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::PatternWithVariable));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$suffix", "$(printf '%s' zz)"]
        );
    }
}

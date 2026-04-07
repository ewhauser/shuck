use crate::rules::common::expansion::ExpansionContext;
use crate::rules::common::query::{self, CommandWalkOptions};
use crate::rules::common::word::classify_word;
use crate::{Checker, Rule, Violation};

pub struct CasePatternVar;

impl Violation for CasePatternVar {
    fn rule() -> Rule {
        Rule::CasePatternVar
    }

    fn message(&self) -> String {
        "case patterns should be literal instead of built from expansions".to_owned()
    }
}

pub fn case_pattern_var(checker: &mut Checker) {
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            query::visit_expansion_words(command, checker.source(), &mut |word, context| {
                if context == ExpansionContext::CasePattern
                    && classify_word(word, checker.source()).is_expanded()
                {
                    spans.push(word.span);
                }
            });
        },
    );

    for span in spans {
        checker.report(CasePatternVar, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_nested_case_pattern_groups_and_substitutions() {
        let source = "\
#!/bin/bash
pat=foo
case $value in
  @($pat|$(printf '%s' bar))) : ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CasePatternVar));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$pat", "$(printf '%s' bar)"]
        );
    }
    #[test]
    fn ignores_case_patterns_built_from_quoted_literal_fragments() {
        let source = "#!/bin/bash\ncase $value in foo\"bar\"'baz') : ;; esac\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CasePatternVar));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_literal_case_patterns_with_glob_and_brace_syntax() {
        let source = "\
#!/bin/bash
case $value in
  *.sh|{a,b}) : ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CasePatternVar));

        assert!(diagnostics.is_empty());
    }
}

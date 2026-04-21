use shuck_ast::{ConditionalUnaryOp, Span, Word};

use crate::{
    Checker, ConditionalNodeFact, ConditionalOperatorFamily, Edit, Fix, FixAvailability, Rule,
    Violation, static_word_text,
};

pub struct TruthyLiteralTest;

impl Violation for TruthyLiteralTest {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::TruthyLiteralTest
    }

    fn message(&self) -> String {
        "this test checks a fixed literal instead of runtime data".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace the bare test term with an explicit true or false literal".to_owned())
    }
}

pub fn truthy_literal_test(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            let mut diagnostics = Vec::new();
            if let Some(simple_test) = fact.simple_test() {
                diagnostics.extend(simple_test_diagnostics(simple_test, source));
            }
            if let Some(conditional) = fact.conditional() {
                diagnostics.extend(conditional_diagnostics(conditional, source));
            }
            diagnostics
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn simple_test_diagnostics(
    fact: &crate::SimpleTestFact<'_>,
    source: &str,
) -> Vec<crate::Diagnostic> {
    fact.truthy_expression_words(source)
        .into_iter()
        .filter_map(|word| simple_test_diagnostic(fact, word, source))
        .collect()
}

fn simple_test_diagnostic(
    fact: &crate::SimpleTestFact<'_>,
    word: &Word,
    source: &str,
) -> Option<crate::Diagnostic> {
    if truthy_word_is_explicit_boolean_literal(word, source) {
        return None;
    }

    let index = fact
        .effective_operands()
        .iter()
        .position(|operand| operand.span == word.span)?;
    fact.effective_operand_class(index)
        .filter(|class| class.is_fixed_literal())?;

    diagnostic_for_truthy_literal(
        word.span,
        explicit_literal_replacement_for_word(word, source),
    )
}

fn conditional_diagnostics(
    fact: &crate::ConditionalFact<'_>,
    source: &str,
) -> Vec<crate::Diagnostic> {
    let excluded_operand_spans = fact
        .nodes()
        .iter()
        .flat_map(|node| match node {
            ConditionalNodeFact::Unary(unary) if unary.op() != ConditionalUnaryOp::Not => unary
                .operand()
                .word()
                .map(|word| vec![word.span])
                .unwrap_or_default(),
            ConditionalNodeFact::Binary(binary)
                if binary.operator_family() != ConditionalOperatorFamily::Logical =>
            {
                [binary.left().word(), binary.right().word()]
                    .into_iter()
                    .flatten()
                    .map(|word| word.span)
                    .collect::<Vec<_>>()
            }
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Unary(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => Vec::new(),
        })
        .collect::<Vec<_>>();

    fact.nodes()
        .iter()
        .filter_map(|node| match node {
            ConditionalNodeFact::BareWord(word)
                if word.operand().class().is_fixed_literal()
                    && word
                        .operand()
                        .word()
                        .is_some_and(|operand| !excluded_operand_spans.contains(&operand.span)) =>
            {
                conditional_diagnostic(word.operand(), source)
            }
            _ => None,
        })
        .collect()
}

fn conditional_diagnostic(
    operand: crate::ConditionalOperandFact<'_>,
    source: &str,
) -> Option<crate::Diagnostic> {
    let word = operand.word()?;
    if truthy_word_is_explicit_boolean_literal(word, source) {
        return None;
    }

    diagnostic_for_truthy_literal(
        word.span,
        explicit_literal_replacement_for_word(word, source),
    )
}

fn diagnostic_for_truthy_literal(
    span: Span,
    replacement: &'static str,
) -> Option<crate::Diagnostic> {
    Some(
        crate::Diagnostic::new(TruthyLiteralTest, span)
            .with_fix(Fix::unsafe_edit(Edit::replacement(replacement, span))),
    )
}

const EMPTY_LITERAL_REPLACEMENT: &str = "\"\"";
const NON_EMPTY_LITERAL_REPLACEMENT: &str = "x";

fn explicit_literal_replacement_for_word(word: &Word, source: &str) -> &'static str {
    explicit_literal_replacement_for_text(
        static_word_text(word, source)
            .as_deref()
            .unwrap_or_else(|| word.span.slice(source)),
    )
}

fn explicit_literal_replacement_for_text(text: &str) -> &'static str {
    if text.is_empty() || quoted_literal_body(text).is_some_and(str::is_empty) {
        EMPTY_LITERAL_REPLACEMENT
    } else {
        NON_EMPTY_LITERAL_REPLACEMENT
    }
}

fn quoted_literal_body(text: &str) -> Option<&str> {
    let quote = text.chars().next()?;
    if !matches!(quote, '"' | '\'') {
        return None;
    }

    text.strip_prefix(quote)?.strip_suffix(quote)
}

fn truthy_word_is_explicit_boolean_literal(word: &Word, source: &str) -> bool {
    matches!(word.span.slice(source), "\"\"" | "x")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn ignores_runtime_sensitive_literal_words() {
        let source = "\
#!/bin/bash
[ ~ ]
test ~user
test x=~
test *.sh
[ {a,b} ]
[[ ~ ]]
[[ *.sh ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![8]
        );
    }

    #[test]
    fn still_reports_plain_fixed_literals() {
        let source = "\
#!/bin/bash
[ 1 ]
test foo
[[ bar ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );
    }

    #[test]
    fn anchors_truthy_simple_tests_on_the_operand() {
        let source = "\
#!/bin/bash
[ '' ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "''");
    }

    #[test]
    fn reports_truthy_terms_inside_simple_test_logical_chains() {
        let source = "\
#!/bin/sh
[ \"$mode\" = yes -o foo ]
[ bar -a \"$mode\" = yes ]
[ \"$mode\" = yes -o \"$other\" = no ]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo", "bar"]
        );
    }

    #[test]
    fn reports_truthy_literals_inside_negated_and_logical_conditionals() {
        let source = "\
#!/bin/bash
[[ ! foo ]]
[[ \"$mode\" == yes || bar ]]
[[ ! -n baz ]]
[[ foo == bar ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo", "bar"]
        );
    }

    #[test]
    fn ignores_tab_stripped_heredoc_substitutions_after_earlier_heredocs() {
        let source = "\
#!/bin/bash
case \"${tag_type}\" in
  newest-tag)
\t:
\t;;
  latest-release-tag)
\t:
\t;;
  latest-regex)
\t:
\t;;
  *)
\ttermux_error_exit <<-EndOfError
\t\tERROR: Invalid TERMUX_PKG_UPDATE_TAG_TYPE: '${tag_type}'.
\t\tAllowed values: 'newest-tag', 'latest-release-tag', 'latest-regex'.
\tEndOfError
\t;;
esac

case \"${http_code}\" in
  404)
\ttermux_error_exit <<-EndOfError
\t\tNo '${tag_type}' found. (${api_url})
\t\tHTTP code: ${http_code}
\t\tTry using '$(
\t\t\tif [[ \"${tag_type}\" == \"newest-tag\" ]]; then
\t\t\t\techo \"latest-release-tag\"
\t\t\telse
\t\t\t\techo \"newest-tag\"
\t\t\tfi
\t\t)'.
\tEndOfError
\t;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert!(
            diagnostics.is_empty(),
            "unexpected diagnostics: {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| (
                    diagnostic.span.start.line,
                    diagnostic.span.slice(source).to_owned(),
                ))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ignores_explicit_boolean_literals() {
        let source = "\
#!/bin/bash
[ x ]
test \"\"
[[ x ]]
[[ \"\" ]]
[ ! x ]
[[ ! \"\" ]]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/bash\n[ 1 ]\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TruthyLiteralTest));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("replace the bare test term with an explicit true or false literal")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_truthy_literal_tests() {
        let source = "\
#!/bin/bash
[ 1 ]
test ''
[[ ! bar ]]
[[ foo || \"\" ]]
[ \"$value\" ]
[[ $value ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::TruthyLiteralTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 4);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ x ]
test \"\"
[[ ! x ]]
[[ x || \"\" ]]
[ \"$value\" ]
[[ $value ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_explicit_boolean_literals_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
[ x ]
test \"\"
[[ x ]]
[[ \"\" ]]
[ ! x ]
[[ ! \"\" ]]
[ \"$value\" ]
[[ $value ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::TruthyLiteralTest),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C020.sh").as_path(),
            &LinterSettings::for_rule(Rule::TruthyLiteralTest),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C020_fix_C020.sh", result);
        Ok(())
    }
}

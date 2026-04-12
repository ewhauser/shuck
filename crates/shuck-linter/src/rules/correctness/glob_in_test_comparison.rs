use crate::{Checker, Rule, SimpleTestShape, SimpleTestSyntax, Violation, static_word_text};

pub struct GlobInTestComparison;

impl Violation for GlobInTestComparison {
    fn rule() -> Rule {
        Rule::GlobInTestComparison
    }

    fn message(&self) -> String {
        "glob matching on the right-hand side of `[ ... ]` won't work here".to_owned()
    }
}

pub fn glob_in_test_comparison(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| command.simple_test())
        .filter_map(|simple_test| report_span(simple_test, checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GlobInTestComparison);
}

fn report_span(simple_test: &crate::SimpleTestFact<'_>, source: &str) -> Option<shuck_ast::Span> {
    if simple_test.syntax() != SimpleTestSyntax::Bracket
        || simple_test.effective_shape() != SimpleTestShape::Binary
    {
        return None;
    }

    let operator = static_word_text(simple_test.effective_operands().get(1)?, source)?;
    if !matches!(operator.as_str(), "=" | "==" | "!=") {
        return None;
    }

    let rhs = *simple_test.effective_operands().get(2)?;
    let rhs_class = simple_test.effective_operand_class(2)?;
    let rhs_text = static_word_text(rhs, source)?;

    (!rhs_class.is_fixed_literal() && looks_like_glob_pattern(&rhs_text)).then_some(rhs.span)
}

fn looks_like_glob_pattern(text: &str) -> bool {
    if text.chars().any(|ch| matches!(ch, '*' | '?')) {
        return true;
    }

    text.char_indices().any(|(start, ch)| {
        ch == '['
            && text[start + ch.len_utf8()..]
                .chars()
                .any(|candidate| candidate == ']')
    })
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_globs_in_bracket_string_comparisons() {
        let source = "\
#!/bin/bash
[ \"$ARCH\" == i?86 ]
[ \"$ARCH\" = *.x86 ]
[ \"$ARCH\" != [[:digit:]] ]
[ ! = i?86 ]
[ ! \"$ARCH\" == i?86 ]
[ ! \"$ARCH\" = *.x86 ]
[ ! \"$ARCH\" != [[:digit:]] ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInTestComparison),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "i?86",
                "*.x86",
                "[[:digit:]]",
                "i?86",
                "i?86",
                "*.x86",
                "[[:digit:]]"
            ]
        );
    }

    #[test]
    fn ignores_quoted_escaped_and_non_bracket_comparisons() {
        let source = "\
#!/bin/bash
[ \"$ARCH\" == \"i?86\" ]
[ \"$ARCH\" == i\\?86 ]
[ \"$ARCH\" == foo ]
test \"$ARCH\" == i?86
[[ \"$ARCH\" == i?86 ]]
[ \"$ARCH\" < i?86 ]
[ ! \"$ARCH\" == \"i?86\" ]
[ ! \"$ARCH\" == i\\?86 ]
[ ! \"$ARCH\" == foo ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInTestComparison),
        );

        assert!(diagnostics.is_empty());
    }
}

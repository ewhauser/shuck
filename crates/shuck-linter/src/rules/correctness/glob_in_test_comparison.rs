use shuck_ast::static_word_text;

use crate::{
    Checker, Diagnostic, Edit, Fix, FixAvailability, LinterFacts, Rule, SimpleTestShape,
    SimpleTestSyntax, Violation,
};

pub struct GlobInTestComparison;

impl Violation for GlobInTestComparison {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::GlobInTestComparison
    }

    fn message(&self) -> String {
        "glob matching on the right-hand side of `[ ... ]` won't work here".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the literal glob operand".to_owned())
    }
}

pub fn glob_in_test_comparison(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| command.simple_test())
        .filter_map(|simple_test| diagnostic_fix(simple_test, checker.facts(), checker.source()))
        .collect::<Vec<_>>();

    for (span, replacement) in diagnostics {
        checker.report_diagnostic_dedup(
            Diagnostic::new(GlobInTestComparison, span)
                .with_fix(Fix::unsafe_edit(Edit::replacement(replacement, span))),
        );
    }
}

fn diagnostic_fix(
    simple_test: &crate::SimpleTestFact<'_>,
    facts: &LinterFacts<'_>,
    source: &str,
) -> Option<(shuck_ast::Span, String)> {
    if simple_test.syntax() != SimpleTestSyntax::Bracket
        || simple_test.effective_shape() != SimpleTestShape::Binary
    {
        return None;
    }

    let operator = static_word_text(simple_test.effective_operands().get(1)?, source)?;
    if !matches!(operator.as_ref(), "=" | "==" | "!=") {
        return None;
    }

    let rhs = *simple_test.effective_operands().get(2)?;
    let rhs_class = simple_test.effective_operand_class(2)?;

    if rhs_class.is_fixed_literal()
        || facts
            .words()
            .any_word_fact(rhs.span)
            .is_none_or(|fact| fact.active_literal_glob_spans(source).is_empty())
    {
        return None;
    }

    Some((rhs.span, double_quoted_replacement(rhs.span.slice(source))))
}

fn double_quoted_replacement(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn respects_zsh_disabled_globbing_in_bracket_comparisons() {
        let source = "\
#!/usr/bin/env zsh
setopt no_glob
[ \"$ARCH\" == i?86 ]
setopt glob
[ \"$ARCH\" == i?86 ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInTestComparison)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["i?86"]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_unquoted_globs_in_bracket_comparisons() {
        let source = "\
#!/bin/bash
[ \"$ARCH\" == i?86 ]
[ \"$ARCH\" = *.x86 ]
[ \"$ARCH\" != [[:digit:]] ]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInTestComparison),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
[ \"$ARCH\" == \"i?86\" ]
[ \"$ARCH\" = \"*.x86\" ]
[ \"$ARCH\" != \"[[:digit:]]\" ]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_already_literal_operands_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
[ \"$ARCH\" == \"i?86\" ]
[ \"$ARCH\" == i\\?86 ]
[[ \"$ARCH\" == i?86 ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInTestComparison),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C090.sh").as_path(),
            &LinterSettings::for_rule(Rule::GlobInTestComparison),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C090_fix_C090.sh", result);
        Ok(())
    }
}

use shuck_ast::{word_is_standalone_variable_like, word_is_standalone_zsh_force_glob_parameter};

use super::pattern_policy;
use crate::{
    Checker, ConditionalNodeFact, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation,
    WordQuote, conditional_binary_op_is_string_match,
};

pub struct GlobInStringComparison;

impl Violation for GlobInStringComparison {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::GlobInStringComparison
    }

    fn message(&self) -> String {
        "quote the right-hand side so string comparisons do not turn into glob matches".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("wrap the right-hand operand in double quotes".to_owned())
    }
}

pub fn glob_in_string_comparison(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| conditional.nodes())
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary)
                if conditional_binary_op_is_string_match(binary.op()) =>
            {
                Some(binary)
            }
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Unary(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => None,
        })
        .filter_map(|binary| {
            let right = binary.right();
            if right.quote() != Some(WordQuote::Unquoted) {
                return None;
            }

            let word = right.word()?;
            if word_is_standalone_zsh_force_glob_parameter(word) {
                return None;
            }
            if pattern_policy::word_expands_only_static_pattern_safe_literals(checker, word) {
                return None;
            }

            word_is_standalone_variable_like(word).then_some(word.span)
        })
        .map(|span| {
            Diagnostic::new(GlobInStringComparison, span).with_fix(Fix::unsafe_edit(
                Edit::replacement(format!("\"{}\"", span.slice(source)), span),
            ))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::test_snippet;
    use crate::test::{test_path_with_fix, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_unquoted_standalone_variable_patterns() {
        let source = "\
#!/bin/bash
if [[ $mirror == $pkgs ]]; then echo same; fi
if [[ \"$a\" = $1 ]]; then :; fi
if [[ \"$a\" != ${b%%x} ]]; then :; fi
if [[ \"$a\" == ${arr[0]} ]]; then :; fi
if [[ \"$a\" == \"$b\" ]]; then :; fi
if [[ \"$a\" == $b* ]]; then :; fi
if [[ \"$a\" == $b$c ]]; then :; fi
if [[ \"$a\" == ${b}_x ]]; then :; fi
if [[ \"$a\" < $b ]]; then :; fi
if [ \"$a\" = $b ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$pkgs", "$1", "${b%%x}", "${arr[0]}"]
        );
    }

    #[test]
    fn skips_zsh_explicit_pattern_expansion_rhs() {
        let source = "\
#!/usr/bin/env zsh
pattern='f*'
if [[ foo == ${~pattern} ]]; then :; fi
if [[ foo = ${=~pattern} ]]; then :; fi
if [[ foo != ${~~~pattern} ]]; then :; fi
if [[ foo == ${~~literal} ]]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${~~literal}"]
        );
    }

    #[test]
    fn ignores_rhs_variables_with_static_pattern_safe_values() {
        let source = "\
#!/usr/bin/env zsh
expected=ready
pattern='r*'
if [[ $actual == $expected ]]; then :; fi
if [[ $actual == $pattern ]]; then :; fi
if [[ $actual == $unknown ]]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$pattern", "$unknown"]
        );
    }

    #[test]
    fn reports_rhs_parameter_operations_even_when_target_bindings_are_static_safe() {
        let source = "\
#!/usr/bin/env zsh
expected=ready
if [[ $actual == ${expected//ready/*} ]]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${expected//ready/*}"]
        );
    }

    #[test]
    fn reports_nested_string_comparisons_inside_command_substitutions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"$( [[ $mirror == $pkgs ]] && echo same )\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$pkgs");
    }

    #[test]
    fn attaches_unsafe_fix_metadata_for_variable_like_rhs() {
        let source = "#!/bin/bash\nif [[ $mirror == $pkgs ]]; then echo same; fi\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("wrap the right-hand operand in double quotes")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_variable_like_rhs_operands() {
        let source = "\
#!/bin/bash
if [[ $mirror == $pkgs ]]; then echo same; fi
if [[ \"$a\" = $1 ]]; then :; fi
if [[ \"$a\" != ${b%%x} ]]; then :; fi
if [[ \"$a\" == ${arr[0]} ]]; then :; fi
printf '%s\\n' \"$( [[ $mirror == $pkgs ]] && echo same )\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 5);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
if [[ $mirror == \"$pkgs\" ]]; then echo same; fi
if [[ \"$a\" = \"$1\" ]]; then :; fi
if [[ \"$a\" != \"${b%%x}\" ]]; then :; fi
if [[ \"$a\" == \"${arr[0]}\" ]]; then :; fi
printf '%s\\n' \"$( [[ $mirror == \"$pkgs\" ]] && echo same )\"
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_variable_like_rhs_operands_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
if [[ \"$a\" == \"$b\" ]]; then :; fi
if [[ \"$a\" == $b* ]]; then :; fi
if [ \"$a\" = $b ]; then :; fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C081.sh").as_path(),
            &LinterSettings::for_rule(Rule::GlobInStringComparison),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C081_fix_C081.sh", result);
        Ok(())
    }
}

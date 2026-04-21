use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct CasePatternVar;

impl Violation for CasePatternVar {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::CasePatternVar
    }

    fn message(&self) -> String {
        "case patterns should be literal instead of built from expansions".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite the pattern as one double-quoted word".to_owned())
    }
}

pub fn case_pattern_var(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .case_pattern_expansions()
        .iter()
        .map(|fact| {
            crate::Diagnostic::new(CasePatternVar, fact.span()).with_fix(Fix::unsafe_edit(
                Edit::replacement(fact.replacement(), fact.span()),
            ))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, assert_diagnostics_diff};

    #[test]
    fn reports_simple_case_pattern_expansions() {
        let source = "\
#!/bin/bash
pat=foo
case $value in
  $pat) : ;;
  $(printf '%s' bar)) : ;;
  \"$left\"$right) : ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CasePatternVar));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$pat", "$(printf '%s' bar)", "\"$left\"$right"]
        );
    }

    #[test]
    fn attaches_unsafe_fix_metadata() {
        let source = "#!/bin/sh\ncase $value in\n  $pat) : ;;\nesac\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CasePatternVar));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Unsafe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("rewrite the pattern as one double-quoted word")
        );
    }

    #[test]
    fn applies_unsafe_fix_to_dynamic_case_patterns() {
        let source = "\
#!/bin/sh
case $value in
  $pat) : ;;
  x$pat) : ;;
  $(printf '%s' bar)) : ;;
  \"$left\"$right) : ;;
esac
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CasePatternVar),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 4);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
case $value in
  \"${pat}\") : ;;
  \"x${pat}\") : ;;
  \"$(printf '%s' bar)\") : ;;
  \"${left}${right}\") : ;;
esac
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C048.sh").as_path(),
            &LinterSettings::for_rule(Rule::CasePatternVar),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C048_fix_C048.sh", result);
        Ok(())
    }

    #[test]
    fn ignores_case_patterns_built_from_quoted_literal_fragments() {
        let source = "#!/bin/bash\ncase $value in foo\"bar\"'baz') : ;; esac\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CasePatternVar));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_fully_quoted_case_pattern_expansions() {
        let source = "\
#!/bin/sh
case $value in
  \"$pat\") : ;;
  x\"$quoted\") : ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CasePatternVar));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_case_patterns_with_real_glob_structure() {
        let source = "\
#!/bin/sh
case $value in
  gm$MAMEVER*) : ;;
  *${IDN_ITEM}*) : ;;
  ${pat}*) : ;;
  *${pat}) : ;;
  x${pat}*) : ;;
  [$hex]) : ;;
  @($pat|bar)) : ;;
  x$left@(foo|bar)) : ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CasePatternVar));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_case_patterns_built_from_arithmetic_expansions() {
        let source = "\
#!/bin/bash
case $value in
  $((error_code <= 125))) : ;;
  $((__git_cmd_idx+1))) : ;;
  x$((1))) : ;;
esac
";
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

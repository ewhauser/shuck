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
    checker.report_all(
        checker.facts().case_pattern_expansion_spans().to_vec(),
        || CasePatternVar,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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

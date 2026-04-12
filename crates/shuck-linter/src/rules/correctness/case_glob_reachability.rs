use crate::{Checker, Rule, Violation};

pub struct CaseGlobReachability;

impl Violation for CaseGlobReachability {
    fn rule() -> Rule {
        Rule::CaseGlobReachability
    }

    fn message(&self) -> String {
        "this case pattern shadows a later pattern".to_owned()
    }
}

pub fn case_glob_reachability(checker: &mut Checker) {
    let spans = checker
        .facts()
        .case_pattern_shadows()
        .iter()
        .map(|shadow| shadow.shadowing_pattern_span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CaseGlobReachability);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_shadowing_patterns_in_same_arm_and_later_arms() {
        let source = "\
#!/bin/sh
case \"$x\" in
  *|foo*) : ;;
  foo) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["*", "foo*"]
        );
    }

    #[test]
    fn ignores_unanalyzed_patterns_and_continue_matching_arms() {
        let source = "\
#!/bin/bash
shopt -s extglob
case \"$x\" in
  [ab]*) : ;;
  afoo) : ;;
esac
case \"$x\" in
  @(foo|bar)*) : ;;
  fooz) : ;;
esac
case \"$x\" in
  foo) : ;;&
  foo) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_static_quoted_pattern_fragments() {
        let source = "\
#!/bin/sh
case \"$x\" in
  foo\"bar\"*) : ;;
  foobarz) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "foo\"bar\"*");
    }

    #[test]
    fn ignores_escaped_wildcards_that_are_meant_to_be_literal() {
        let source = "\
#!/bin/sh
case \"$x\" in
  \\?) : ;;
  :) : ;;
  *) : ;;
esac
case \"$x\" in
  lts/\\*) : ;;
  lts/*) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseGlobReachability),
        );

        assert!(diagnostics.is_empty());
    }
}

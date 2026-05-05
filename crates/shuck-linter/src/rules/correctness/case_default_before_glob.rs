use crate::{Checker, Rule, Violation};

pub struct CaseDefaultBeforeGlob;

impl Violation for CaseDefaultBeforeGlob {
    fn rule() -> Rule {
        Rule::CaseDefaultBeforeGlob
    }

    fn message(&self) -> String {
        "this case pattern is unreachable because an earlier pattern already matches it".to_owned()
    }
}

pub fn case_default_before_glob(checker: &mut Checker) {
    let spans = checker
        .facts()
        .case_pattern_shadows()
        .iter()
        .map(|shadow| shadow.shadowed_pattern_span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CaseDefaultBeforeGlob);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_shadowed_patterns_in_same_arm_and_later_arms() {
        let source = "\
#!/bin/sh
case \"$x\" in
  *|foo*) : ;;
  foo) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foo*", "foo"]
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
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_extended_glob_patterns_when_reachability_is_uncertain() {
        let source = "\
#!/usr/bin/env zsh
setopt extended_glob
case \"$x\" in
  foo^bar) : ;;
  fooz) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_literals_shadowed_by_suffix_globs() {
        let source = "\
#!/bin/sh
case \"$x\" in
  *foo) : ;;
  barfoo) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "barfoo");
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
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn only_reports_the_first_later_pattern_for_each_shadowing_glob() {
        let source = "\
#!/bin/sh
case \"$x\" in
  foo*) : ;;
  foobar) : ;;
  foobaz) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foobar"]
        );
    }

    #[test]
    fn treats_fallthrough_arms_as_shadowing_sources_until_matching_resumes() {
        let source = "\
#!/bin/bash
case \"$x\" in
  foo*) : ;&
  bar) : ;;
  foobar) : ;;
esac
case \"$x\" in
  foo*) : ;&
  bar) : ;;&
  foobar) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaseDefaultBeforeGlob),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["foobar"]
        );
    }
}

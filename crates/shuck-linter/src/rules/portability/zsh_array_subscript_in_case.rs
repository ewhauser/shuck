use super::targets_non_zsh_shell;
use crate::{Checker, Rule, Violation};

pub struct ZshArraySubscriptInCase;

impl Violation for ZshArraySubscriptInCase {
    fn rule() -> Rule {
        Rule::ZshArraySubscriptInCase
    }

    fn message(&self) -> String {
        "this case pattern cannot match the case subject in this shell".to_owned()
    }
}

pub fn zsh_array_subscript_in_case(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    checker.report_all_dedup(
        checker.facts().case_pattern_impossible_spans().to_vec(),
        || ZshArraySubscriptInCase,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_unbraced_array_style_subjects_as_impossible_patterns() {
        let source = "#!/bin/sh\ncase \"$words[1]\" in\n  install) : ;;\nesac\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshArraySubscriptInCase),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["install"]
        );
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\ncase \" $oldobjs \" in\n  \" \") : ;;\nesac\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshArraySubscriptInCase).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_literal_padding_that_rules_out_a_pattern() {
        let source = "#!/bin/sh\ncase \" $oldobjs \" in\n  \" \") : ;;\n  \"  \") : ;;\nesac\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshArraySubscriptInCase),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\" \""]
        );
    }

    #[test]
    fn ignores_reachable_patterns_on_dynamic_subjects() {
        let source = "#!/bin/sh\ncase \"prefix${value}suffix\" in\n  *suffix) : ;;\n  prefix*suffix) : ;;\nesac\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshArraySubscriptInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_braced_and_fully_fixed_subjects() {
        let source = "\
#!/bin/sh
case \"${words[1]}\" in
  install) : ;;
esac
case foo in
  bar) : ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshArraySubscriptInCase),
        );

        assert!(diagnostics.is_empty());
    }
}

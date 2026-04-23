use crate::{Checker, Rule, ShellDialect, Violation};

pub struct DoubleBracketInSh;
pub struct TestEqualityOperator;
pub struct IfElifBashTest;
pub struct ExtglobInSh;
pub struct CaretNegationInBracket;
pub struct ArraySubscriptTest;
pub struct ArraySubscriptCondition;
pub struct ExtglobInTest;
pub struct LexicalComparisonInDoubleBracket;
pub struct RegexMatchInSh;
pub struct VTestInSh;
pub struct ATestInSh;
pub struct OptionTestInSh;
pub struct StickyBitTestInSh;
pub struct OwnershipTestInSh;

impl Violation for DoubleBracketInSh {
    fn rule() -> Rule {
        Rule::DoubleBracketInSh
    }

    fn message(&self) -> String {
        "`[[ ... ]]` is not available in POSIX sh".to_owned()
    }
}

impl Violation for TestEqualityOperator {
    fn rule() -> Rule {
        Rule::TestEqualityOperator
    }

    fn message(&self) -> String {
        "use `=` instead of `==` in POSIX test expressions".to_owned()
    }
}

impl Violation for IfElifBashTest {
    fn rule() -> Rule {
        Rule::IfElifBashTest
    }

    fn message(&self) -> String {
        "`elif` uses `[[ ... ]]`, which is not available in POSIX sh".to_owned()
    }
}

impl Violation for ExtglobInSh {
    fn rule() -> Rule {
        Rule::ExtglobInSh
    }

    fn message(&self) -> String {
        "extended glob syntax is not available in POSIX sh".to_owned()
    }
}

impl Violation for CaretNegationInBracket {
    fn rule() -> Rule {
        Rule::CaretNegationInBracket
    }

    fn message(&self) -> String {
        "caret negation in bracket expressions is not portable to POSIX sh".to_owned()
    }
}

impl Violation for ArraySubscriptTest {
    fn rule() -> Rule {
        Rule::ArraySubscriptTest
    }

    fn message(&self) -> String {
        "array-style subscripts in test expressions are not portable to POSIX sh".to_owned()
    }
}

impl Violation for ArraySubscriptCondition {
    fn rule() -> Rule {
        Rule::ArraySubscriptCondition
    }

    fn message(&self) -> String {
        "array-style subscripts in `[[ ... ]]` are not portable to POSIX sh".to_owned()
    }
}

impl Violation for ExtglobInTest {
    fn rule() -> Rule {
        Rule::ExtglobInTest
    }

    fn message(&self) -> String {
        "extended glob syntax in test operands is not portable to POSIX sh".to_owned()
    }
}

impl Violation for LexicalComparisonInDoubleBracket {
    fn rule() -> Rule {
        Rule::LexicalComparisonInDoubleBracket
    }

    fn message(&self) -> String {
        "lexicographical `<` and `>` inside `[[ ... ]]` are not POSIX sh test operators".to_owned()
    }
}

impl Violation for RegexMatchInSh {
    fn rule() -> Rule {
        Rule::RegexMatchInSh
    }

    fn message(&self) -> String {
        "`=~` regex matching is not available in POSIX sh".to_owned()
    }
}

impl Violation for VTestInSh {
    fn rule() -> Rule {
        Rule::VTestInSh
    }

    fn message(&self) -> String {
        "`-v` tests are not available in POSIX sh".to_owned()
    }
}

impl Violation for ATestInSh {
    fn rule() -> Rule {
        Rule::ATestInSh
    }

    fn message(&self) -> String {
        "use `-e` instead of `-a` for file-existence checks in POSIX sh".to_owned()
    }
}

impl Violation for OptionTestInSh {
    fn rule() -> Rule {
        Rule::OptionTestInSh
    }

    fn message(&self) -> String {
        "`-o` option tests are not available in POSIX sh".to_owned()
    }
}

impl Violation for StickyBitTestInSh {
    fn rule() -> Rule {
        Rule::StickyBitTestInSh
    }

    fn message(&self) -> String {
        "`-k` file tests are not portable to POSIX sh".to_owned()
    }
}

impl Violation for OwnershipTestInSh {
    fn rule() -> Rule {
        Rule::OwnershipTestInSh
    }

    fn message(&self) -> String {
        "`-O` file tests are not portable to POSIX sh".to_owned()
    }
}

fn is_posix_sh_shell(shell: ShellDialect) -> bool {
    matches!(shell, ShellDialect::Sh | ShellDialect::Dash)
}

macro_rules! cached_portability_rule {
    ($function:ident, $accessor:ident, $violation:ident) => {
        pub fn $function(checker: &mut Checker) {
            if !is_posix_sh_shell(checker.shell()) {
                return;
            }

            let spans = checker
                .facts()
                .conditional_portability()
                .$accessor()
                .to_vec();
            checker.report_all_dedup(spans, || $violation);
        }
    };
}

cached_portability_rule!(
    double_bracket_in_sh,
    double_bracket_in_sh,
    DoubleBracketInSh
);
cached_portability_rule!(
    test_equality_operator,
    test_equality_operator,
    TestEqualityOperator
);
cached_portability_rule!(if_elif_bash_test, if_elif_bash_test, IfElifBashTest);
cached_portability_rule!(extglob_in_sh, extglob_in_sh, ExtglobInSh);
cached_portability_rule!(
    caret_negation_in_bracket,
    caret_negation_in_bracket,
    CaretNegationInBracket
);
cached_portability_rule!(
    array_subscript_test,
    array_subscript_test,
    ArraySubscriptTest
);
cached_portability_rule!(
    array_subscript_condition,
    array_subscript_condition,
    ArraySubscriptCondition
);
pub fn extglob_in_test(checker: &mut Checker) {
    if !is_posix_sh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .conditional_portability()
        .extglob_in_test()
        .to_vec();
    checker.report_all_dedup(spans, || ExtglobInTest);
}
cached_portability_rule!(
    lexical_comparison_in_double_bracket,
    lexical_comparison_in_double_bracket,
    LexicalComparisonInDoubleBracket
);
cached_portability_rule!(regex_match_in_sh, regex_match_in_sh, RegexMatchInSh);
cached_portability_rule!(v_test_in_sh, v_test_in_sh, VTestInSh);
cached_portability_rule!(a_test_in_sh, a_test_in_sh, ATestInSh);
cached_portability_rule!(option_test_in_sh, option_test_in_sh, OptionTestInSh);
cached_portability_rule!(
    sticky_bit_test_in_sh,
    sticky_bit_test_in_sh,
    StickyBitTestInSh
);
cached_portability_rule!(
    ownership_test_in_sh,
    ownership_test_in_sh,
    OwnershipTestInSh
);

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_at_extglob_in_posix_shells() {
        let source = "#!/bin/sh\necho @(foo|bar)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtglobInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInSh);
    }

    #[test]
    fn reports_at_extglob_in_conditional_patterns_in_posix_shells() {
        let source = "#!/bin/sh\n[[ $OSTYPE == *@(linux|freebsd)* ]] || exit 1\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtglobInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInSh);
        assert_eq!(diagnostics[0].span.slice(source), "@(linux|freebsd)");
    }

    #[test]
    fn reports_at_extglob_in_case_patterns_in_posix_shells() {
        let source = "#!/bin/sh\ncase \"$x\" in @(foo|bar)) : ;; esac\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtglobInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInSh);
        assert_eq!(diagnostics[0].span.slice(source), "@(foo|bar)");
    }

    #[test]
    fn reports_at_extglob_in_parameter_patterns_in_posix_shells() {
        let source = "#!/bin/sh\ntrimmed=${name%@($suffix|$(printf '%s' zz))}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtglobInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInSh);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "@($suffix|$(printf '%s' zz))"
        );
    }

    #[test]
    fn reports_at_extglob_spanning_mixed_word_parts_in_posix_shells() {
        let source = "#!/bin/sh\necho @($choice|bar)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtglobInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::ExtglobInSh);
        assert_eq!(diagnostics[0].span.slice(source), "@($choice|bar)");
    }

    #[test]
    fn array_subscript_test_only_reports_unbraced_test_operands() {
        let source = "\
#!/bin/sh
if [ $tools[kops] ]; then :; fi
if [ \"${tools[kops]}\" ]; then :; fi
if [ ${#tools[@]} -eq 0 ]; then :; fi
if [ ${cost%[\\.]*} -lt 10 ]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ArraySubscriptTest));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$tools[kops]"]
        );
    }

    #[test]
    fn ignores_at_extglob_literals_in_assignment_values() {
        let source = "#!/bin/sh\nname=@(foo|bar)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtglobInSh));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_caret_negation_in_bracket_in_posix_shells() {
        let source = "\
#!/bin/sh
echo [^a]*
case x in
  [^a]*) : ;;
esac
[[ $x = [^a]* ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaretNegationInBracket),
        );

        assert_eq!(diagnostics.len(), 3);
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.rule == Rule::CaretNegationInBracket)
        );
    }

    #[test]
    fn ignores_caret_negation_in_parameter_patterns_in_posix_shells() {
        let source = "\
#!/bin/sh
trimmed=${value#[^a]*}
pkgopts=\"${XBPS_CURRENT_PKG//[^A-Za-z0-9_]/_}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaretNegationInBracket),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_caret_negation_spanning_mixed_word_parts_in_posix_shells() {
        let source = "#!/bin/sh\necho [^$chars]*\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaretNegationInBracket),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::CaretNegationInBracket);
        assert_eq!(diagnostics[0].span.slice(source), "[^$chars]");
    }

    #[test]
    fn reports_caret_negation_in_for_and_select_lists_in_posix_shells() {
        let source = "\
#!/bin/sh
for f in [^a]*; do
  :
done
select f in [^b]*; do
  break
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaretNegationInBracket),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "[^a]");
        assert_eq!(diagnostics[1].span.slice(source), "[^b]");
    }

    #[test]
    fn reports_caret_negation_in_nested_command_substitutions_in_posix_shells() {
        let source = "#!/bin/sh\necho \"$(printf '%s\\n' [^a]*)\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaretNegationInBracket),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::CaretNegationInBracket);
        assert_eq!(diagnostics[0].span.slice(source), "[^a]");
    }

    #[test]
    fn ignores_caret_negation_in_nested_parameter_patterns_in_posix_shells() {
        let source = "\
#!/bin/sh
printf '%s\n' \"$(
    sanitized=${name//[^a]/_}
    printf '%s' \"$sanitized\"
)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CaretNegationInBracket),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn sh_portability_rules_ignore_bash_shells() {
        let source = "\
#!/bin/bash
if [[ -v assoc[$key] && $term == @(foo|bar) && $term < z && $# > 1 ]]; then
  :
fi
[ \"$1\" == foo ]
[ -k \"$file\" ]
[ \"$x\" = (foo|bar)* ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([
                Rule::DoubleBracketInSh,
                Rule::TestEqualityOperator,
                Rule::IfElifBashTest,
                Rule::ExtglobInSh,
                Rule::CaretNegationInBracket,
                Rule::ArraySubscriptTest,
                Rule::ArraySubscriptCondition,
                Rule::ExtglobInTest,
                Rule::LexicalComparisonInDoubleBracket,
                Rule::RegexMatchInSh,
                Rule::VTestInSh,
                Rule::ATestInSh,
                Rule::OptionTestInSh,
                Rule::StickyBitTestInSh,
                Rule::OwnershipTestInSh,
            ])
            .with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}

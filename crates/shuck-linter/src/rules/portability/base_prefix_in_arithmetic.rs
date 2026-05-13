use shuck_ast::Span;

use crate::{
    ArithmeticLiteralKind, Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect,
    Violation,
};

pub struct BasePrefixInArithmetic;

impl Violation for BasePrefixInArithmetic {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::BasePrefixInArithmetic
    }

    fn message(&self) -> String {
        "base prefixes like `10#` are not portable in `sh` arithmetic".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace the explicit-base literal with decimal digits".to_owned())
    }
}

pub fn base_prefix_in_arithmetic(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .arithmetic_literal_facts()
        .iter()
        .filter_map(|fact| {
            (fact.kind() == ArithmeticLiteralKind::ExplicitBasePrefix).then_some(fact.span())
        })
        .map(|span| {
            let diagnostic = Diagnostic::new(BasePrefixInArithmetic, span);
            match base_prefix_fix(span, source) {
                Some(fix) => diagnostic.with_fix(fix),
                None => diagnostic,
            }
        })
        .collect::<Vec<_>>();
    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn base_prefix_fix(span: Span, source: &str) -> Option<Fix> {
    let text = span.slice(source);
    let (base, digits) = text.split_once('#')?;
    if base != "10" {
        return None;
    }
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let normalized = digits.trim_start_matches('0');
    let normalized = if normalized.is_empty() {
        "0"
    } else {
        normalized
    };
    Some(Fix::unsafe_edit(Edit::replacement(normalized, span)))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_base_prefixes_in_sh() {
        let source = "\
#!/bin/sh
echo $((10#123))
echo $((10#${foo}))
echo ${foo:10#1:2}
: > \"$((10#1))\"
echo ${foo:-$((10#1))}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BasePrefixInArithmetic),
        );

        assert_eq!(diagnostics.len(), 5);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["10#123", "10#", "10#1", "10#1", "10#1"]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_literal_decimal_base_prefixes() {
        let source = "#!/bin/sh\necho $((10#00123))\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::BasePrefixInArithmetic),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\necho $((123))\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn skips_unsafe_fix_for_non_decimal_base_prefixes() {
        let source = "#!/bin/sh\necho $((2#10 + 16#10))\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::BasePrefixInArithmetic),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 2);
    }

    #[test]
    fn ignores_base_prefixes_in_bash() {
        let source = "\
#!/bin/bash
echo $((10#123))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BasePrefixInArithmetic),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_parameter_trim_operators_in_sh() {
        let source = "\
#!/bin/sh
: \"${progname:=\"${0##*/}\"}\"
echo ${foo:-${1##*/}}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BasePrefixInArithmetic),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_parameter_trim_inside_arithmetic_in_sh() {
        let source = "\
#!/bin/sh
echo $((42949 - ${1#-} / 100000))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BasePrefixInArithmetic),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_parameter_defaults_with_base_prefixes_in_sh_arithmetic() {
        let source = "\
#!/bin/sh
echo $(( ${foo:-10#1} ))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BasePrefixInArithmetic),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "10#1");
    }
}

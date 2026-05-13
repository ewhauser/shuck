use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct LegacyArithmeticInSh;

impl Violation for LegacyArithmeticInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LegacyArithmeticInSh
    }

    fn message(&self) -> String {
        "legacy `$[...]` arithmetic is not portable in `sh` scripts".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite as `$((...))`".to_owned())
    }
}

pub fn legacy_arithmetic_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .legacy_arithmetic_fragments()
        .iter()
        .filter_map(|fragment| legacy_arithmetic_fix(fragment.span(), source))
        .map(|(span, fix)| Diagnostic::new(LegacyArithmeticInSh, span).with_fix(fix))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic(diagnostic);
    }
}

fn legacy_arithmetic_fix(span: Span, source: &str) -> Option<(Span, Fix)> {
    let text = span.slice(source);
    let body = text.strip_prefix("$[")?.strip_suffix(']')?;
    Some((
        span,
        Fix::safe_edit(Edit::replacement(format!("$(({body}))"), span)),
    ))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_legacy_arithmetic_fragment() {
        let source = "#!/bin/sh\ni=$[$i+1]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticInSh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$[$i+1]");
    }

    #[test]
    fn anchors_on_spaced_legacy_arithmetic_fragment() {
        let source = "#!/bin/sh\ni=$[ $i - 1 ]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticInSh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$[ $i - 1 ]");
    }

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\ni=$[$i+1]\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_legacy_arithmetic_in_sh() {
        let source = "#!/bin/sh\ni=$[$i+1]\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LegacyArithmeticInSh),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\ni=$(($i+1))\n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}

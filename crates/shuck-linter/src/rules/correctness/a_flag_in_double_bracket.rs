use shuck_ast::ConditionalUnaryOp;

use crate::{
    Checker, ConditionalNodeFact, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation,
};

pub struct AFlagInDoubleBracket;

impl Violation for AFlagInDoubleBracket {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::AFlagInDoubleBracket
    }

    fn message(&self) -> String {
        "use `-e` or `&&` instead of `-a` inside `[[ ... ]]`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace `-a` with `-e`".to_owned())
    }
}

pub fn a_flag_in_double_bracket(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| {
            conditional
                .nodes()
                .iter()
                .filter_map(|node| match node {
                    ConditionalNodeFact::Unary(unary)
                        if unary.op() == ConditionalUnaryOp::Exists
                            && unary.operator_span().slice(source) == "-a" =>
                    {
                        Some(unary.operator_span())
                    }
                    ConditionalNodeFact::BareWord(_)
                    | ConditionalNodeFact::Unary(_)
                    | ConditionalNodeFact::Binary(_)
                    | ConditionalNodeFact::Other(_) => None,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(AFlagInDoubleBracket, span)
                .with_fix(Fix::safe_edit(Edit::replacement("-e", span))),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_double_bracket_a_flags() {
        let source = "\
#!/bin/sh
[[ -a \"$path\" ]]
[[ ! -a \"$path\" ]]
[[ -a \"$path\" && -e \"$other\" ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AFlagInDoubleBracket),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-a", "-a", "-a"]
        );
    }

    #[test]
    fn ignores_bracket_test_a_flags_and_other_unary_ops() {
        let source = "\
#!/bin/sh
[ -a \"$path\" ]
[[ -e \"$path\" ]]
[[ -o noclobber ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AFlagInDoubleBracket),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_to_double_bracket_a_flags() {
        let source = "\
#!/bin/sh
[[ -a \"$path\" ]]
[[ ! -a \"$path\" ]]
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AFlagInDoubleBracket),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
[[ -e \"$path\" ]]
[[ ! -e \"$path\" ]]
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C122.sh").as_path(),
            &LinterSettings::for_rule(Rule::AFlagInDoubleBracket),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("C122_fix_C122.sh", result);
        Ok(())
    }
}

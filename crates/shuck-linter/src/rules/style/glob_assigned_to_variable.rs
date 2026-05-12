use crate::{
    Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation,
    WordFactHostKind, WordQuote,
};

pub struct GlobAssignedToVariable;

impl Violation for GlobAssignedToVariable {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::GlobAssignedToVariable
    }

    fn message(&self) -> String {
        "quote assigned glob patterns so they stay literal".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the assigned glob pattern".to_owned())
    }
}

pub fn glob_assigned_to_variable(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .expansion_word_facts(ExpansionContext::AssignmentValue)
        .chain(
            checker
                .facts()
                .expansion_word_facts(ExpansionContext::DeclarationAssignmentValue),
        )
        .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
        .filter(|fact| !checker.facts().is_compound_assignment_value_word(*fact))
        .filter(|fact| fact.classification().quote != WordQuote::FullyQuoted)
        .filter_map(|fact| {
            let glob_spans = fact.active_literal_glob_spans(source);
            (!glob_spans.is_empty()).then(|| {
                Diagnostic::new(GlobAssignedToVariable, fact.span())
                    .with_fix(quote_glob_spans_fix(glob_spans, source))
            })
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn quote_glob_spans_fix(spans: Vec<shuck_ast::Span>, source: &str) -> Fix {
    let edits = spans.into_iter().map(|span| {
        let text = span.slice(source);
        Edit::replacement(format!("'{text}'"), span)
    });
    Fix::safe_edits(edits)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_assignment_values_with_unquoted_globs() {
        let source = "\
#!/bin/bash
LOCS=*.oxt
LOCS=$dir/*.oxt
LOCS=\"$dir\"/*.oxt
LOCS=${dir}/*.oxt
LOCS=$(pwd)/*.oxt
readonly LOCS=foo*bar
export LOCS=$dir/*.txt
LOCS=${disk_info[${#disk_info[@]} - 1]/*\\/}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobAssignedToVariable),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "*.oxt",
                "$dir/*.oxt",
                "\"$dir\"/*.oxt",
                "${dir}/*.oxt",
                "$(pwd)/*.oxt",
                "foo*bar",
                "$dir/*.txt"
            ]
        );
    }

    #[test]
    fn ignores_fully_quoted_or_non_glob_assignment_values() {
        let source = "\
#!/bin/bash
LOCS=\"*.oxt\"
LOCS='*.oxt'
LOCS=\\*.oxt
LOCS=$dir/file.txt
readonly LOCS=\"*.txt\"
export LOCS='$dir/*.txt'
LOCS=(*.oxt)
LOCS=(\"$dir\"/*.oxt)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobAssignedToVariable),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn applies_safe_fix_by_quoting_literal_glob_fragments() {
        let source = "\
#!/bin/bash
LOCS=*.oxt
LOCS=$dir/*.oxt
readonly LOCS=foo*bar
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::GlobAssignedToVariable),
            Applicability::Safe,
        );

        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
LOCS='*'.oxt
LOCS=$dir/'*'.oxt
readonly LOCS=foo'*'bar
"
        );
        assert_eq!(result.fixes_applied, 3);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S055.sh").as_path(),
            &LinterSettings::for_rule(Rule::GlobAssignedToVariable),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("S055_fix_S055.sh", result);
        Ok(())
    }
}

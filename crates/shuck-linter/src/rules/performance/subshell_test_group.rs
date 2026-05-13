use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SubshellTestGroup;

impl Violation for SubshellTestGroup {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SubshellTestGroup
    }

    fn message(&self) -> String {
        "use braces to group these tests instead of a subshell".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("replace the subshell with a brace group".to_owned())
    }
}

pub fn subshell_test_group(checker: &mut Checker) {
    let single_test_spans = checker.facts().single_test_subshell_spans();
    let spans = checker
        .facts()
        .subshell_test_group_spans()
        .iter()
        .copied()
        .filter(|span| !single_test_spans.contains(span))
        .collect::<Vec<_>>();
    for span in spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(SubshellTestGroup, span).with_fix(brace_group_fix(span)),
        );
    }
}

fn brace_group_fix(span: Span) -> Fix {
    Fix::unsafe_edits([
        Edit::replacement_at(span.start.offset, span.start.offset + 1, "{"),
        Edit::replacement_at(span.end.offset.saturating_sub(1), span.end.offset, "; }"),
    ])
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn anchors_on_the_grouping_subshell() {
        let source = "\
#!/bin/sh
a=1
b=jpg
if [ -n \"$a\" ] && ( [ \"$b\" = jpeg ] || [ \"$b\" = jpg ] ); then echo ok; fi
if ! ( [ \"$b\" = jpeg ] || [ \"$b\" = jpg ] ); then echo ok; fi
if ( [ \"$b\" = jpeg ] || [ \"$b\" = jpg ] ); then echo ok; fi
( { [ \"$b\" = jpeg ] || [ \"$b\" = jpg ]; } )
( [ \"$b\" = jpeg ] ; [ \"$b\" = jpg ] )
( cd /tmp || exit 1
  [ \"$b\" = jpg ]
 )
( [ \"$b\" = jpeg ] )
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellTestGroup));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "( [ \"$b\" = jpeg ] || [ \"$b\" = jpg ] )",
                "( [ \"$b\" = jpeg ] || [ \"$b\" = jpg ] )",
                "( { [ \"$b\" = jpeg ] || [ \"$b\" = jpg ]; } )",
                "( [ \"$b\" = jpeg ] ; [ \"$b\" = jpg ] )",
            ]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_replace_subshell_group_with_braces() {
        let source = "#!/bin/sh\na=1\nb=jpg\nif [ -n \"$a\" ] && ( [ \"$b\" = jpeg ] || [ \"$b\" = jpg ] ); then echo ok; fi\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SubshellTestGroup),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\na=1\nb=jpg\nif [ -n \"$a\" ] && { [ \"$b\" = jpeg ] || [ \"$b\" = jpg ] ; }; then echo ok; fi\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}

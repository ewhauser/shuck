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
    let source = checker.source();
    let single_test_spans = checker.facts().command_facts().single_test_subshell_spans();
    let spans = checker
        .facts()
        .command_facts()
        .subshell_test_group_spans()
        .iter()
        .copied()
        .filter(|span| !single_test_spans.contains(span))
        .collect::<Vec<_>>();
    for span in spans {
        checker.report_diagnostic_dedup(
            Diagnostic::new(SubshellTestGroup, span).with_fix(brace_group_fix(source, span)),
        );
    }
}

fn brace_group_fix(source: &str, span: Span) -> Fix {
    let close_start = span.end.offset.saturating_sub(1);
    let close_replacement = if offset_is_indented_line_start(source, close_start) {
        "}"
    } else {
        "; }"
    };

    Fix::unsafe_edits([
        Edit::replacement_at(span.start.offset, span.start.offset + 1, "{"),
        Edit::replacement_at(close_start, span.end.offset, close_replacement),
    ])
}

fn offset_is_indented_line_start(source: &str, offset: usize) -> bool {
    let line_start = source[..offset].rfind('\n').map_or(0, |offset| offset + 1);
    source[line_start..offset]
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
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

    #[test]
    fn applies_unsafe_fix_to_multiline_subshell_group() {
        let source = "\
#!/bin/sh
if (
  [ -f a ]
  [ -f b ]
); then
  echo ok
fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SubshellTestGroup),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
if {
  [ -f a ]
  [ -f b ]
}; then
  echo ok
fi
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}

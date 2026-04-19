use crate::{Checker, Rule, Violation};

pub struct SubshellTestGroup;

impl Violation for SubshellTestGroup {
    fn rule() -> Rule {
        Rule::SubshellTestGroup
    }

    fn message(&self) -> String {
        "use braces to group these tests instead of a subshell".to_owned()
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
    checker.report_all_dedup(spans, || SubshellTestGroup);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}

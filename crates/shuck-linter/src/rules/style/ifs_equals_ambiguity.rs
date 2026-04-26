use shuck_ast::{AssignmentNode, Span};

use crate::{Checker, Rule, Violation};

pub struct IfsEqualsAmbiguity;

impl Violation for IfsEqualsAmbiguity {
    fn rule() -> Rule {
        Rule::IfsEqualsAmbiguity
    }

    fn message(&self) -> String {
        "quote `=` when assigning IFS to avoid looking like `==`".to_owned()
    }
}

pub fn ifs_equals_ambiguity(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| command_assignment_spans(fact.arena_assignments(), source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || IfsEqualsAmbiguity);
}

fn command_assignment_spans(assignments: &[AssignmentNode], source: &str) -> Vec<Span> {
    assignments
        .iter()
        .filter_map(|assignment| ifs_equals_ambiguity_span(assignment, source))
        .collect()
}

fn ifs_equals_ambiguity_span(assignment: &AssignmentNode, source: &str) -> Option<Span> {
    if assignment.append || assignment.target.name.as_str() != "IFS" {
        return None;
    }

    (assignment.span.slice(source) == "IFS==")
        .then(|| Span::at(assignment.span.start.advanced_by("IFS=")))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_the_second_equals_sign() {
        let source = "\
#!/bin/bash
IFS== read x
while IFS== read -r key val; do
  :
done < /dev/null
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfsEqualsAmbiguity));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![(2, 5), (3, 11)]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.span.start == diagnostic.span.end)
        );
    }

    #[test]
    fn ignores_quoted_equals_and_other_assignments() {
        let source = "\
#!/bin/bash
IFS='=' read x
IFS=\"=\" read y
IFS=\\= read z
foo==bar env
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfsEqualsAmbiguity));

        assert!(diagnostics.is_empty());
    }
}

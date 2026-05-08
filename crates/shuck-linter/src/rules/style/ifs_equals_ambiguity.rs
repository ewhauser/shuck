use shuck_ast::{Assignment, BuiltinCommand, Command, Span};

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct IfsEqualsAmbiguity;

impl Violation for IfsEqualsAmbiguity {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::IfsEqualsAmbiguity
    }

    fn message(&self) -> String {
        "quote `=` when assigning IFS to avoid looking like `==`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite `IFS==` as `IFS='='`".to_owned())
    }
}

pub fn ifs_equals_ambiguity(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| command_assignment_spans(fact.command(), source))
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(Diagnostic::new(IfsEqualsAmbiguity, span).with_fix(
            Fix::safe_edit(Edit::replacement_at(
                span.start.offset,
                span.start.offset + 1,
                "'='",
            )),
        ));
    }
}

fn command_assignment_spans(command: &Command, source: &str) -> Vec<Span> {
    match command {
        Command::Simple(command) => command
            .assignments
            .iter()
            .filter_map(|assignment| ifs_equals_ambiguity_span(assignment, source))
            .collect(),
        Command::Builtin(command) => builtin_assignments(command)
            .iter()
            .filter_map(|assignment| ifs_equals_ambiguity_span(assignment, source))
            .collect(),
        Command::Decl(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => Vec::new(),
    }
}

fn builtin_assignments(command: &BuiltinCommand) -> &[Assignment] {
    match command {
        BuiltinCommand::Break(command) => &command.assignments,
        BuiltinCommand::Continue(command) => &command.assignments,
        BuiltinCommand::Return(command) => &command.assignments,
        BuiltinCommand::Exit(command) => &command.assignments,
    }
}

fn ifs_equals_ambiguity_span(assignment: &Assignment, source: &str) -> Option<Span> {
    if assignment.append || assignment.target.name.as_str() != "IFS" {
        return None;
    }

    (assignment.span.slice(source) == "IFS==")
        .then(|| Span::at(assignment.span.start.advanced_by("IFS=")))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

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

    #[test]
    fn applies_safe_fix_to_ambiguous_ifs_equals_assignments() {
        let source = "\
#!/bin/bash
IFS== read x
while IFS== read -r key val; do
  :
done < /dev/null
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::IfsEqualsAmbiguity),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
IFS='=' read x
while IFS='=' read -r key val; do
  :
done < /dev/null
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_quoted_equals_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
IFS='=' read x
IFS=\"=\" read y
IFS=\\= read z
foo==bar env
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::IfsEqualsAmbiguity),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S042.sh").as_path(),
            &LinterSettings::for_rule(Rule::IfsEqualsAmbiguity),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("S042_fix_S042.sh", result);
        Ok(())
    }
}

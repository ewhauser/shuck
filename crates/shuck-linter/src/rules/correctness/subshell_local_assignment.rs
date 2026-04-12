use crate::{Checker, Rule, Violation};

pub struct SubshellLocalAssignment {
    pub name: String,
}

impl Violation for SubshellLocalAssignment {
    fn rule() -> Rule {
        Rule::SubshellLocalAssignment
    }

    fn message(&self) -> String {
        format!(
            "assignment to `{}` inside a subshell does not update the outer shell",
            self.name
        )
    }
}

pub fn subshell_local_assignment(checker: &mut Checker) {
    let spans = checker.facts().subshell_local_assignment_spans().to_vec();

    for span in spans {
        let name = span.slice(checker.source()).to_owned();
        checker.report(SubshellLocalAssignment { name }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_reads_that_still_see_the_outer_binding_after_a_subshell_assignment() {
        let source = "\
#!/bin/sh
count=0
(count=1)
echo \"$count\"
items=old
(items=new)
printf '%s\\n' \"$items\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$count", "$items"]
        );
    }

    #[test]
    fn ignores_pipeline_cases_and_parent_reassignments() {
        let source = "\
#!/bin/sh
count=0
printf '%s\\n' x | while read -r _; do count=1; done
echo \"$count\"
items=old
(items=new)
items=latest
echo \"$items\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_local_declarations_inside_subshells() {
        let source = "\
#!/bin/bash
demo() {
  value=outer
  (local value=inner)
  echo \"$value\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_command_substitution_assignments_that_do_not_escape() {
        let source = "\
#!/bin/bash
value=outer
snapshot=\"$(
  value=inner
  printf '%s\\n' done
)\"
echo \"$value\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$value"]
        );
    }

    #[test]
    fn ignores_reads_that_stay_inside_the_same_command_substitution() {
        let source = "\
#!/bin/bash
value=outer
snapshot=\"$(
  value=inner
  printf '%s\\n' \"$value\"
)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_later_reads_when_the_only_assignment_was_exported_inside_a_subshell() {
        let source = "\
#!/bin/sh
(
  export value=inner
)
printf '%s\\n' \"$value\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$value"]
        );
    }

    #[test]
    fn ignores_cross_function_reads_after_a_parent_shell_reset() {
        let source = "\
#!/bin/sh
first() {
  (
    export value=inner
  )
}
second() {
  value=outer
}
third() {
  printf '%s\\n' \"$value\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_later_export_reassignments_after_a_subshell_assignment() {
        let source = "\
#!/bin/sh
demo() {
  (
    export value=inner
  )
  export value=outer
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["value"]
        );
    }

    #[test]
    fn reports_later_append_assignments_after_a_subshell_assignment() {
        let source = "\
#!/bin/bash
demo() {
  (
    value=inner
  )
  value+=-suffix
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["value"]
        );
    }

    #[test]
    fn reports_self_reference_inside_later_export_after_a_subshell_assignment() {
        let source = "\
#!/bin/sh
first() {
  (
    export PATH=/usr/bin:$PATH
  )
}
second() {
  export PATH=$HOME/bin:$PATH
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["PATH", "$PATH"]
        );
    }
}

use crate::{Checker, Rule, Violation};

pub struct SubshellLocalAssignment {
    pub name: String,
}

impl Violation for SubshellLocalAssignment {
    fn rule() -> Rule {
        Rule::SubshellLocalAssignment
    }

    fn message(&self) -> String {
        format!("assignment to `{}` only changes the subshell copy", self.name)
    }
}

pub fn subshell_local_assignment(checker: &mut Checker) {
    let sites = checker.facts().subshell_assignment_sites().to_vec();

    for site in sites {
        checker.report(
            SubshellLocalAssignment {
                name: site.name.to_string(),
            },
            site.span,
        );
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
            vec!["count", "items"]
        );
    }

    #[test]
    fn reports_pipeline_child_reads_that_happen_after_the_pipeline_finishes() {
        let source = "\
#!/bin/sh
count=0
printf '%s\\n' x | while read -r _; do count=1; done
echo \"$count\"
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
            vec!["count"]
        );
    }

    #[test]
    fn ignores_parent_reassignments_after_nonpersistent_updates() {
        let source = "\
#!/bin/sh
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
    fn reports_parameter_default_reads_after_pipeline_assignments() {
        let source = "\
#!/bin/sh
printf '%s\\n' x | while read -r _; do : \"${value:=inner}\"; done
printf '%s\\n' \"${value:=outer}\"
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
            vec!["${value:=inner}"]
        );
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
            vec!["value"]
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
    fn reports_later_arithmetic_assignments_after_pipeline_updates() {
        let source = "\
#!/bin/bash
PASS=0
printf '%s\\n' x | while read -r _; do PASS=1; done
((PASS++))
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
            vec!["PASS"]
        );
    }

    #[test]
    fn ignores_ifs_reads_after_pipeline_updates() {
        let source = "\
#!/bin/sh
printf '%s\\n' x | while read -r _; do IFS=:; done
printf '%s\\n' \"$IFS\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_inner_command_substitution_updates_when_the_parent_assignment_resets_the_name() {
        let source = "\
#!/bin/sh
k1=0
k1=\"$(printf '%s' 1 || k1=0)\"
printf '%s\\n' \"$k1\"
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
            vec!["value"]
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
            vec!["PATH"]
        );
    }

    #[test]
    fn reports_only_the_first_assignment_in_a_single_child_scope() {
        let source = "\
#!/bin/sh
x=0
(
  x=1
  x=2
)
echo \"$x\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.slice(source), "x");
    }

    #[test]
    fn reports_only_the_latest_child_scope_before_a_later_outer_use() {
        let source = "\
#!/bin/sh
x=0
(x=1)
(x=2)
echo \"$x\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 4);
        assert_eq!(diagnostics[0].span.slice(source), "x");
    }
}

use crate::{Checker, Rule, Violation};

pub struct SubshellSideEffect {
    pub name: String,
}

impl Violation for SubshellSideEffect {
    fn rule() -> Rule {
        Rule::SubshellSideEffect
    }

    fn message(&self) -> String {
        format!(
            "assignment to `{}` only affects the child shell here",
            self.name
        )
    }
}

pub fn subshell_side_effect(checker: &mut Checker) {
    let spans = checker.facts().subshell_side_effect_spans().to_vec();

    for span in spans {
        let name = span.slice(checker.source()).to_owned();
        checker.report(SubshellSideEffect { name }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_subshell_assignment_sites_that_have_later_outer_reads() {
        let source = "\
#!/bin/sh
count=0
(count=1)
echo \"$count\"
items=old
(items=new)
printf '%s\\n' \"$items\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["count", "items"]
        );
    }

    #[test]
    fn reports_pipeline_child_assignments_that_do_not_escape() {
        let source = "\
#!/bin/sh
count=0
printf '%s\\n' x | while read -r _; do count=1; done
echo \"$count\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["count"]
        );
    }

    #[test]
    fn reports_parameter_default_assignments_inside_pipeline_children() {
        let source = "\
#!/bin/sh
printf '%s\\n' x | while read -r _; do : \"${value:=inner}\"; done
printf '%s\\n' \"${value:=outer}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "value");
    }

    #[test]
    fn reports_command_substitution_assignment_sites_that_have_later_outer_reads() {
        let source = "\
#!/bin/bash
value=outer
snapshot=\"$(
  value=inner
  printf '%s\\n' done
)\"
echo \"$value\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["value"]
        );
    }

    #[test]
    fn ignores_ifs_assignments_inside_pipeline_children() {
        let source = "\
#!/bin/sh
printf '%s\\n' x | while read -r _; do IFS=:; done
printf '%s\\n' \"$IFS\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_child_shell_assignments_without_later_outer_uses() {
        let source = "\
#!/bin/sh
count=0
(count=1)
printf '%s\\n' done
printf '%s\\n' x | while read -r _; do items=1; done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_child_shell_assignments_when_the_parent_resets_before_reuse() {
        let source = "\
#!/bin/sh
count=0
printf '%s\\n' x | while read -r _; do count=1; done
count=2
echo \"$count\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty());
    }
}

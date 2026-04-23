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
            "`{}` may still resolve to the outer-shell value here",
            self.name
        )
    }
}

pub fn subshell_side_effect(checker: &mut Checker) {
    let sites = checker.facts().subshell_later_use_sites().to_vec();

    for site in sites {
        checker.report(
            SubshellSideEffect {
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
            vec!["$count", "$items"]
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
            vec!["$count"]
        );
    }

    #[test]
    fn ignores_bash_pipeline_assignments_when_pipefail_is_enabled() {
        let source = "\
#!/usr/bin/env bash
set -o pipefail
count=0
printf '%s\\n' x | while read -r _; do count=1; done
echo \"$count\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_command_substitution_assignments_even_when_pipefail_is_enabled() {
        let source = "\
#!/usr/bin/env bash
set -o pipefail
value=outer
snapshot=\"$(value=inner | cat)\"
echo \"$value\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$value"]
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
        assert_eq!(diagnostics[0].span.slice(source), "${value:=outer}");
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
            vec!["$value"]
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
    fn reports_later_reads_after_uninitialized_declarations() {
        let source = "\
#!/bin/bash
demo() {
  (value=inner)
  local value
  printf '%s\\n' \"${value:-}\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${value:-}"]
        );
    }

    #[test]
    fn reports_bare_export_as_a_later_use_after_subshell_assignment() {
        let source = "\
#!/bin/bash
(value=inner)
export value
printf '%s\\n' \"${value:-}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["value", "${value:-}"]
        );
    }

    #[test]
    fn reports_local_declarations_inside_pipeline_defined_functions_when_parent_has_value() {
        let source = "\
#!/usr/bin/env bash
NETWORK=outer
printf '%s\\n' x | while read -r _; do
  f() { local NETWORK=\"$1\"; }
done
echo \"$NETWORK\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$NETWORK"]
        );
    }

    #[test]
    fn ignores_effectively_local_function_assignments_without_parent_value() {
        let source = "\
#!/usr/bin/env bash
printf '%s\\n' x | while read -r _; do
  f() {
    local LOAD=\"$1\"
    LOAD=\"$( echo ${LOAD:=0} | sed -ne 's/x/y/p' )\"
    [ ${LOAD:=0} -gt 60 ] && echo high
  }
  echo \"$LOAD\"
  f \"$LOAD\"
done | sort
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_prompt_runtime_references_on_ps4_assignment_targets() {
        let source = "\
#!/usr/bin/env bash
(rvm_path=inner)
export PS4=\"+ \\${rvm_path:-} \"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["PS4"]
        );
    }

    #[test]
    fn ignores_dynamic_arithmetic_commands_that_read_arrays() {
        let source = "\
#!/usr/bin/env bash
declare -A proc
filter=x
if ((proc[selected]==(1${filter:++1})-proc[start])); then :; fi
echo \"${proc[start]}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_future_bindings_when_matching_later_reads() {
        let source = "\
#!/usr/bin/env bash
rvm_ruby_string=outer
(rvm_ruby_string=inner)
echo \"$rvm_ruby_string\"
for rvm_ruby_string in a; do :; done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$rvm_ruby_string"]
        );
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

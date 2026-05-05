use compact_str::CompactString;

use crate::{Checker, Diagnostic, Rule, Violation};

pub struct SubshellLocalAssignment {
    pub name: CompactString,
}

impl Violation for SubshellLocalAssignment {
    fn rule() -> Rule {
        Rule::SubshellLocalAssignment
    }

    fn message(&self) -> String {
        format!(
            "assignment to `{}` only changes the subshell copy",
            self.name
        )
    }
}

pub fn subshell_local_assignment(checker: &mut Checker) {
    checker.report_fact_diagnostics(|facts, report| {
        for site in facts.subshell_assignment_sites() {
            report(Diagnostic::new(
                SubshellLocalAssignment {
                    name: site.name.as_str().into(),
                },
                site.span,
            ));
        }
    });
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
    fn escaped_name_only_declarations_do_not_hide_subshell_assignments() {
        let source = "\
#!/bin/bash
demo() {
  ( \\typeset value; value=inner )
  echo \"$value\"
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
    fn ignores_zsh_final_pipeline_component_assignments() {
        let source = "\
#!/bin/zsh
header=
printf '%s\\n' x |& grep x | while read -r _; do header=ok; done
print -r -- \"$header\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_nonfinal_pipeline_component_assignments() {
        let source = "\
#!/bin/zsh
header=
print x | { header=bad; print y; } | cat
print -r -- \"$header\"
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
            vec!["header"]
        );
    }

    #[test]
    fn reports_zsh_sh_emulation_final_pipeline_component_assignments() {
        let source = "\
#!/bin/zsh
emulate sh
header=
printf '%s\\n' x | while read -r _; do header=bad; done
print -r -- \"$header\"
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
            vec!["header"]
        );
    }

    #[test]
    fn reports_zsh_compound_sh_emulation_final_pipeline_component_assignments() {
        let source = "\
#!/bin/zsh
{ emulate sh; }
header=
printf '%s\\n' x | while read -r _; do header=bad; done
print -r -- \"$header\"
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
            vec!["header"]
        );
    }

    #[test]
    fn reports_zsh_if_condition_sh_emulation_final_pipeline_component_assignments() {
        let source = "\
#!/bin/zsh
if emulate sh; then :; fi
header=
printf '%s\\n' x | while read -r _; do header=bad; done
print -r -- \"$header\"
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
            vec!["header"]
        );
    }

    #[test]
    fn reports_zsh_binary_sh_emulation_final_pipeline_component_assignments() {
        let source = "\
#!/bin/zsh
emulate sh && :
header=
printf '%s\\n' x | while read -r _; do header=bad; done
print -r -- \"$header\"
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
            vec!["header"]
        );
    }

    #[test]
    fn reports_zsh_always_body_sh_emulation_final_pipeline_component_assignments() {
        let source = "\
#!/bin/zsh
{ emulate sh; } always { :; }
header=
printf '%s\\n' x | while read -r _; do header=bad; done
print -r -- \"$header\"
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
            vec!["header"]
        );
    }

    #[test]
    fn reports_zsh_always_cleanup_sh_emulation_final_pipeline_component_assignments() {
        let source = "\
#!/bin/zsh
{ :; } always { emulate sh; }
header=
printf '%s\\n' x | while read -r _; do header=bad; done
print -r -- \"$header\"
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
            vec!["header"]
        );
    }

    #[test]
    fn ignores_zsh_option_map_keys_without_visible_opts_binding() {
        let source = "\
#!/bin/zsh
f() {
  local quiet=0
  ( (( !OPTS[opt_-q,--quiet] )) )
  (( quiet ))
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_option_map_keys_in_if_arithmetic_conditions() {
        let source = "\
#!/bin/zsh
f() {
  local quiet=0
  (
    if (( ! OPTS[opt_-q,--quiet] )) {
      :
    }
  )
  (( quiet ))
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_option_map_keys_after_option_map_assignment() {
        let source = "\
#!/bin/zsh
f() {
  local quiet=0
  OPTS[opt_-q,--quiet]=1
  ( (( OPTS[opt_-q,--quiet] )) )
  (( quiet ))
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_option_shaped_indexed_opts_arithmetic_subshells() {
        let source = "\
#!/bin/zsh
f() {
  local -a OPTS
  local opt_=1 q=1 quiet=0
  ( (( OPTS[opt_-q,--quiet] )) )
  (( quiet ))
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_option_shaped_indexed_opts_after_option_map_assignment() {
        let source = "\
#!/bin/zsh
f() {
  local -a OPTS
  local opt_=1 q=1 quiet=0
  OPTS[opt_-q,--quiet]=1
  ( (( OPTS[opt_-q,--quiet] )) )
  (( quiet ))
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_option_shaped_caller_indexed_opts_arithmetic_subshells() {
        let source = "\
#!/bin/zsh
callee() {
  local opt_=1 q=1 quiet=0
  ( (( OPTS[opt_-q,--quiet] )) )
  (( quiet ))
}
caller() {
  local -a OPTS
  callee
}
caller
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
    }

    #[test]
    fn reports_for_loop_child_assignments_on_the_loop_keyword() {
        let source = "\
#!/bin/sh
{ for opt_mode in compile link; do :; done; } | sed -n p
printf '%s\\n' \"$opt_mode\"
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
            vec!["for"]
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
    fn ignores_zsh_later_reads_after_helper_resets_name_in_parent_scope() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
(
  for REPLY in a; do :; done
)
helper
print -r -- $REPLY
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_later_reads_when_helper_reset_is_conditional() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
(
  for REPLY in a; do :; done
)
[[ -n $cond ]] && helper
print -r -- $REPLY
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_after_conditional_subshell_assignment() {
        let source = "\
#!/bin/zsh
REPLY=old
if [[ -n $cond ]]; then
  (
    REPLY=value
  )
fi
print -r -- $REPLY
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
            vec!["REPLY"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_when_reset_is_in_disjoint_case_branch() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
case $site in
  github)
    (
      for REPLY in a; do :; done
    )
    ;;
  cygwin)
    helper
    ;;
esac
print -r -- $REPLY
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_after_top_level_helper_call_before_definition() {
        let source = "\
#!/bin/zsh
(
  for REPLY in a; do :; done
)
helper
print -r -- $REPLY
helper() {
  REPLY=value
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_when_helper_only_resets_loop_variable() {
        let source = "\
#!/bin/zsh
helper() {
  for REPLY in \"$@\"; do :; done
}
(
  for REPLY in a; do :; done
)
helper
print -r -- $REPLY
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_after_branch_only_helper_reset() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
(
  for REPLY in a; do :; done
)
if [[ -n $cond ]]; then
  helper
fi
print -r -- $REPLY
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_after_conditional_helper_body_reset() {
        let source = "\
#!/bin/zsh
helper() {
  [[ -n $cond ]] && REPLY=value
}
(
  for REPLY in a; do :; done
)
helper
print -r -- $REPLY
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_after_dead_helper_body_reset() {
        let source = "\
#!/bin/zsh
helper() {
  return
  REPLY=value
}
(
  for REPLY in a; do :; done
)
helper
print -r -- $REPLY
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
            vec!["for"]
        );
    }

    #[test]
    fn ignores_zsh_later_reads_after_always_run_binary_left_helper_reset() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
(
  for REPLY in a; do :; done
)
helper || :
print -r -- $REPLY
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_later_reads_after_background_helper_reset() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
(
  for REPLY in a; do :; done
)
helper &
print -r -- $REPLY
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
            vec!["for"]
        );
    }

    #[test]
    fn ignores_zsh_later_reads_after_pipeline_tail_helper_reset() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
(
  for REPLY in a; do :; done
)
print -r -- input | helper
print -r -- $REPLY
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_later_reads_after_pipeline_tail_helper_reset_in_or_list() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
(
  for REPLY in a; do :; done
)
print -r -- input | helper || :
print -r -- $REPLY
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_later_reads_after_pipe_ampersand_tail_helper_reset() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
(
  for REPLY in a; do :; done
)
print -r -- input |& helper
print -r -- $REPLY
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_later_reads_after_middle_pipeline_helper_reset() {
        let source = "\
#!/bin/zsh
(
  for REPLY in a; do :; done
)
print -r -- input | _reply-helper | cat
print -r -- $REPLY
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_reads_in_helper_call_arguments_before_reset_runs() {
        let source = "\
#!/bin/zsh
helper() {
  REPLY=value
}
(
  for REPLY in a; do :; done
)
helper \"$REPLY\"
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_after_conditional_set_a_outparam_helper() {
        let source = "\
#!/bin/zsh
fill() {
  [[ -n $cond ]] && set -A $1 ${(f)\"$(
    shift
    for d; do
      print -r -- $d
    done
  )\"}
}
fill d /tmp
print -r -- $d
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_after_attached_set_a_helper_argument() {
        let source = "\
#!/bin/zsh
fill() {
  set -Aresult $1
}
(
  for d in /tmp; do :; done
)
fill d
print -r -- $d
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
            vec!["for"]
        );
    }

    #[test]
    fn ignores_zsh_later_reads_in_sibling_nested_scopes() {
        let source = "\
#!/bin/zsh
case $site in
  github)
    (
      for REPLY in a; do :; done
    )
    ;;
  cygwin)
    (
      print -r -- $REPLY
    )
    ;;
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_later_reads_in_parent_nested_subshell_scope() {
        let source = "\
#!/bin/zsh
(
  (
    for REPLY in a; do :; done
  )
  print -r -- $REPLY
)
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_after_subshell_set_a_outparam_helper() {
        let source = "\
#!/bin/zsh
fill() {
  (
    set -A $1 value
  )
}
(
  for d in /tmp; do :; done
)
fill d
print -r -- $d
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
            vec!["for"]
        );
    }

    #[test]
    fn ignores_zsh_later_reads_after_later_defined_helper_resets_name() {
        let source = "\
#!/bin/zsh
demo() {
  (
    for REPLY in a; do :; done
  )
  helper
  print -r -- $REPLY
}
helper() {
  REPLY=value
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_zsh_later_reads_after_function_local_helper_call_before_definition() {
        let source = "\
#!/bin/zsh
demo() {
  (
    for REPLY in a; do :; done
  )
  helper
  print -r -- $REPLY
  helper() {
    REPLY=value
  }
}
demo
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
            vec!["for"]
        );
    }

    #[test]
    fn reports_zsh_later_reads_when_function_runs_before_later_helper_definition() {
        let source = "\
#!/bin/zsh
demo() {
  (
    for REPLY in a; do :; done
  )
  helper
  print -r -- $REPLY
}
demo
helper() {
  REPLY=value
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
            vec!["for"]
        );
    }

    #[test]
    fn ignores_zsh_reply_reads_after_unresolved_private_helper_calls() {
        let source = "\
#!/bin/zsh
demo() {
  (
    for reply in a; do :; done
  )
  .helper-from-sourced-file input
  print -r -- $reply
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_set_a_outparam_helpers_after_command_substitution_loop_assignments() {
        let source = "\
#!/bin/zsh
fill() {
  set -A $1 ${(f)\"$(
    shift
    for d; do
      print -r -- $d
    done
  )\"}
}
fill d /tmp
print -r -- $d
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_quoted_set_a_outparam_helpers_after_command_substitution_loop_assignments() {
        let source = "\
#!/bin/zsh
fill() {
  set -A \"$1\" ${(f)\"$(
    shift
    for d; do
      print -r -- $d
    done
  )\"}
}
fill d /tmp
print -r -- $d
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_process_substitution_fd_closes_after_parent_fd_assignment() {
        let source = "\
#!/bin/zsh
demo() {
  local fd
  sysopen -ro cloexec -u fd <(
    (
      local fd
      sysopen -wo create,excl -u fd -- lock
      exec {fd}>&-
    ) &!
  )
  IFS= read -ru $fd
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SubshellLocalAssignment),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
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

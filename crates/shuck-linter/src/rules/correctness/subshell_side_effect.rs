use compact_str::CompactString;

use crate::{Checker, Diagnostic, Rule, Violation};

pub struct SubshellSideEffect {
    pub name: CompactString,
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
    checker.report_fact_diagnostics(|facts, report| {
        for site in facts.subshell_later_use_sites() {
            report(Diagnostic::new(
                SubshellSideEffect {
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
    fn escaped_name_only_declarations_do_not_hide_later_outer_reads() {
        let source = "\
#!/bin/bash
demo() {
  ( \\typeset value; value=inner )
  echo \"$value\"
}
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
    fn ignores_zsh_final_pipeline_component_assignments() {
        let source = "\
#!/bin/zsh
header=
printf '%s\\n' x |& grep x | while read -r _; do header=ok; done
print -r -- \"$header\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$header"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$header"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$header"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$header"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$header"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$header"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$header"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$REPLY"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$REPLY"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$REPLY"]
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

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
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubshellSideEffect));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
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

use crate::{Checker, Rule, Violation};

pub struct StatusCaptureAfterBranchTest;

impl Violation for StatusCaptureAfterBranchTest {
    fn rule() -> Rule {
        Rule::StatusCaptureAfterBranchTest
    }

    fn message(&self) -> String {
        "`$?` here refers to a condition result, not an earlier command".to_owned()
    }
}

pub fn status_capture_after_branch_test(checker: &mut Checker) {
    checker.report_fact_slice(
        |facts| facts.condition_status_capture_spans(),
        || StatusCaptureAfterBranchTest,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_status_reads_in_first_branch_commands_after_test_conditions() {
        let source = "\
#!/bin/sh
if [ \"$x\" = y ]; then first=$?; fi
while [ \"$x\" = y ]; do again=$?; break; done
[[ \"$x\" = y ]] || return $?
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?", "$?"]
        );
    }

    #[test]
    fn reports_follow_up_elif_conditions_and_case_bodies() {
        let source = "\
#!/bin/sh
foo
if [ $? -eq 0 ]; then
  :
elif [ $? -eq 1 ]; then
  :
fi
if [ \"$mode\" = foo ]; then
  case $mode in
    foo) tend $? ;;
  esac
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?"]
        );
    }

    #[test]
    fn keeps_only_narrow_safe_branch_status_exemptions() {
        let source = "\
#!/bin/sh
safe_capture() {
if [ \"$a\" ]; then
  work=1
  for item in a; do
    :
  done
elif [ $? -eq 4 ]; then
  :
elif [ $? -eq 3 ]; then
  :
fi
if [ \"$x\" ]; then
  :
else
  _ret=\"$?\"
  _err \"saved\"
fi
return $_ret
}
if [ \"$x\" ]; then
  :
else
  saved=$?
fi
if [ $? -ne 0 ]; then
  logError \"status=$?\"
  exit 1
fi
if [ -z \"$?\" ]; then
  die \"status=$?\"
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?", "$?"]
        );
    }

    #[test]
    fn ignores_non_test_conditions_and_late_status_reads() {
        let source = "\
#!/bin/sh
if false; then ok=$?; fi
if [ \"$x\" = y ]; then :; later=$?; fi
if [ \"$x\" = y ] || true; then mixed=$?; fi
foo || return $?
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_embedded_branch_status_captures_outside_safe_exemptions() {
        let source = "\
#!/bin/sh
keep_result() {
  if [ \"$x\" ]; then
    :
  else
    \\typeset __result=$?
    unset y
    return $__result
  fi
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?"]
        );
    }

    #[test]
    fn reports_sequential_followups_after_test_statements() {
        let source = "\
#!/bin/bash
[[ \"${second_line}\" == \"quz\" ]];
tend $?
[[ ${s0} == \"${s2}\" ]] &&
[[ ${s1} != *f* ]]
tend $?
[[ \"${later}\" == \"ok\" ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?"]
        );
    }

    #[test]
    fn reports_sequential_followups_before_if_tail_conditions() {
        let source = "\
#!/bin/bash
[[ \"${second_line}\" == \"quz\" ]]
tend $?
if [ -f later_if ]; then
  :
elif [ -f later_elif ]; then
  :
fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?"]
        );
    }

    #[test]
    fn ignores_completion_status_after_if_and_while_commands() {
        let source = "\
#!/bin/bash
if [ -f foo ]; then :; fi
saved=$?
while [ -f foo ]; do break; done
again=$?
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_one_off_sequential_test_followups_without_later_test_blocks() {
        let source = "\
#!/bin/bash
[[ \"${second_line}\" == \"quz\" ]]
tend $?
nextcmd
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn suppresses_only_precise_function_return_guard_patterns() {
        let source = "\
#!/bin/bash
build_config() {
  helper || return $?
  [[ -n \"${flag:-}\" ]] || return $?
  export out=\"$(printf ok)\"
}
pkg_check() {
  [[ -f \"${base}/include/$1\" ]] || return $?
  case \"$mode\" in
    a) ext=a ;;
    *) ext=b ;;
  esac
  file=\"$(find \"${base}\" -name \"$2.$ext\" | head -n 1)\"
  [[ -n \"$file\" ]] || return $?
}
macruby_install_extract_pkg()
(
  one=\"$(mktemp -d)\"
  two=\"$(mktemp -d)\"
  [[ -n \"$one\" && -d \"$one\" && -n \"$two\" && -d \"$two\" ]] || return $?
  ok=1
)
[[ -n \"$top\" ]] || return $?
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?", "$?", "$?"]
        );
    }

    #[test]
    fn suppresses_accumulator_style_return_guards_without_hiding_plain_complex_guards() {
        let source = "\
#!/bin/bash
macruby_install_extract_pkg()
(
  set -x
  \typeset __source __target __temp1 __temp2 __result
  __source=\"$1\"
  __target=\"$2\"
  __temp1=\"$(mktemp -d)\"
  __temp2=\"$(mktemp -d)\"
  [[ -n \"${__temp1}\" && -d \"${__temp1}\" && -n \"${__temp2}\" && -d \"${__temp2}\" ]] || return $?
  __result=0
  pkgutil --expand \"${__source}\" \"${__temp1}\" || __result=$?
  [[ -n \"${__temp1}\" ]] &&
  mv -f \"${__temp2}\"/* \"${__target}/\" ||
  __result=$?
  return ${__result}
)
plain_complex_guard()
(
  one=\"$(mktemp -d)\"
  two=\"$(mktemp -d)\"
  [[ -n \"$one\" && -d \"$one\" && -n \"$two\" && -d \"$two\" ]] || return $?
  ok=1
)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$?"]
        );
    }

    #[test]
    fn suppresses_precise_function_return_guards_in_case_bodies() {
        let source = "\
#!/bin/bash
pkg_check() {
  case \"$mode\" in
    a)
      helper || return $?
      [[ -n \"$path\" ]] || return $?
      ;;
  esac
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::StatusCaptureAfterBranchTest),
        );

        assert!(diagnostics.is_empty());
    }
}

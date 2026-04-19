use crate::{Checker, Rule, Violation};

pub struct LegacyBackticks;

impl Violation for LegacyBackticks {
    fn rule() -> Rule {
        Rule::LegacyBackticks
    }

    fn message(&self) -> String {
        "prefer `$(...)` over legacy backtick substitution".to_owned()
    }
}

pub fn legacy_backticks(checker: &mut Checker) {
    let spans = checker
        .facts()
        .backtick_fragments()
        .iter()
        .filter(|fragment| !fragment.is_empty())
        .map(|fragment| fragment.span())
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_dedup(LegacyBackticks, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_each_backtick_fragment() {
        let source = "echo \"prefix `date` suffix `uname`\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`date`", "`uname`"]
        );
    }

    #[test]
    fn ignores_escaped_backticks_inside_double_quotes() {
        let source = "echo \"\\`run\\`'s command \\`%s\\` exited with code 127, indicating 'Command not found'.\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn anchors_a_plain_backtick_substitution_once() {
        let source = "commands=(`pyenv-commands --sh`)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`pyenv-commands --sh`"]
        );
    }

    #[test]
    fn ignores_empty_backtick_markup_inside_double_quotes() {
        let source = "echo \"Resolve the conflict and run ``${PROGRAM} --continue``.\"\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_backticks_inside_multiline_double_quotes_after_line_continuation() {
        let source = "\
ECHO=echo
$ECHO \"\\
*** ERROR
`cat lockfile 2>/dev/null`
\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`cat lockfile 2>/dev/null`"]
        );
    }

    #[test]
    fn reports_backticks_after_a_quoted_heredoc_help_block() {
        let source = "\
cat <<\\_ACEOF
Use `configure' or `make' to build this project.
_ACEOF
x=`pwd`
y=`uname`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["`pwd`", "`uname`"]
        );
    }

    #[test]
    fn reports_multiple_backtick_assignments_with_single_quoted_sed_scripts() {
        let source = "\
ac_dir_suffix=/`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`
ac_top_builddir_sub=`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`",
                "`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`",
            ]
        );
    }

    #[test]
    fn reports_backticks_inside_recursive_help_loop() {
        let source = "\
if test \"$ac_init_help\" = \"recursive\"; then
  for ac_dir in : $ac_subdirs_all; do test \"x$ac_dir\" = x: && continue
    test -d \"$ac_dir\" ||
      { cd \"$srcdir\" && ac_pwd=`pwd` && srcdir=. && test -d \"$ac_dir\"; } ||
      continue
    case \"$ac_dir\" in
    *)
      ac_dir_suffix=/`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`
      ac_top_builddir_sub=`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`
      ;;
    esac
  done
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "`pwd`",
                "`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`",
                "`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`",
            ]
        );
    }

    #[test]
    fn reports_sed_backticks_after_quoted_heredoc_backticks() {
        let source = "\
cat <<\\_ACEOF
Use these variables to override the choices made by `configure' or to help
it to find libraries and programs with nonstandard names/locations.
_ACEOF
ac_dir_suffix=/`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`
ac_top_builddir_sub=`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "`$as_echo \"$ac_dir\" | sed 's|^\\.[\\\\/]||'`",
                "`$as_echo \"$ac_dir_suffix\" | sed 's|/[^\\\\/]*|/..|g;s|/||'`",
            ]
        );
    }

    #[test]
    fn ignores_escaped_backticks_in_case_patterns() {
        let source = "\
case \"$ch\" in
  \\`)
    printf '%s\\n' literal
    ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_escaped_backticks_after_nested_expansions_in_double_quotes() {
        let source = "\
echo \"::error ::Failed to reuse PR #${PR:-} ${WORKFLOW_ID:+\"(workflow run ${WORKFLOW_ID})\"} build artifacts, see \\`Gathering build summary\\` step logs.\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LegacyBackticks));

        assert!(diagnostics.is_empty());
    }
}

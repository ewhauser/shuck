use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation};

pub struct RedundantSpacesInEcho;

impl Violation for RedundantSpacesInEcho {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::RedundantSpacesInEcho
    }

    fn message(&self) -> String {
        "quote repeated spaces to avoid them collapsing into one".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("collapse repeated spaces between echo arguments".to_owned())
    }
}

pub fn redundant_spaces_in_echo(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .redundant_echo_space_facts()
        .iter()
        .map(|fact| {
            crate::Diagnostic::new(RedundantSpacesInEcho, fact.diagnostic_span()).with_fix(
                Fix::safe_edits(
                    fact.space_spans()
                        .iter()
                        .copied()
                        .map(|span| Edit::replacement(" ", span)),
                ),
            )
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic(diagnostic);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect, assert_diagnostics_diff};

    #[test]
    fn reports_repeated_spaces_between_echo_arguments() {
        let source = "\
#!/bin/bash
echo foo    bar
echo -n    \"foo\"
echo \"foo\"    bar
echo foo    \"bar\"
echo foo  bar
echo    foo
command echo foo    bar
builtin echo foo    bar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "echo foo    bar",
                "echo -n    \"foo\"",
                "echo \"foo\"    bar",
                "echo foo    \"bar\""
            ]
        );
    }

    #[test]
    fn ignores_single_argument_and_wrapped_echoes() {
        let source = "\
#!/bin/sh
echo    foo
command echo foo    bar
builtin echo foo    bar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho).with_shell(ShellDialect::Sh),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_backslash_continued_echo_arguments() {
        let source = "\
#!/bin/bash
echo \"pyenv: cannot rehash: couldn't acquire lock\"\\
  \"$PROTOTYPE_SHIM_PATH for $PYENV_REHASH_TIMEOUT seconds. Last error message:\" >&2
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_backslash_continued_echo_arguments_in_nested_control_flow() {
        let source = "\
#!/usr/bin/env bash
    if [[ -z $tested_for_other_write_errors ]]; then
      ( t=\"$(TMPDIR=\"$SHIM_PATH\" mktemp)\" && rm \"$t\" ) && tested_for_other_write_errors=1 ||
        { echo \"pyenv: cannot rehash: $SHIM_PATH isn't writable\" >&2; break; }
    fi
    # POSIX sleep(1) doesn't provide subsecond precision, but many others do
    sleep 0.1 2>/dev/null || sleep 1
  fi
done

if [ -z \"${acquired}\" ]; then
  if [[ -n $tested_for_other_write_errors ]]; then
      echo \"pyenv: cannot rehash: couldn't acquire lock\"\\
        \"$PROTOTYPE_SHIM_PATH for $PYENV_REHASH_TIMEOUT seconds. Last error message:\" >&2
      echo \"$last_acquire_error\" >&2
  fi
  exit 1
fi
unset tested_for_other_write_errors
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_repeated_spaces_after_utf8_argument() {
        let source = "\
#!/bin/bash
echo café    bar
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["echo café    bar"]
        );
    }

    #[test]
    fn applies_safe_fix_to_repeated_echo_spaces() {
        let source = "\
#!/bin/bash
echo foo    bar
echo -n    \"foo\"
echo a    b    c
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
echo foo bar
echo -n \"foo\"
echo a b c
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_non_redundant_echo_spacing_unchanged_when_fixing() {
        let source = "\
#!/bin/sh
echo foo  bar
echo    foo
command echo foo    bar
builtin echo foo    bar
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho).with_shell(ShellDialect::Sh),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("style").join("S037.sh").as_path(),
            &LinterSettings::for_rule(Rule::RedundantSpacesInEcho),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("S037_fix_S037.sh", result);
        Ok(())
    }
}

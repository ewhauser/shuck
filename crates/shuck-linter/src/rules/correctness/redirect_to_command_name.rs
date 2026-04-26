use shuck_ast::RedirectKind;

use crate::{Checker, Edit, Fix, FixAvailability, Rule, Violation, WordQuote};

pub struct RedirectToCommandName;

impl Violation for RedirectToCommandName {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::RedirectToCommandName
    }

    fn message(&self) -> String {
        "redirection target matches a command name; use a distinct file path".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("prefix the redirection target with `./`".to_owned())
    }
}

pub fn redirect_to_command_name(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| fact.redirect_facts().iter())
        .filter_map(|redirect| {
            let kind = redirect.kind();
            if !matches!(
                kind,
                RedirectKind::Output
                    | RedirectKind::Clobber
                    | RedirectKind::Append
                    | RedirectKind::Input
                    | RedirectKind::ReadWrite
                    | RedirectKind::OutputBoth
            ) {
                return None;
            }

            let analysis = redirect.analysis()?;
            if !analysis.is_file_target() {
                return None;
            }
            if !analysis.expansion.is_fixed_literal() || analysis.is_runtime_sensitive() {
                return None;
            }
            if analysis.expansion.quote != WordQuote::Unquoted {
                return None;
            }

            let span = redirect.target_span()?;
            let name = span.slice(source);
            if name.is_empty() || name.contains('/') || !is_shadowed_command_name(name) {
                return None;
            }

            Some(span)
        })
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic_dedup(
            crate::Diagnostic::new(RedirectToCommandName, span)
                .with_fix(Fix::safe_edit(Edit::insertion(span.start.offset, "./"))),
        );
    }
}

fn is_shadowed_command_name(name: &str) -> bool {
    matches!(
        name,
        "alias"
            | "awk"
            | "basename"
            | "bg"
            | "break"
            | "c99"
            | "cat"
            | "cd"
            | "chmod"
            | "chown"
            | "cksum"
            | "cmp"
            | "comm"
            | "command"
            | "continue"
            | "cp"
            | "csplit"
            | "cut"
            | "date"
            | "dd"
            | "df"
            | "dirname"
            | "du"
            | "echo"
            | "env"
            | "eval"
            | "exec"
            | "exit"
            | "expand"
            | "expr"
            | "fg"
            | "find"
            | "fold"
            | "getopts"
            | "grep"
            | "hash"
            | "head"
            | "jobs"
            | "join"
            | "kill"
            | "link"
            | "ln"
            | "ls"
            | "m4"
            | "make"
            | "mkdir"
            | "mkfifo"
            | "more"
            | "mv"
            | "nice"
            | "nl"
            | "nohup"
            | "od"
            | "paste"
            | "pathchk"
            | "printf"
            | "pwd"
            | "read"
            | "readonly"
            | "renice"
            | "return"
            | "rm"
            | "rmdir"
            | "sed"
            | "set"
            | "shift"
            | "sh"
            | "sleep"
            | "sort"
            | "split"
            | "strings"
            | "tail"
            | "test"
            | "time"
            | "touch"
            | "tr"
            | "trap"
            | "tty"
            | "type"
            | "ulimit"
            | "umask"
            | "unalias"
            | "uname"
            | "unexpand"
            | "uniq"
            | "unlink"
            | "unset"
            | "wait"
            | "wc"
            | "xargs"
            | "zcat"
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_command_named_redirect_targets() {
        let source = "\
#!/bin/bash
cat input > c99
cat input >> grep
cat input 2> sed
cat input < awk
cat input <> basename
cat input >| command
cat input &> printf
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectToCommandName),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3, 4, 5, 6, 7, 8]
        );
    }

    #[test]
    fn exposes_safe_fix_metadata_for_reported_targets() {
        let source = "#!/bin/bash\ncat input > c99\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectToCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "c99");
        assert_eq!(
            diagnostics[0].fix.as_ref().map(|fix| fix.applicability()),
            Some(Applicability::Safe)
        );
        assert_eq!(
            diagnostics[0].fix_title.as_deref(),
            Some("prefix the redirection target with `./`")
        );
    }

    #[test]
    fn applies_safe_fix_to_command_named_redirect_targets() {
        let source = "\
#!/bin/bash
cat input > c99
cat input >> grep
cat input 2> sed
cat input < awk
cat input <> basename
cat input >| command
cat input &> printf
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::RedirectToCommandName),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 7);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\ncat input > ./c99\ncat input >> ./grep\ncat input 2> ./sed\ncat input < ./awk\ncat input <> ./basename\ncat input >| ./command\ncat input &> ./printf\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn leaves_qualified_and_dynamic_redirect_targets_unchanged_when_fixing() {
        let source = "\
#!/bin/bash
cat input > \"cat\"
cat input > ./cat
cat input > /tmp/cat
cat input > cat.txt
cat input > \"$name\"
cat input > ${name}
cat input <<< cat
cat input > true
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::RedirectToCommandName),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn leaves_qualified_and_dynamic_redirect_targets_unchanged_when_applying_fixes() {
        let source = "\
#!/bin/bash
cat input > \"cat\"
cat input > ./cat
cat input > /tmp/cat
cat input > cat.txt
cat input > \"$name\"
cat input > ${name}
cat input <<< cat
cat input > true
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::RedirectToCommandName),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn snapshots_safe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C059.sh").as_path(),
            &LinterSettings::for_rule(Rule::RedirectToCommandName),
            Applicability::Safe,
        )?;

        assert_diagnostics_diff!("C059_fix_C059.sh", result);
        Ok(())
    }
}

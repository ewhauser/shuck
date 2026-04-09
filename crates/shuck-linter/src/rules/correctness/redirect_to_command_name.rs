use shuck_ast::RedirectKind;

use crate::{Checker, Rule, Violation, WordQuote};

pub struct RedirectToCommandName;

impl Violation for RedirectToCommandName {
    fn rule() -> Rule {
        Rule::RedirectToCommandName
    }

    fn message(&self) -> String {
        "redirection target matches a command name; use a distinct file path".to_owned()
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
            let kind = redirect.redirect().kind;
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

    checker.report_all_dedup(spans, || RedirectToCommandName);
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
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_command_named_redirect_targets() {
        let source = "\
#!/bin/bash
cat input > c99
cat input >> grep
cat input 2> sed
cat input < awk
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
            vec![2, 3, 4, 5, 6, 7]
        );
    }

    #[test]
    fn ignores_quoted_paths_dynamic_targets_and_non_command_names() {
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
}

use crate::{Checker, Rule, ShellDialect, Violation};
use shuck_ast::{Command, Span};
use shuck_semantic::{ScopeId, ScopeKind};

pub struct UncheckedDirectoryChange {
    pub command: &'static str,
}

impl Violation for UncheckedDirectoryChange {
    fn rule() -> Rule {
        Rule::UncheckedDirectoryChange
    }

    fn message(&self) -> String {
        format!(
            "`{}` should check whether the directory change succeeded",
            self.command
        )
    }
}

pub fn unchecked_directory_change(checker: &mut Checker) {
    if !supports_directory_change_rules(checker.shell()) {
        return;
    }

    for (command, span) in unchecked_directory_change_impl(checker, false) {
        checker.report(UncheckedDirectoryChange { command }, span);
    }
}

pub(crate) fn unchecked_directory_change_in_function_spans(
    checker: &mut Checker,
) -> Vec<(&'static str, Span)> {
    if !supports_directory_change_rules(checker.shell()) {
        return Vec::new();
    }

    unchecked_directory_change_impl(checker, true)
}

fn unchecked_directory_change_impl(
    checker: &mut Checker,
    inside_function_only: bool,
) -> Vec<(&'static str, Span)> {
    let semantic = checker.semantic();
    let source = checker.source();
    let errexit_enabled_somewhere = checker.facts().errexit_enabled_anywhere();
    checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            let scope = semantic.scope_at(fact.stmt().span.start.offset);
            if fact.is_nested_word_command()
                && !matches!(semantic.scope_kind(scope), ScopeKind::CommandSubstitution)
            {
                return None;
            }

            let directory_change = fact.options().directory_change()?;
            let inside_function = direct_function_scope(semantic, scope).is_some();
            if inside_function_only && !inside_function {
                return None;
            }
            if !inside_function_only
                && inside_function
                && checker.is_rule_enabled(Rule::UncheckedDirectoryChangeInFunction)
            {
                return None;
            }
            let unchecked = semantic
                .flow_context_at(&fact.stmt().span)
                .map(|context| !context.exit_status_checked)
                .unwrap_or(true);

            (unchecked
                && !errexit_enabled_somewhere
                && !directory_change.is_plain_directory_stack_marker())
            .then_some((directory_change.command_name(), report_span(fact, source)))
        })
        .collect::<Vec<_>>()
}

fn report_span(fact: &crate::facts::CommandFact<'_>, source: &str) -> Span {
    match fact.command() {
        Command::Simple(command) => {
            let mut start = command.name.span.start;
            if command.name.span.slice(source).starts_with('\\') {
                start = start.advanced_by("\\");
            }
            let end = command
                .args
                .last()
                .map(|word| word.span.end)
                .into_iter()
                .chain(fact.redirects().iter().map(|redirect| redirect.span.end))
                .max_by_key(|position| position.offset)
                .unwrap_or(command.name.span.end);
            Span::from_positions(start, end)
        }
        _ => fact.span(),
    }
}

fn direct_function_scope(
    semantic: &shuck_semantic::SemanticModel,
    mut scope: ScopeId,
) -> Option<ScopeId> {
    loop {
        let current = semantic
            .scopes()
            .iter()
            .find(|candidate| candidate.id == scope)?;
        match &current.kind {
            ScopeKind::Function(_) => return Some(scope),
            ScopeKind::CommandSubstitution | ScopeKind::Subshell => return None,
            _ => match current.parent {
                Some(parent) => scope = parent,
                None => return None,
            },
        }
    }
}

fn supports_directory_change_rules(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Unknown
            | ShellDialect::Sh
            | ShellDialect::Bash
            | ShellDialect::Dash
            | ShellDialect::Ksh
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_plain_cd_commands() {
        let source = "cd /tmp\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cd /tmp");
    }

    #[test]
    fn ignores_checked_directory_changes() {
        let source = "\
if cd /tmp; then
\tpwd
fi
if ! builtin cd /var; then
\treturn 1
fi
cd /tmp || exit 1
cd /tmp && pwd
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_cd_in_following_list_position() {
        let source = "echo start && cd /tmp\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cd /tmp");
    }

    #[test]
    fn reports_pushd_popd_escaped_and_wrapped_cd() {
        let source = "\
pushd /tmp
popd
\\cd /var >/dev/null
builtin cd /opt
cd /srv # inline comment
pushd /work >/dev/null
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "pushd /tmp",
                "popd",
                "cd /var >/dev/null",
                "builtin cd /opt",
                "cd /srv",
                "pushd /work >/dev/null"
            ]
        );
    }

    #[test]
    fn ignores_directory_changes_when_errexit_is_enabled_anywhere() {
        let source = "\
cd /start
set -e
cd /tmp
set +e
cd /var
set -o errexit
pushd /opt
set +o errexit
popd
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_directory_stack_markers() {
        let source = "\
cd .
cd ..
cd /
cd ../..
cd ./../..
pushd .
pushd ..
pushd /
pushd ../..
popd
cd ./child
cd ../../child
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["popd", "cd ./child", "cd ../../child"]
        );
    }

    #[test]
    fn wrapped_directory_stack_markers_still_report() {
        let source = "\
builtin cd ..
command cd /
\\cd ..
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["builtin cd ..", "command cd /"]
        );
    }

    #[test]
    fn ignores_directory_changes_when_shebang_enables_errexit() {
        let source = "\
#!/bin/bash -eux
cd /tmp
pushd /var
popd
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn shebang_without_errexit_still_reports() {
        let source = "\
#!/bin/bash -u
cd /tmp
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cd /tmp");
    }

    #[test]
    fn ignores_errexit_shebang_after_leading_blank_lines() {
        let source = "\n#!/bin/bash -eu\ncd /tmp\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_nested_cd_inside_command_substitutions() {
        let source = "path=\"$( \\cd /tmp>/dev/null; pwd )\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cd /tmp>/dev/null");
    }

    #[test]
    fn reports_directory_changes_inside_functions_when_specific_rule_is_disabled() {
        let source = "\
#!/bin/sh
f() {
\tcd /tmp
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cd /tmp");
    }

    #[test]
    fn nested_command_substitutions_inside_functions_still_use_the_general_rule() {
        let source = "\
#!/bin/sh
f() {
\tpath=\"$(cd /tmp; pwd)\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([
                Rule::UncheckedDirectoryChange,
                Rule::UncheckedDirectoryChangeInFunction,
            ]),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::UncheckedDirectoryChange);
        assert_eq!(diagnostics[0].span.slice(source), "cd /tmp");
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "\
#!/bin/zsh
cd /tmp
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChange).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

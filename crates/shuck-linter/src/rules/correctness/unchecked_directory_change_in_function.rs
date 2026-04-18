use std::collections::HashMap;

use crate::{Checker, Rule, Violation};
use shuck_semantic::{ScopeId, ScopeKind};

use super::unchecked_directory_change::{report_span, supports_directory_change_rules};

pub struct UncheckedDirectoryChangeInFunction {
    pub command: &'static str,
}

impl Violation for UncheckedDirectoryChangeInFunction {
    fn rule() -> Rule {
        Rule::UncheckedDirectoryChangeInFunction
    }

    fn message(&self) -> String {
        format!(
            "`{}` should prefer a subshell over manually changing back",
            self.command
        )
    }
}

pub fn unchecked_directory_change_in_function(checker: &mut Checker) {
    if !supports_directory_change_rules(checker.shell())
        || checker.facts().errexit_enabled_anywhere()
    {
        return;
    }

    let semantic = checker.semantic();
    let source = checker.source();
    let mut pending_unchecked_cd_by_scope = HashMap::<ScopeId, usize>::new();
    let mut reported_scope = std::collections::HashSet::<ScopeId>::new();
    let mut reports = Vec::new();

    for fact in checker.facts().commands() {
        let scope = semantic.scope_at(fact.stmt().span.start.offset);
        if fact.is_nested_word_command()
            && !matches!(semantic.scope_kind(scope), ScopeKind::CommandSubstitution)
        {
            continue;
        }
        let Some(directory_change) = fact.options().directory_change() else {
            continue;
        };
        if directory_change.kind() != crate::facts::DirectoryChangeCommandKind::Cd {
            continue;
        }

        let unchecked = semantic
            .flow_context_at(&fact.stmt().span)
            .map(|context| !context.exit_status_checked)
            .unwrap_or(true);
        let pending_unchecked_cd = pending_unchecked_cd_by_scope.entry(scope).or_default();

        if directory_change.is_manual_restore_candidate() {
            if *pending_unchecked_cd > 0 {
                if unchecked && fact.wrappers().is_empty() && reported_scope.insert(scope) {
                    reports.push((directory_change.command_name(), report_span(fact, source)));
                }
                *pending_unchecked_cd -= 1;
            }
            continue;
        }

        if unchecked && !directory_change.is_plain_directory_stack_marker() {
            *pending_unchecked_cd += 1;
        }
    }

    for (command, span) in reports {
        checker.report(UncheckedDirectoryChangeInFunction { command }, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_manual_directory_restore_in_functions() {
        let source = "\
#!/bin/sh
f() {
\tcd /tmp
\tpwd
\tcd ..
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].rule,
            Rule::UncheckedDirectoryChangeInFunction
        );
        assert_eq!(diagnostics[0].span.slice(source), "cd ..");
    }

    #[test]
    fn reports_manual_directory_restore_at_top_level() {
        let source = "\
#!/bin/sh
cd /tmp
pwd
cd -
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cd -");
    }

    #[test]
    fn ignores_manual_restore_when_initial_cd_is_checked() {
        let source = "\
#!/bin/sh
f() {
\tcd /tmp || return
\tpwd
\tcd ..
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_manual_restore_when_the_restore_is_checked() {
        let source = "\
#!/bin/sh
f() {
\tcd /tmp
\tpwd
\tcd .. || return
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_wrapped_manual_restore_commands() {
        let source = "\
#!/bin/sh
f() {
\tbuiltin cd /tmp
\tpwd
\tbuiltin cd ..
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn only_reports_the_first_manual_restore_per_scope() {
        let source = "\
#!/bin/sh
f() {
\tcd /tmp
\tpwd
\tcd ..
\tcd /var
\tpwd
\tcd ..
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cd ..");
    }

    #[test]
    fn reports_manual_restore_inside_subshells() {
        let source = "\
#!/bin/sh
(
\tcd /tmp
\tpwd
\tcd ..
)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cd ..");
    }

    #[test]
    fn reports_manual_restore_inside_command_substitutions() {
        let source = "\
#!/bin/sh
root=$(cd \"$(dirname \"$0\")\"; cd ..; pwd)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "cd ..");
    }

    #[test]
    fn reports_cd_dash_when_both_rules_are_enabled() {
        let source = "\
#!/bin/sh
f() {
\tcd /tmp
\tpwd
\tcd - >/dev/null
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rules([
                Rule::UncheckedDirectoryChange,
                Rule::UncheckedDirectoryChangeInFunction,
            ]),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.rule, diagnostic.span.slice(source)))
                .collect::<Vec<_>>(),
            vec![
                (Rule::UncheckedDirectoryChange, "cd /tmp"),
                (Rule::UncheckedDirectoryChange, "cd - >/dev/null"),
                (Rule::UncheckedDirectoryChangeInFunction, "cd - >/dev/null")
            ]
        );
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "\
#!/bin/zsh
f() {
\tcd /tmp
\tpwd
\tcd ..
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UncheckedDirectoryChangeInFunction)
                .with_shell(crate::ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

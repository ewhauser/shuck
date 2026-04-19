use crate::{Checker, Rule, Violation};

pub struct FindExecDirWithShell;

impl Violation for FindExecDirWithShell {
    fn rule() -> Rule {
        Rule::FindExecDirWithShell
    }

    fn message(&self) -> String {
        "shell command text passed through `find -exec` or `-execdir` can inject filenames"
            .to_owned()
    }
}

pub fn find_execdir_with_shell(checker: &mut Checker) {
    let spans = checker
        .facts()
        .structural_commands()
        .filter_map(|fact| fact.options().find_exec_shell())
        .flat_map(|fact| fact.shell_command_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || FindExecDirWithShell);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_sh_c_exec_shell_interpolation() {
        let source = "#!/bin/sh\nfind . -exec sh -c 'printf \"%s\\n\" {}' \\;\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindExecDirWithShell),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::FindExecDirWithShell);
        assert_eq!(diagnostics[0].span.slice(source), "'printf \"%s\\n\" {}'");
    }

    #[test]
    fn reports_bash_c_execdir_shell_interpolation() {
        let source = "#!/bin/sh\nfind . -execdir bash -c 'printf \"%s\\n\" {}' \\;\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindExecDirWithShell),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::FindExecDirWithShell);
        assert_eq!(diagnostics[0].span.slice(source), "'printf \"%s\\n\" {}'");
    }

    #[test]
    fn ignores_find_exec_shell_forms_without_shell_interpolation() {
        let source = "#!/bin/sh\nfind . -exec mv -- {} renamed-file \\;\nfind . -exec sh ./rename-helper {} \\;\nfind . -exec sh -c 'printf safe\\n' \\;\nfind . -execdir mv -- {} renamed-file \\;\nfind . -execdir sh ./rename-helper {} \\;\nfind . -execdir sh -c 'printf safe\\n' \\;\nfind . -execdir bash -c 'echo safe' \\;\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FindExecDirWithShell),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

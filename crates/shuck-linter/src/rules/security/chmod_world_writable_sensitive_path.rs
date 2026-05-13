use crate::{Checker, Rule, Violation};

pub struct ChmodWorldWritableSensitivePath;

impl Violation for ChmodWorldWritableSensitivePath {
    fn rule() -> Rule {
        Rule::ChmodWorldWritableSensitivePath
    }

    fn message(&self) -> String {
        "`chmod` makes a sensitive path writable by everyone".to_owned()
    }
}

pub fn chmod_world_writable_sensitive_path(checker: &mut Checker) {
    let spans = checker
        .facts()
        .command_facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("chmod"))
        .filter_map(|fact| fact.options().chmod())
        .flat_map(|chmod| {
            chmod
                .world_writable_sensitive_path_spans(checker.source())
                .iter()
                .copied()
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || ChmodWorldWritableSensitivePath);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_world_writable_sensitive_targets() {
        let source = "#!/bin/sh\nchmod -R 777 ~/.ssh\nchmod o+w \"$HOME/.gnupg\"\nchmod a=rw /etc/ssh/sshd_config\nchmod =777 ~/.ssh/config\nchmod +002 ~/.aws\nchmod o-w,o+w ~/.kube\nchmod u+w,o=u ~/.docker\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ChmodWorldWritableSensitivePath),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "~/.ssh",
                "\"$HOME/.gnupg\"",
                "/etc/ssh/sshd_config",
                "~/.ssh/config",
                "~/.aws",
                "~/.kube",
                "~/.docker"
            ]
        );
    }

    #[test]
    fn ignores_bounded_or_non_world_writable_chmod_targets() {
        let source = "#!/bin/sh\nchmod 700 ~/.ssh\nchmod 755 ~/.ssh\nchmod u+w ~/.ssh/config\nchmod +w ~/.ssh/config\nchmod o+w,o-w ~/.ssh\nchmod a=rw,o-w ~/.gnupg\nchmod u-w,o=u ~/.docker\nchmod -002 ~/.aws\nchmod 777 ./tmp\nchmod 777 /tmp\nchmod --reference ref ~/.ssh\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ChmodWorldWritableSensitivePath),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

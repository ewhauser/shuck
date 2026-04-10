use crate::{Checker, Rule, Violation};

pub struct SshLocalExpansion;

impl Violation for SshLocalExpansion {
    fn rule() -> Rule {
        Rule::SshLocalExpansion
    }

    fn message(&self) -> String {
        "ssh command text is expanded locally before the remote shell sees it".to_owned()
    }
}

pub fn ssh_local_expansion(checker: &mut Checker) {
    let spans = checker
        .facts()
        .structural_commands()
        .filter_map(|fact| fact.options().ssh())
        .flat_map(|fact| fact.local_expansion_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SshLocalExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_expansions_in_destination_arguments() {
        let source = "\
#!/bin/sh
ssh
ssh \"$host\"
ssh \"$host\" printf '%s\\n' ok
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_expansions_in_remote_command_arguments() {
        let source = "\
#!/bin/sh
ssh \"$host\" \"echo $HOME\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "$HOME");
    }

    #[test]
    fn reports_expansions_after_static_ssh_options() {
        let source = "\
#!/bin/sh
ssh -i \"$key\" \"$host\" \"echo $HOME\"
ssh -p 2222 host \"echo $USER\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "$HOME");
        assert_eq!(diagnostics[1].span.slice(source), "$USER");
    }
}

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
        .commands()
        .iter()
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
    fn ignores_local_ssh_options_before_destination() {
        let source = "\
#!/bin/sh
ssh -i \"$key\" \"$host\" \"echo $HOME\"
ssh -o BatchMode=yes host \"echo $USER\"
ssh -- host \"echo $PATH\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_non_terminal_and_assignment_style_remote_expansions() {
        let source = "\
#!/bin/sh
ssh \"$host\" cmd \"$HOME\" --force
ssh \"$host\" HELLO=\"$HOME\"
ssh \"$host\" foo=\"$USER\" bar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_only_the_final_fully_quoted_remote_argument() {
        let source = "\
#!/bin/sh
ssh \"$host\" \"$HOME\" \"$USER\"
ssh \"$host\" cmd \"$HOME\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "$USER");
        assert_eq!(diagnostics[1].span.slice(source), "$HOME");
    }

    #[test]
    fn ignores_remote_command_shapes_with_leading_dash_arguments() {
        let source = "\
#!/bin/sh
ssh host -t \"echo $HOME\"
ssh host ls -l \"$HOME\"
ssh host cmd --flag \"$HOME\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_expansions_inside_command_substitutions() {
        let source = "\
#!/bin/sh
URL=$(ssh \"$host\" url \"$REPO\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "$REPO");
    }
}

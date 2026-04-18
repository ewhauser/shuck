use crate::{Checker, Rule, Violation};

pub struct EnvPrefixCommandOnly;

impl Violation for EnvPrefixCommandOnly {
    fn rule() -> Rule {
        Rule::EnvPrefixCommandOnly
    }

    fn message(&self) -> String {
        "this command-prefix assignment is not visible to later expansions on the same command"
            .to_owned()
    }
}

pub fn env_prefix_command_only(checker: &mut Checker) {
    checker.report_all_dedup(
        checker.facts().env_prefix_assignment_scope_spans().to_vec(),
        || EnvPrefixCommandOnly,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_command_prefix_assignments_reused_later_in_the_same_command() {
        let source = "\
#!/bin/bash
CFLAGS=\"${SLKCFLAGS}\" ./configure --with-optmizer=${CFLAGS}
PATH=/tmp \"$PATH\"/bin/tool
A=1 B=\"$A\" C=\"$B\" cmd
foo=\"$foo\" bar=\"$foo\" cmd
foo=1 export \"$foo\"
foo=1 bar[$foo]=x cmd
COUNTDOWN=$[ $COUNTDOWN - 1 ] echo \"$COUNTDOWN\"
X=1 A=$[ $X + 1 ] true
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EnvPrefixCommandOnly),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "CFLAGS",
                "PATH",
                "A",
                "B",
                "foo",
                "foo",
                "foo",
                "COUNTDOWN",
                "X"
            ]
        );
    }

    #[test]
    fn ignores_assignments_without_a_later_same_command_reference() {
        let source = "\
#!/bin/bash
foo=1 echo hi
foo=\"$foo\" cmd
foo=1 cmd \"$(printf %s \"$foo\")\"
foo=1 foo=2 cmd
foo=1 bar=\"$foo\"
COUNTDOWN=$[ $COUNTDOWN - 1 ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EnvPrefixCommandOnly),
        );

        assert!(diagnostics.is_empty());
    }
}

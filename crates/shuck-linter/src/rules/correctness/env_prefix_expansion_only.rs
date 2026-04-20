use crate::{Checker, Rule, Violation};

pub struct EnvPrefixExpansionOnly;

impl Violation for EnvPrefixExpansionOnly {
    fn rule() -> Rule {
        Rule::EnvPrefixExpansionOnly
    }

    fn message(&self) -> String {
        "this same-command expansion still sees the earlier shell value".to_owned()
    }
}

pub fn env_prefix_expansion_only(checker: &mut Checker) {
    checker.report_all_dedup(
        checker.facts().env_prefix_expansion_scope_spans().to_vec(),
        || EnvPrefixExpansionOnly,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_later_expansions_that_cannot_see_prefix_assignments() {
        let source = "\
#!/bin/bash
CFLAGS=\"${SLKCFLAGS}\" ./configure --with-optmizer=${CFLAGS}
PATH=/tmp \"$PATH\"/bin/tool
A=1 B=\"$A\" C=\"$B\" cmd
foo=1 export \"$foo\"
foo=1 bar[$foo]=x cmd
COUNTDOWN=$[ $COUNTDOWN - 1 ] echo \"$COUNTDOWN\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EnvPrefixExpansionOnly),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${CFLAGS}",
                "$PATH",
                "$A",
                "$B",
                "$foo",
                "$foo",
                "$COUNTDOWN"
            ]
        );
    }

    #[test]
    fn ignores_nested_commands_assignment_only_forms_and_redirects() {
        let source = "\
#!/bin/bash
foo=1 echo hi
foo=\"$foo\" cmd
foo=1 cmd \"$(printf %s \"$foo\")\"
foo=1 foo=2 cmd
foo=1 bar=\"$foo\"
FOO=tmp cmd >\"$FOO\"
COUNTDOWN=$[ $COUNTDOWN - 1 ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::EnvPrefixExpansionOnly),
        );

        assert!(diagnostics.is_empty());
    }
}

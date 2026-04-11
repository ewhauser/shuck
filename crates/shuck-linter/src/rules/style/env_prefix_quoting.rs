use shuck_ast::Command;

use crate::{
    Checker, ExpansionContext, Rule, Violation, word_double_quoted_scalar_only_expansion_spans,
};

pub struct EnvPrefixQuoting;

impl Violation for EnvPrefixQuoting {
    fn rule() -> Rule {
        Rule::EnvPrefixQuoting
    }

    fn message(&self) -> String {
        "drop redundant quotes around expansion-only env-prefix assignments".to_owned()
    }
}

pub fn env_prefix_quoting(checker: &mut Checker) {
    let facts = checker.facts();
    let spans = facts
        .expansion_word_facts(ExpansionContext::AssignmentValue)
        .filter(|fact| {
            let command = facts.command(fact.command_id());
            matches!(command.command(), Command::Simple(_))
                && {
                    let body_span = command.body_span();
                    body_span.start.offset < body_span.end.offset
                }
        })
        .filter(|fact| !fact.analysis().has_array_expansion())
        .flat_map(|fact| word_double_quoted_scalar_only_expansion_spans(fact.word()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || EnvPrefixQuoting);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_expansion_only_env_prefix_assignments() {
        let source = "\
#!/bin/bash
CFLAGS=\"${SLKCFLAGS}\" ./configure --target=\"$ARCH\"
A=\"$a\" B=\"${b:-fallback}\" cmd
C=\"$left\"\"$right\" command run
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EnvPrefixQuoting));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${SLKCFLAGS}", "$a", "${b:-fallback}", "$left", "$right"]
        );
    }

    #[test]
    fn ignores_non_prefix_or_behavior_changing_forms() {
        let source = "\
#!/bin/bash
CFLAGS=$SLKCFLAGS ./configure
CFLAGS=\"~\" ./configure
CFLAGS=\"prefix$SLKCFLAGS\" ./configure
CFLAGS=\"${arr[@]}\" ./configure
export CFLAGS=\"${SLKCFLAGS}\"
CFLAGS=\"${SLKCFLAGS}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::EnvPrefixQuoting));

        assert!(diagnostics.is_empty());
    }
}

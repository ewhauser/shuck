use crate::{Checker, CommandSubstitutionKind, Rule, ShellDialect, Violation};

pub struct BashFileSlurp;

impl Violation for BashFileSlurp {
    fn rule() -> Rule {
        Rule::BashFileSlurp
    }

    fn message(&self) -> String {
        "`$(< file)` is not portable in `sh` scripts".to_owned()
    }
}

pub fn bash_file_slurp(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| {
            fact.substitution_facts()
                .iter()
                .filter(|substitution| substitution.kind() == CommandSubstitutionKind::Command)
                .filter(|substitution| substitution.is_bash_file_slurp())
                .map(|substitution| substitution.span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BashFileSlurp);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_bash_file_slurp_substitutions() {
        let source = "\
#!/bin/sh
a=$(<input.txt)
b=\"$( < spaced.txt )\"
c=$(0< fd.txt)
d=$(<quiet.txt 2>/dev/null)
muted=$(<silent.txt >/dev/null)
closed=$(<closed.txt 0<&-)
skip=$(1< not-stdin.txt)
also_skip=$(<> readwrite.txt)
portable=$(cat < input.txt)
other=$(> out.txt)
assigned=$(foo=bar)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BashFileSlurp));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$(<input.txt)",
                "$( < spaced.txt )",
                "$(0< fd.txt)",
                "$(<quiet.txt 2>/dev/null)",
            ]
        );
    }

    #[test]
    fn ignores_file_slurp_syntax_in_bash() {
        let source = "value=$(<input.txt)\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BashFileSlurp).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct PlusEqualsAppend;

impl Violation for PlusEqualsAppend {
    fn rule() -> Rule {
        Rule::PlusEqualsAppend
    }

    fn message(&self) -> String {
        "`+=` assignment is not portable in `sh` scripts".to_owned()
    }
}

pub fn plus_equals_append(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let spans = checker.facts().plus_equals_assignment_spans().to_vec();
    checker.report_all_dedup(spans, || PlusEqualsAppend);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_plus_equals_operators() {
        let source = "\
#!/bin/sh
x+=64
arr+=(one two)
readonly value+=suffix
index[1+2]+=3
(( i += 1 ))
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::PlusEqualsAppend));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["x", "arr", "value", "index[1+2]"]
        );
    }

    #[test]
    fn ignores_plus_equals_in_bash_scripts() {
        let source = "x+=64\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PlusEqualsAppend).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_plus_equals_in_ksh_scripts() {
        let source = "x+=64\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PlusEqualsAppend).with_shell(ShellDialect::Ksh),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::PlusEqualsAppend);
    }
}

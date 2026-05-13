use crate::{Checker, Rule, ShellDialect, Violation};

pub struct AssignSpecialZero;

impl Violation for AssignSpecialZero {
    fn rule() -> Rule {
        Rule::AssignSpecialZero
    }

    fn message(&self) -> String {
        "the positional zero parameter cannot be assigned".to_owned()
    }
}

pub fn assign_special_zero(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Sh {
        return;
    }

    checker.report_fact_slice_dedup(
        |facts| facts.command_facts().assign_special_zero_spans(),
        || AssignSpecialZero,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_sh_command_names_that_assign_positional_zero() {
        let source = "\
#!/bin/sh
0=demo
f() { 0=demo; }
0=demo env
+0=demo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AssignSpecialZero));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["0=demo", "0=demo", "0=demo", "0=demo"]
        );
    }

    #[test]
    fn ignores_other_shells_and_unknown_shell() {
        for source in ["#!/bin/bash\n0=demo\n", "#!/bin/dash\n0=demo\n", "0=demo\n"] {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::AssignSpecialZero));
            assert!(diagnostics.is_empty());
        }
    }

    #[test]
    fn ignores_declarations_wrappers_and_other_zero_like_words() {
        let source = "\
#!/bin/sh
export 0=demo
readonly 0=demo
command 0=demo
env 0=demo true
a[0]=demo
00=demo
1=demo
0+=demo
\"0=demo\"
echo \"$0\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AssignSpecialZero));

        assert!(diagnostics.is_empty());
    }
}

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct SpaceyAssign;

impl Violation for SpaceyAssign {
    fn rule() -> Rule {
        Rule::SpaceyAssign
    }

    fn message(&self) -> String {
        "assignment spacing makes this run as a command".to_owned()
    }
}

pub fn spacey_assign(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    checker.report_fact_slice_dedup(
        |facts| facts.command_facts().spacey_assignment_spans(),
        || SpaceyAssign,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_variable_like_commands_followed_by_equals_words() {
        let source = "\
#!/bin/sh
name = demo
empty =
joined =demo
read = value
test = foo
time = 1
FOO=1 name = value
time timed = value
time FOO=1 timed = value
f() { inside = \"$value\"; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceyAssign));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "name = demo",
                "empty =",
                "joined =demo",
                "read = value",
                "test = foo",
                "time = 1",
                "name = value",
                "timed = value",
                "timed = value",
                "inside = \"$value\""
            ]
        );
    }

    #[test]
    fn ignores_assignments_declarations_comparisons_and_quoted_equals() {
        let source = "\
#!/bin/sh
name=value
name= value
export name = demo
name == demo
name \"=\" demo
name \\= demo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceyAssign));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn skips_zsh() {
        let source = "\
#!/bin/zsh
name = demo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SpaceyAssign));

        assert!(diagnostics.is_empty());
    }
}

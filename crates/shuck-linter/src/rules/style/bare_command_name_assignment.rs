use crate::{Checker, Rule, Violation};

pub struct BareCommandNameAssignment;

impl Violation for BareCommandNameAssignment {
    fn rule() -> Rule {
        Rule::BareCommandNameAssignment
    }

    fn message(&self) -> String {
        "bare command-like text in an assignment should be quoted or captured with `$(...)`"
            .to_owned()
    }
}

pub fn bare_command_name_assignment(checker: &mut Checker) {
    let spans = checker
        .facts()
        .bare_command_name_assignment_spans()
        .to_vec();

    checker.report_all_dedup(spans, || BareCommandNameAssignment);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_plain_assignments_and_single_assignment_command_prefixes() {
        let source = "\
#!/bin/sh
tool=grep
paths[$path]=set
tool=sh printf '%s\\n' hi
pager=cat \"$1\" -u perl
f() {
  state=sh return 0
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BareCommandNameAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "tool",
                "paths[$path]",
                "tool=sh printf '%s\\n' hi",
                "pager=cat \"$1\" -u perl",
                "state=sh return 0",
            ]
        );
    }

    #[test]
    fn ignores_quoted_dynamic_declaration_and_multi_assignment_forms() {
        let source = "\
#!/bin/bash
tool=\"grep\"
tool=$(grep pattern file)
tool=git
tool=grep other=set printf '%s\\n' hi
f() {
  local scoped=sh
  readonly pinned=sh
  export exported=sh
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BareCommandNameAssignment),
        );

        assert!(diagnostics.is_empty());
    }
}

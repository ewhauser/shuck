use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct BareCommandNameAssignment;

impl Violation for BareCommandNameAssignment {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::BareCommandNameAssignment
    }

    fn message(&self) -> String {
        "bare command-like text in an assignment should be quoted or captured with `$(...)`"
            .to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the assignment value".to_owned())
    }
}

pub fn bare_command_name_assignment(checker: &mut Checker) {
    let source = checker.source();
    checker.report_fact_diagnostics_dedup(|facts, report| {
        for span in facts.bare_command_name_assignment_spans().iter().copied() {
            let diagnostic = Diagnostic::new(BareCommandNameAssignment, span);
            report(match quote_assignment_value_fix(span, source) {
                Some(fix) => diagnostic.with_fix(fix),
                None => diagnostic,
            });
        }
    });
}

fn quote_assignment_value_fix(span: Span, source: &str) -> Option<Fix> {
    let line = source
        .get(span.start.offset..)?
        .split_inclusive('\n')
        .next()?;
    let eq_relative = line.find('=')?;
    let value_start = span.start.offset + eq_relative + 1;
    let value_text = source.get(value_start..)?;
    let value_len = value_text
        .chars()
        .take_while(|ch| !ch.is_whitespace() && *ch != ';')
        .map(char::len_utf8)
        .sum::<usize>();
    if value_len == 0 {
        return None;
    }
    Some(Fix::safe_edits([
        Edit::insertion(value_start, "\""),
        Edit::insertion(value_start + value_len, "\""),
    ]))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

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

    #[test]
    fn applies_safe_fix_by_quoting_literal_assignment_value() {
        let source = "#!/bin/sh\ntool=grep\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::BareCommandNameAssignment),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\ntool=\"grep\"\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn respects_zsh_equals_path_expansion_for_assignment_values() {
        let source = "\
#!/bin/zsh
unsetopt equals
tool==grep run
setopt magic_equal_subst
magic_literal==grep run
setopt equals
path==grep run
setopt magic_equal_subst
magic==grep run
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
            vec!["tool==grep run", "magic_literal==grep run"]
        );
    }
}

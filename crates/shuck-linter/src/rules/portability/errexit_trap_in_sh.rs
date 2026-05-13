use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct ErrexitTrapInSh;

impl Violation for ErrexitTrapInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::ErrexitTrapInSh
    }

    fn message(&self) -> String {
        "`set` trap inheritance flags are not portable in `sh` scripts".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the trap inheritance flag".to_owned())
    }
}

pub fn errexit_trap_in_sh(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Sh {
        return;
    }

    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("set"))
        .flat_map(|fact| {
            fact.options().set().into_iter().flat_map(|set| {
                set.errtrace_flag_spans()
                    .iter()
                    .chain(set.functrace_flag_spans().iter())
                    .copied()
            })
        })
        .collect::<Vec<_>>();

    for span in spans {
        checker.report_diagnostic(
            Diagnostic::new(ErrexitTrapInSh, span)
                .with_fix(errexit_trap_fix(checker.source(), span)),
        );
    }
}

fn errexit_trap_fix(source: &str, span: shuck_ast::Span) -> Fix {
    let text = span.slice(source);
    let mut chars = text.chars();
    let Some(sign @ ('-' | '+')) = chars.next() else {
        return Fix::unsafe_edit(Edit::deletion(span));
    };
    let retained = chars
        .filter(|ch| !matches!(ch, 'E' | 'T'))
        .collect::<String>();
    if retained.is_empty() {
        Fix::unsafe_edit(Edit::deletion(span))
    } else {
        Fix::unsafe_edit(Edit::replacement(format!("{sign}{retained}"), span))
    }
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_nonportable_trap_inheritance_flags_in_sh() {
        let source = "\
#!/bin/sh
set -E
set +T
set -ET
set -o errtrace
set +o functrace
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ErrexitTrapInSh));

        assert_eq!(diagnostics.len(), 4);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-E", "+T", "-ET", "-ET"]
        );
    }

    #[test]
    fn ignores_bash_shells() {
        let source = "\
#!/bin/bash
set -E
set -T
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ErrexitTrapInSh));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_positional_operands_after_double_dash() {
        let source = "\
#!/bin/sh
set -E -T -- +E +T
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ErrexitTrapInSh));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["-E", "-T"]
        );
    }

    #[test]
    fn applies_unsafe_fix_to_remove_nonportable_trap_flags() {
        let source = "#!/bin/sh\nset -E\nset -eE\nset -ET\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::ErrexitTrapInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 3);
        assert_eq!(result.fixed_source, "#!/bin/sh\nset \nset -e\nset \n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}

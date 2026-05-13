use rustc_hash::FxHashSet;
use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation};

pub struct DefaultValueInColonAssign;

impl Violation for DefaultValueInColonAssign {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::DefaultValueInColonAssign
    }

    fn message(&self) -> String {
        "quote default-assignment expansions passed to ':' to avoid splitting and globbing"
            .to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the default-assignment expansion".to_owned())
    }
}

pub fn default_value_in_colon_assign(checker: &mut Checker) {
    let colon_command_ids = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is(":"))
        .map(|fact| fact.id())
        .collect::<FxHashSet<_>>();

    let source = checker.source();
    let diagnostics = checker
        .facts()
        .words()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| colon_command_ids.contains(&fact.command_id()))
        .flat_map(|fact| fact.unquoted_assign_default_spans())
        .map(|span| {
            Diagnostic::new(DefaultValueInColonAssign, span)
                .with_fix(Fix::safe_edit(double_quote_span_edit(span, source)))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn double_quote_span_edit(span: Span, source: &str) -> Edit {
    Edit::replacement(format!("\"{}\"", span.slice(source)), span)
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_unquoted_assign_default_expansions_passed_to_colon() {
        let source = "\
#!/bin/bash
: ${HISTORY_FLAGS=''}
command : ${x=}
builtin : ${y:=fallback}
: prefix${z=word}suffix
: \"${quoted=ok}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DefaultValueInColonAssign),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "${HISTORY_FLAGS=''}",
                "${x=}",
                "${y:=fallback}",
                "${z=word}"
            ]
        );
    }

    #[test]
    fn ignores_non_colon_or_non_assignment_default_expansions() {
        let source = "\
#!/bin/bash
echo ${x=}
printf '%s\\n' ${x:=fallback}
: \"${x=}\"
: ${x:-fallback}
: ${x-fallback}
: ${x+replacement}
 env VAR=1 : ${x:=fallback}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::DefaultValueInColonAssign),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_by_quoting_default_assignment_expansion() {
        let source = "#!/bin/bash\n: ${x:=fallback}\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DefaultValueInColonAssign),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/bash\n: \"${x:=fallback}\"\n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}

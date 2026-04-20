use rustc_hash::FxHashSet;

use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct DefaultValueInColonAssign;

impl Violation for DefaultValueInColonAssign {
    fn rule() -> Rule {
        Rule::DefaultValueInColonAssign
    }

    fn message(&self) -> String {
        "quote default-assignment expansions passed to ':' to avoid splitting and globbing"
            .to_owned()
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

    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| colon_command_ids.contains(&fact.command_id()))
        .flat_map(|fact| fact.unquoted_assign_default_spans())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || DefaultValueInColonAssign);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}

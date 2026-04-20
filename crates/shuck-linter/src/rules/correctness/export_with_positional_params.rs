use rustc_hash::FxHashSet;

use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct ExportWithPositionalParams;

impl Violation for ExportWithPositionalParams {
    fn rule() -> Rule {
        Rule::ExportWithPositionalParams
    }

    fn message(&self) -> String {
        "export variable names directly instead of positional-parameter splats".to_owned()
    }
}

pub fn export_with_positional_params(checker: &mut Checker) {
    let export_ids = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("export"))
        .map(|fact| fact.id())
        .collect::<FxHashSet<_>>();

    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| export_ids.contains(&fact.command_id()))
        .filter(|fact| fact.is_pure_positional_at_splat())
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ExportWithPositionalParams);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_export_dynamic_operands_with_positional_at_splats() {
        let source = "\
#!/bin/bash
export \"$@\" ${@} \"${@:2}\"
export -- \"$@\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportWithPositionalParams),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["\"$@\"", "${@}", "\"${@:2}\"", "\"$@\""]
        );
    }

    #[test]
    fn ignores_non_export_or_non_splat_operands() {
        let source = "\
#!/bin/bash
arr=(a b)
name=HOME
export \"$name\" ${name} \"prefix$@suffix\" \"$*\" \"${arr[@]}\" \"$1\" foo
export target=\"$@\"
local \"$@\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportWithPositionalParams),
        );

        assert!(diagnostics.is_empty());
    }
}

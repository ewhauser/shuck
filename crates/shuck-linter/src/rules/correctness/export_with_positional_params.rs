use rustc_hash::FxHashSet;

use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct ExportWithPositionalParams;

impl Violation for ExportWithPositionalParams {
    fn rule() -> Rule {
        Rule::ExportWithPositionalParams
    }

    fn message(&self) -> String {
        "pass variable names directly to export instead of parameter expansions".to_owned()
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
        .filter(|fact| fact.is_plain_parameter_reference())
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ExportWithPositionalParams);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_single_direct_parameter_expansions_passed_to_export() {
        let source = "\
#!/bin/bash
export \"$@\" ${@}
export \"$name\" ${name} \"${name}\" $name
export \"$1\" ${1}
export \"$*\" ${*}
export \"$#\"
export -- \"$name\"
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
            vec![
                "\"$@\"",
                "${@}",
                "\"$name\"",
                "${name}",
                "\"${name}\"",
                "$name",
                "\"$1\"",
                "${1}",
                "\"$*\"",
                "${*}",
                "\"$#\"",
                "\"$name\"",
            ]
        );
    }

    #[test]
    fn ignores_non_export_and_non_plain_parameter_words() {
        let source = "\
#!/bin/bash
arr=(a b)
name=HOME
export \"${@:2}\" \"$@$@\" \"prefix$name\" \"${name:-fallback}\" \"${!name}\" \"${arr[@]}\" \"${arr[0]}\" foo
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

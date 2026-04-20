use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct PositionalArgsInString;

impl Violation for PositionalArgsInString {
    fn rule() -> Rule {
        Rule::PositionalArgsInString
    }

    fn message(&self) -> String {
        "positional-parameter splats inside strings collapse argument boundaries".to_owned()
    }
}

pub fn positional_args_in_string(checker: &mut Checker) {
    let spans = [
        ExpansionContext::CommandName,
        ExpansionContext::CommandArgument,
    ]
    .into_iter()
    .flat_map(|context| checker.facts().expansion_word_facts(context))
    .filter_map(|fact| fact.folded_positional_at_splat_span_in_source(checker.source()))
    .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || PositionalArgsInString);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_positional_splats_folded_into_strings() {
        let source = "\
#!/bin/bash
set -- a b
printf '%s\\n' \"$@$@\"
printf '%s\\n' \"$@\"\"$@\"
printf '%s\\n' \"items: $@\"
printf '%s\\n' x$@y
x$@y --version
if [ \"_$@\" = \"_--version\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalArgsInString),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@", "$@", "$@", "$@", "$@", "$@"]
        );
    }

    #[test]
    fn ignores_pure_and_non_command_string_contexts() {
        let source = "\
#!/bin/bash
set -- a b
printf '%s\\n' \"$@\" \"${@}\" \"${@:2}\" ${@} ${@:2}
printf '%s\\n' \"$*\" \"${@:-fallback}\" \"${array[@]}\"
foo=\"items: $@\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalArgsInString),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_general_array_mixes_that_are_out_of_scope() {
        let source = "\
#!/bin/bash
args=(--foo bar)
errors=(oops nope)
printf '%s\\n' \"D-Bus calling with: ${args[@]}\"
printf '%s\\n' \"Errors:\\n${errors[@]}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalArgsInString),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_folded_positional_splats_even_with_escaped_literals_earlier_in_word() {
        let source = "\
#!/bin/bash
set -- a b
echo \"gvm_pkgset_use: \\$@   => $@\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PositionalArgsInString),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$@"]
        );
    }
}

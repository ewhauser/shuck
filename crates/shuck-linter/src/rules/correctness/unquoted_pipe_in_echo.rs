use rustc_hash::FxHashSet;

use crate::{
    Checker, ExpansionContext, Rule, Violation, word_unquoted_escaped_pipe_or_brace_spans_in_source,
};

pub struct UnquotedPipeInEcho;

impl Violation for UnquotedPipeInEcho {
    fn rule() -> Rule {
        Rule::UnquotedPipeInEcho
    }

    fn message(&self) -> String {
        "quote echo arguments that contain escaped pipes or braces".to_owned()
    }
}

pub fn unquoted_pipe_in_echo(checker: &mut Checker) {
    let echo_command_ids = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("echo"))
        .map(|fact| fact.id())
        .collect::<FxHashSet<_>>();

    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandArgument)
        .filter(|fact| echo_command_ids.contains(&fact.command_id()))
        .filter(|fact| !fact.is_nested_word_command())
        .filter(|fact| fact.classification().is_fixed_literal())
        .filter(|fact| fact.scalar_expansion_spans().is_empty())
        .filter(|fact| fact.array_expansion_spans().is_empty())
        .filter(|fact| fact.command_substitution_spans().is_empty())
        .filter(|fact| {
            !word_unquoted_escaped_pipe_or_brace_spans_in_source(fact.word(), checker.source())
                .is_empty()
        })
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedPipeInEcho);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_echo_arguments_with_escaped_pipes_or_braces() {
        let source = "\
#!/bin/bash
echo usage: cmd [start\\|stop\\|restart]
echo token\\{on,off\\}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedPipeInEcho));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["[start\\|stop\\|restart]", "token\\{on,off\\}"]
        );
    }

    #[test]
    fn ignores_quoted_echo_arguments_and_non_echo_commands() {
        let source = "\
#!/bin/bash
echo \"usage: cmd [start\\|stop\\|restart]\"
echo 'token\\{on,off\\}'
echo \"{start,stop}\"
echo TERMUX_SUBPKG_INCLUDE=\\\"$(find ${_ADD_PREFIX}lib{,32} -name '*.a' -o -name '*.la' 2> /dev/null) $TERMUX_PKG_STATICSPLIT_EXTRA_PATTERNS\\\"
echo HEAD@{1}
printf '%s\\n' usage: cmd [start\\|stop\\|restart]
echo plain|pipeline
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedPipeInEcho));

        assert!(diagnostics.is_empty());
    }
}

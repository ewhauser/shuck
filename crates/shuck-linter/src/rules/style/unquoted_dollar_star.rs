use crate::{Checker, ExpansionContext, Rule, Violation, WordOccurrenceRef};

pub struct UnquotedDollarStar;

impl Violation for UnquotedDollarStar {
    fn rule() -> Rule {
        Rule::UnquotedDollarStar
    }

    fn message(&self) -> String {
        "quote star-splat expansions to preserve argument boundaries".to_owned()
    }
}

pub fn unquoted_dollar_star(checker: &mut Checker) {
    let spans = [
        ExpansionContext::CommandName,
        ExpansionContext::CommandArgument,
        ExpansionContext::ForList,
        ExpansionContext::SelectList,
    ]
    .into_iter()
    .flat_map(|context| checker.facts().expansion_word_facts(context))
    .filter(|fact| !fact.has_literal_affixes())
    .flat_map(unquoted_star_splat_spans)
    .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedDollarStar);
}

fn unquoted_star_splat_spans(fact: WordOccurrenceRef<'_, '_>) -> Vec<shuck_ast::Span> {
    fact.unquoted_star_splat_spans()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_star_splats_in_command_and_loop_list_contexts() {
        let source = "\
#!/bin/bash
arr=(a b)
printf '%s\\n' $* ${*} ${arr[*]} ${arr[*]:1}
for item in ${*:1}; do
  :
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedDollarStar));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$*", "${*}", "${arr[*]}", "${arr[*]:1}", "${*:1}"]
        );
    }

    #[test]
    fn reports_unquoted_star_splats_in_command_names() {
        let source = "\
#!/bin/bash
$* --version
${arr[*]} --version
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedDollarStar));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$*", "${arr[*]}"]
        );
    }

    #[test]
    fn ignores_quoted_and_non_star_expansions() {
        let source = "\
#!/bin/bash
arr=(a b)
printf '%s\\n' \"$*\" \"${arr[*]}\" ${arr[@]} ${arr[@]:1}
printf '%s\\n' prefix${arr[*]}suffix x$*y
foo=$*
printf '%s\\n' >\"$*\"
if [[ $* == foo ]]; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedDollarStar));

        assert!(diagnostics.is_empty());
    }
}

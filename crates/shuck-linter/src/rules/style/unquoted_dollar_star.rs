use shuck_ast::Span;

use crate::{
    Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation,
    WordOccurrenceRef,
};

pub struct UnquotedDollarStar;

impl Violation for UnquotedDollarStar {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::UnquotedDollarStar
    }

    fn message(&self) -> String {
        "quote star-splat expansions to preserve argument boundaries".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the star-splat expansion".to_owned())
    }
}

pub fn unquoted_dollar_star(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = [
        ExpansionContext::CommandName,
        ExpansionContext::CommandArgument,
        ExpansionContext::ForList,
        ExpansionContext::SelectList,
    ]
    .into_iter()
    .flat_map(|context| checker.facts().words().expansion_word_facts(context))
    .filter(|fact| !fact.has_literal_affixes())
    .flat_map(unquoted_star_splat_spans)
    .map(|span| {
        Diagnostic::new(UnquotedDollarStar, span)
            .with_fix(Fix::unsafe_edit(double_quote_span_edit(span, source)))
    })
    .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn unquoted_star_splat_spans(fact: WordOccurrenceRef<'_, '_>) -> Vec<shuck_ast::Span> {
    fact.unquoted_star_splat_spans()
}

fn double_quote_span_edit(span: Span, source: &str) -> Edit {
    Edit::replacement(format!("\"{}\"", span.slice(source)), span)
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

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

    #[test]
    fn applies_unsafe_fix_by_quoting_star_splats() {
        let source = "#!/bin/bash\nprintf '%s\\n' $* ${arr[*]}\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnquotedDollarStar),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\nprintf '%s\\n' \"$*\" \"${arr[*]}\"\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}

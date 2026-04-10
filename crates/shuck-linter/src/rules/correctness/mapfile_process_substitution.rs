use shuck_ast::RedirectKind;

use crate::{Checker, CommandSubstitutionKind, Rule, Violation};

pub struct MapfileProcessSubstitution;

impl Violation for MapfileProcessSubstitution {
    fn rule() -> Rule {
        Rule::MapfileProcessSubstitution
    }

    fn message(&self) -> String {
        "`mapfile` reads from a process substitution".to_owned()
    }
}

pub fn mapfile_process_substitution(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("mapfile") || fact.effective_name_is("readarray"))
        .flat_map(|fact| {
            let stdin_redirect_spans = fact
                .redirect_facts()
                .iter()
                .filter(|redirect| redirect.redirect().kind == RedirectKind::Input)
                .filter_map(|redirect| redirect.target_span())
                .collect::<Vec<_>>();
            fact.substitution_facts()
                .iter()
                .filter(|substitution| substitution.kind() == CommandSubstitutionKind::ProcessInput)
                .filter(move |substitution| {
                    stdin_redirect_spans.contains(&substitution.host_word_span())
                })
                .map(|substitution| substitution.span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || MapfileProcessSubstitution);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_mapfile_and_readarray_from_process_substitution() {
        let source = "\
mapfile -t files < <(find . -name '*.pyc')
readarray -t files < <(find . -name '*.log')
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MapfileProcessSubstitution),
        );

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["<(find . -name '*.pyc')", "<(find . -name '*.log')"]
        );
    }

    #[test]
    fn ignores_non_process_substitution_inputs() {
        let source = "\
find . -name '*.pyc' | mapfile -t files
mapfile -t files < input.txt
mapfile -t files >(wc -l)
tmp=<(find . -name '*.tmp') mapfile -t files < input.txt
tmp=<(find . -name '*.tmp') readarray -t files < input.txt
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MapfileProcessSubstitution),
        );

        assert!(diagnostics.is_empty());
    }
}

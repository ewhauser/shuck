use crate::{Checker, Rule, Violation};

pub struct FindOutputLoop;

impl Violation for FindOutputLoop {
    fn rule() -> Rule {
        Rule::FindOutputLoop
    }

    fn message(&self) -> String {
        "expanding `find` output in a `for` loop splits paths on whitespace".to_owned()
    }
}

pub fn find_output_loop(checker: &mut Checker) {
    let spans = checker
        .facts()
        .for_headers()
        .iter()
        .flat_map(|header| header.words().iter())
        .filter(|word| word.contains_find_substitution())
        .map(|word| word.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || FindOutputLoop);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_wrapped_find_substitutions_in_for_loops() {
        let source = "for item in $(command find . -type f); do :; done\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputLoop));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "$(command find . -type f)"
        );
    }

    #[test]
    fn ignores_non_find_substitutions() {
        let source = "for item in $(command printf '%s\\n' hi); do :; done\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputLoop));

        assert!(diagnostics.is_empty());
    }
}

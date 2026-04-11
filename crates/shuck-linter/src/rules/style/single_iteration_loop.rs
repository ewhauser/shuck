use crate::{Checker, ExpansionContext, Rule, Violation, WordFactContext};

pub struct SingleIterationLoop;

impl Violation for SingleIterationLoop {
    fn rule() -> Rule {
        Rule::SingleIterationLoop
    }

    fn message(&self) -> String {
        "this `for` loop iterates over a single item".to_owned()
    }
}

pub fn single_iteration_loop(checker: &mut Checker) {
    let spans = checker
        .facts()
        .for_headers()
        .iter()
        .filter(|header| !header.is_nested_word_command())
        .filter_map(|header| {
            let [word] = header.words() else {
                return None;
            };

            let fact = checker.facts().word_fact(
                word.span(),
                WordFactContext::Expansion(ExpansionContext::ForList),
            )?;
            let runtime_hazards = fact.runtime_literal().hazards;
            let analysis = fact.analysis();
            let hazards = analysis.hazards;
            if runtime_hazards.pathname_matching
                || runtime_hazards.brace_fanout
                || hazards.pathname_matching
                || hazards.brace_fanout
                || analysis.array_valued
                || analysis.can_expand_to_multiple_fields
            {
                return None;
            }

            Some(word.span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SingleIterationLoop);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_only_single_literal_for_list_items() {
        let source = "\
#!/bin/sh
for item in a; do
\tprintf '%s\\n' \"$item\"
done
for item in *.txt; do
\tprintf '%s\\n' \"$item\"
done
for item in \"$@\"; do
\tprintf '%s\\n' \"$item\"
done
for item in \"${dir}\"/x.patch; do
\tprintf '%s\\n' \"$item\"
done
for item in \"$(printf /tmp)\"/x.patch; do
\tprintf '%s\\n' \"$item\"
done
for item in foo${bar}baz; do
\tprintf '%s\\n' \"$item\"
done
for item in ~; do
\tprintf '%s\\n' \"$item\"
done
";
        let diagnostics =
            test_snippet(source, &LinterSettings::for_rule(Rule::SingleIterationLoop));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["a", "\"${dir}\"/x.patch", "\"$(printf /tmp)\"/x.patch", "~"]
        );
    }
}

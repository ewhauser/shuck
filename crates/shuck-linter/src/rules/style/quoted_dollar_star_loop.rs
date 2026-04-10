use crate::{Checker, ExpansionContext, Rule, Violation, word_quoted_star_splat_spans};

pub struct QuotedDollarStarLoop;

impl Violation for QuotedDollarStarLoop {
    fn rule() -> Rule {
        Rule::QuotedDollarStarLoop
    }

    fn message(&self) -> String {
        "quoted star-splat loop items collapse into one value".to_owned()
    }
}

pub fn quoted_dollar_star_loop(checker: &mut Checker) {
    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::ForList)
        .filter(|fact| !word_quoted_star_splat_spans(fact.word()).is_empty())
        .map(|fact| fact.span())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedDollarStarLoop);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_star_splats_in_for_lists() {
        let source = "\
#!/bin/bash
arr=(a b)
for item in \"$*\" \"${*}\" \"${*:1}\" \"${arr[*]}\" \"x$*y\"; do
  :
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedDollarStarLoop),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"$*\"",
                "\"${*}\"",
                "\"${*:1}\"",
                "\"${arr[*]}\"",
                "\"x$*y\""
            ]
        );
    }

    #[test]
    fn ignores_non_loop_and_non_star_splats() {
        let source = "\
#!/bin/bash
arr=(a b)
for item in \"$@\" \"${arr[@]}\" \"${arr[@]:1}\" ${arr[*]}; do
  :
done
select item in \"$*\"; do
  break
done
printf '%s\\n' \"$*\" \"${arr[*]}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedDollarStarLoop),
        );

        assert!(diagnostics.is_empty());
    }
}

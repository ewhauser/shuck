use crate::facts::word_spans;
use crate::{Checker, Rule, Violation, WordQuote};

pub struct QuotedDollarStarLoop;

impl Violation for QuotedDollarStarLoop {
    fn rule() -> Rule {
        Rule::QuotedDollarStarLoop
    }

    fn message(&self) -> String {
        "fully quoted loop-list expansions collapse into one value".to_owned()
    }
}

pub fn quoted_dollar_star_loop(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .for_headers()
        .iter()
        .filter_map(|header| {
            let [word] = header.words() else {
                return None;
            };

            let classification = word.classification();
            if classification.quote != WordQuote::FullyQuoted
                || classification.is_fixed_literal()
                || !word_spans::all_elements_array_expansion_part_spans(word.word(), source)
                    .is_empty()
            {
                return None;
            }

            (classification.has_command_substitution()
                || !word_spans::word_double_quoted_scalar_only_expansion_spans(word.word())
                    .is_empty()
                || !word_spans::word_quoted_star_splat_spans(word.word()).is_empty())
            .then_some(word.span())
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedDollarStarLoop);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_fully_quoted_loop_list_expansions() {
        let source = "\
#!/bin/bash
arr=(a b)
for item in \"$var\"; do :; done
for item in \"${var}\"; do :; done
for item in \"${!name}\"; do :; done
for item in \"$(printf x)\"; do :; done
for item in \"$*\"; do :; done
for item in \"${*}\"; do :; done
for item in \"${*:1}\"; do :; done
for item in \"${arr[*]}\"; do :; done
for item in \"x$*y\"; do :; done
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
                "\"$var\"",
                "\"${var}\"",
                "\"${!name}\"",
                "\"$(printf x)\"",
                "\"$*\"",
                "\"${*}\"",
                "\"${*:1}\"",
                "\"${arr[*]}\"",
                "\"x$*y\""
            ]
        );
    }

    #[test]
    fn ignores_mixed_loop_lists_with_explicit_items() {
        let source = "\
#!/bin/bash
for item in \"$var\" literal \"$@\" \"$*\"; do
  :
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedDollarStarLoop),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_nonproblematic_for_list_expansions() {
        let source = "\
#!/bin/bash
arr=(a b)
cfg_one=1
cfg_two=2
for item in \"${!name@Q}\"; do
  printf '%s\\n' \"$item\"
done
for item in \"$@\" \"${arr[@]}\" \"${arr[@]:1}\" \"${arr[@]:-fallback}\" \"${!arr[@]}\" \"${!cfg@}\" \"${!name@Q}\" \"x$@y\" \"x${arr[@]}y\" ${arr[*]}; do
  printf '%s\\n' \"$item\"
done
select item in \"$*\"; do
  printf '%s\\n' \"$item\"
  break
done
printf '%s\\n' \"$*\" \"${arr[*]}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedDollarStarLoop),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

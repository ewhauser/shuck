use crate::facts::word_spans;
use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation, WordQuote};

pub struct QuotedDollarStarLoop;

impl Violation for QuotedDollarStarLoop {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::QuotedDollarStarLoop
    }

    fn message(&self) -> String {
        "fully quoted loop-list expansions collapse into one value".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the loop-list quotes".to_owned())
    }
}

pub fn quoted_dollar_star_loop(checker: &mut Checker) {
    let diagnostics = checker
        .facts()
        .command_facts()
        .for_headers()
        .iter()
        .filter_map(|header| {
            let [word] = header.words() else {
                return None;
            };

            let classification = word.classification();
            if classification.quote != WordQuote::FullyQuoted
                || classification.is_fixed_literal()
                || word.has_all_elements_array_expansion()
            {
                return None;
            }

            (classification.has_command_substitution()
                || !word_spans::word_double_quoted_scalar_only_expansion_spans(word.word())
                    .is_empty()
                || !word_spans::word_quoted_star_splat_spans(word.word()).is_empty())
            .then_some(word.span())
        })
        .filter_map(quoted_loop_word_fix)
        .map(|(span, fix)| Diagnostic::new(QuotedDollarStarLoop, span).with_fix(fix))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn quoted_loop_word_fix(span: Span) -> Option<(Span, Fix)> {
    if span.end.offset < span.start.offset + 2 {
        return None;
    }
    Some((
        span,
        Fix::unsafe_edits([
            Edit::deletion_at(span.start.offset, span.start.offset + 1),
            Edit::deletion_at(span.end.offset - 1, span.end.offset),
        ]),
    ))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

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
    fn applies_unsafe_fix_by_unquoting_single_loop_list_word() {
        let source = "#!/bin/bash\nfor item in \"$*\"; do :; done\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::QuotedDollarStarLoop),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\nfor item in $*; do :; done\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
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

    #[test]
    fn ignores_nested_for_loops_over_quoted_at_expansions() {
        let source = "\
#!/bin/bash
COMMITS=(a b)
arns_to_block=(one two)
query=\"$(
  for commit in \"${COMMITS[@]}\"; do
    printf '%s\\n' \"$commit\"
  done
)\"
json=$(
  for arn in \"${arns_to_block[@]}\"; do
    printf '%s\\n' \"$arn\"
  done
)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedDollarStarLoop),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_corpus_wrapped_quoted_at_expansions() {
        let source = "\
#!/bin/bash
COMMITS=(abc def)
WORKFLOW_COMMITS_QUERY=\"
query {
  repository(owner: \\\"termux\\\", name: \\\"termux-packages\\\") {
  $(
    for commit in \"${COMMITS[@]}\"; do
      echo \"_${commit::7}: object(oid: \\\"${commit}\\\") { ...workflowRun }\"
    done
  )
  }
}
\"

arns_to_block=(one two)
aws s3api put-bucket-policy --bucket \"$bucket\" --policy \"$(cat <<EOF
{
  \\\"Principal\\\": {
    \\\"AWS\\\": [
$(
  for arn in \"${arns_to_block[@]}\"; do
    printf '%10s\\\"%s\\\",\\n' \"\" \"$arn\"
  done |
  sed '$ s/,$//'
)
    ]
  }
}
EOF
)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::QuotedDollarStarLoop),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

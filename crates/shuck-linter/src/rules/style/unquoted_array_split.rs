use shuck_ast::Span;

use crate::{Checker, Rule, Violation};

pub struct UnquotedArraySplit;

impl Violation for UnquotedArraySplit {
    fn rule() -> Rule {
        Rule::UnquotedArraySplit
    }

    fn message(&self) -> String {
        "quote array assignment expansions to avoid accidental splitting".to_owned()
    }
}

pub fn unquoted_array_split(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .array_assignment_split_word_facts()
        .flat_map(|fact| {
            let candidate_spans = fact
                .array_assignment_split_scalar_expansion_spans()
                .iter()
                .copied()
                .chain(fact.unquoted_array_expansion_spans().iter().copied())
                .collect::<Vec<_>>();
            let command_substitution_spans = fact.command_substitution_spans();
            fact.word()
                .parts_with_spans()
                .filter_map(|(part, part_span)| {
                    candidate_spans
                        .contains(&part_span)
                        .then_some((part, part_span))
                })
                .filter(|(_part, part_span)| {
                    !command_substitution_spans
                        .iter()
                        .any(|outer| span_contains(*outer, *part_span))
                        && !is_excluded_special_parameter_span(*part_span, source)
                })
                .map(|(_, part_span)| part_span)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedArraySplit);
}

fn span_contains(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && outer.end.offset >= inner.end.offset
}

fn is_excluded_special_parameter_span(span: Span, source: &str) -> bool {
    matches!(span.slice(source), "$!" | "$?" | "$$" | "$#" | "$-")
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_expansions_in_array_assignments() {
        let source = "\
#!/bin/bash
x='a b'
arr=($x ${x} prefix$x $@ $* ${items[@]} ${items[*]} ${x:-a b} $HOME/*.txt)
declare listed=($x)
arr+=($tail)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedArraySplit));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$x",
                "${x}",
                "$x",
                "$@",
                "$*",
                "${items[@]}",
                "${items[*]}",
                "${x:-a b}",
                "$HOME",
                "$x",
                "$tail"
            ]
        );
    }

    #[test]
    fn ignores_quoted_scalar_assignments_and_keyed_entries() {
        let source = "\
#!/bin/bash
value=$x
arr=(\"$x\" \"${items[@]}\" \"${x:-a b}\")
arr=([0]=$x [1]=\"${y}\")
declare -A map=([k]=$x)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedArraySplit));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn leaves_command_substitution_spans_for_s018() {
        let source = "\
#!/bin/bash
arr=($(cmd))
arr=(foo $(cmd)$x bar)
arr=(\"$(cmd)\" \"$x\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedArraySplit));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$x"]
        );
    }

    #[test]
    fn ignores_quoted_command_substitutions_with_quoted_inner_expansions() {
        let source = "\
#!/bin/bash
arr=(\"$(printf '%s\\n' \"$x\")\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedArraySplit));
        let slices = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(slices, Vec::<&str>::new());
    }

    #[test]
    fn ignores_expansions_inside_quoted_pipelined_heredoc_substitutions() {
        let source = r#"# shellcheck shell=bash
project=owner/repo
graphql_request=(
  -X POST
  -d "$(
    cat <<-EOF | tr '\n' ' '
      {
        "query": "query {
          repository(owner: \"${project%/*}\", name: \"${project##*/}\") {
            refs(refPrefix: \"refs/tags/\")
          }
        }"
      }
EOF
  )"
)
"#;
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedArraySplit));
        let slices = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.slice(source))
            .collect::<Vec<_>>();
        assert_eq!(slices, Vec::<&str>::new());
    }

    #[test]
    fn ignores_safe_special_parameters() {
        let source = "\
#!/bin/bash
arr=($! $? $$ $# $-)
arr=($0 $1)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedArraySplit));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$0", "$1"]
        );
    }

    #[test]
    fn ignores_use_replacement_expansions_in_array_assignments() {
        let source = "\
#!/bin/bash
arr=(${flag:+-f} ${flag:+$fallback} ${name:+\"$name\" \"$regex\"} ${items[@]+\"${items[@]}\"})
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedArraySplit));

        assert!(diagnostics.is_empty());
    }
}

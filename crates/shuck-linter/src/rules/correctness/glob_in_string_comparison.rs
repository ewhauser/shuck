use shuck_ast::{ConditionalBinaryOp, Word, WordPart};

use crate::{Checker, ConditionalNodeFact, Rule, Violation, WordQuote};

pub struct GlobInStringComparison;

impl Violation for GlobInStringComparison {
    fn rule() -> Rule {
        Rule::GlobInStringComparison
    }

    fn message(&self) -> String {
        "quote the right-hand side so string comparisons do not turn into glob matches".to_owned()
    }
}

pub fn glob_in_string_comparison(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.conditional())
        .flat_map(|conditional| conditional.nodes())
        .filter_map(|node| match node {
            ConditionalNodeFact::Binary(binary) if conditional_string_match_op(binary.op()) => {
                Some(binary)
            }
            ConditionalNodeFact::BareWord(_)
            | ConditionalNodeFact::Unary(_)
            | ConditionalNodeFact::Binary(_)
            | ConditionalNodeFact::Other(_) => None,
        })
        .filter_map(|binary| {
            let right = binary.right();
            if right.quote() != Some(WordQuote::Unquoted) {
                return None;
            }

            let word = right.word()?;
            standalone_variable_like_word(word).then_some(word.span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GlobInStringComparison);
}

fn conditional_string_match_op(op: ConditionalBinaryOp) -> bool {
    matches!(
        op,
        ConditionalBinaryOp::PatternEqShort
            | ConditionalBinaryOp::PatternEq
            | ConditionalBinaryOp::PatternNe
    )
}

fn standalone_variable_like_word(word: &Word) -> bool {
    match word.parts.as_slice() {
        [part] => matches!(
            part.kind,
            WordPart::Variable(_)
                | WordPart::Parameter(_)
                | WordPart::ParameterExpansion { .. }
                | WordPart::Length(_)
                | WordPart::ArrayAccess(_)
                | WordPart::ArrayLength(_)
                | WordPart::ArrayIndices(_)
                | WordPart::Substring { .. }
                | WordPart::ArraySlice { .. }
                | WordPart::IndirectExpansion { .. }
                | WordPart::PrefixMatch { .. }
                | WordPart::Transformation { .. }
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_standalone_variable_patterns() {
        let source = "\
#!/bin/bash
if [[ $mirror == $pkgs ]]; then echo same; fi
if [[ \"$a\" = $1 ]]; then :; fi
if [[ \"$a\" != ${b%%x} ]]; then :; fi
if [[ \"$a\" == ${arr[0]} ]]; then :; fi
if [[ \"$a\" == \"$b\" ]]; then :; fi
if [[ \"$a\" == $b* ]]; then :; fi
if [[ \"$a\" == $b$c ]]; then :; fi
if [[ \"$a\" == ${b}_x ]]; then :; fi
if [[ \"$a\" < $b ]]; then :; fi
if [ \"$a\" = $b ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$pkgs", "$1", "${b%%x}", "${arr[0]}"]
        );
    }

    #[test]
    fn reports_nested_string_comparisons_inside_command_substitutions() {
        let source = "\
#!/bin/bash
printf '%s\\n' \"$( [[ $mirror == $pkgs ]] && echo same )\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::GlobInStringComparison),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$pkgs");
    }
}

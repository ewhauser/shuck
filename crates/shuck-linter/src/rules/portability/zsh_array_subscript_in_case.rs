use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ZshArraySubscriptInCase;

impl Violation for ZshArraySubscriptInCase {
    fn rule() -> Rule {
        Rule::ZshArraySubscriptInCase
    }

    fn message(&self) -> String {
        "zsh-style array subscripts in case subjects are not portable to this shell".to_owned()
    }
}

pub fn zsh_array_subscript_in_case(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .case_subject_facts()
        .flat_map(|fact| case_subscript_spans(fact.word().span.slice(checker.source()), fact.word().span))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshArraySubscriptInCase);
}

fn case_subscript_spans(text: &str, span: Span) -> Vec<Span> {
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'$' || bytes.get(index + 1) == Some(&b'{') {
            index += 1;
            continue;
        }

        let name_start = index + 1;
        let Some(&first) = bytes.get(name_start) else {
            break;
        };
        if !(first == b'_' || first.is_ascii_alphabetic()) {
            index += 1;
            continue;
        }

        let mut cursor = name_start + 1;
        while bytes
            .get(cursor)
            .is_some_and(|byte| *byte == b'_' || byte.is_ascii_alphanumeric())
        {
            cursor += 1;
        }

        if bytes.get(cursor) != Some(&b'[') {
            index = cursor;
            continue;
        }
        let Some(relative_end) = text[cursor + 1..].find(']') else {
            index = cursor + 1;
            continue;
        };

        let bracket_offset = cursor;
        let bracket_start = span.start.advanced_by(&text[..bracket_offset]);
        spans.push(Span::from_positions(
            bracket_start,
            bracket_start.advanced_by("["),
        ));
        index = cursor + relative_end + 2;
    }

    spans
}

fn targets_non_zsh_shell(shell: ShellDialect) -> bool {
    matches!(
        shell,
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn ignores_braced_parameter_expansions() {
        let source = "#!/bin/sh\ncase \"${words[1]}\" in\n  install) : ;;\nesac\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshArraySubscriptInCase),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\ncase \"$words[1]\" in\n  install) : ;;\nesac\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshArraySubscriptInCase)
                .with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }
}

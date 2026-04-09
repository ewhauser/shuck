use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct ZshParameterFlag;

impl Violation for ZshParameterFlag {
    fn rule() -> Rule {
        Rule::ZshParameterFlag
    }

    fn message(&self) -> String {
        "zsh parameter flags are not portable to this shell".to_owned()
    }
}

pub fn zsh_parameter_flag(checker: &mut Checker) {
    if !targets_non_zsh_shell(checker.shell()) {
        return;
    }

    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .flat_map(|fact| parameter_flag_spans(fact.word().span.slice(checker.source()), fact.word().span))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ZshParameterFlag);
}

fn parameter_flag_spans(text: &str, span: Span) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut index = 0usize;

    while let Some(relative) = text[index..].find("${") {
        let start = index + relative;
        let Some(end) = parameter_expansion_end(text, start) else {
            break;
        };
        let content = &text[start + 2..end];
        if let Some(flag_offset) = nested_target_modifier_offset(content) {
            let absolute = start + 2 + flag_offset;
            let colon_start = span.start.advanced_by(&text[..absolute]);
            spans.push(Span::from_positions(
                colon_start,
                colon_start.advanced_by(":"),
            ));
        }
        index = end + 1;
    }

    spans
}

fn parameter_expansion_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut index = start;

    while index < bytes.len() {
        if bytes[index..].starts_with(b"${") {
            depth += 1;
            index += 2;
            continue;
        }
        if bytes[index] == b'}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }

    None
}

fn nested_target_modifier_offset(content: &str) -> Option<usize> {
    if !(content.starts_with("$(") || content.starts_with("${")) {
        return None;
    }

    let bytes = content.as_bytes();
    let mut index = 0usize;
    let mut parameter_depth = 0usize;
    let mut paren_depth = 0usize;

    while index < bytes.len() {
        if bytes[index..].starts_with(b"${") {
            parameter_depth += 1;
            index += 2;
            continue;
        }
        if bytes[index..].starts_with(b"$(") {
            paren_depth += 1;
            index += 2;
            continue;
        }
        if bytes[index] == b'}' && parameter_depth > 0 {
            parameter_depth -= 1;
            index += 1;
            continue;
        }
        if bytes[index] == b')' && paren_depth > 0 {
            paren_depth -= 1;
            index += 1;
            continue;
        }
        if bytes[index] != b':' || parameter_depth > 0 || paren_depth > 0 {
            index += 1;
            continue;
        }
        let Some(&next) = bytes.get(index + 1) else {
            break;
        };
        if !next.is_ascii_alphabetic() {
            index += 1;
            continue;
        }

        let mut cursor = index + 2;
        while bytes.get(cursor).is_some_and(|byte| byte.is_ascii_alphabetic()) {
            cursor += 1;
        }
        let terminator = bytes.get(cursor).copied();
        if matches!(terminator, Some(b'/') | Some(b':') | Some(b'}')) {
            return Some(index);
        }
        index = cursor;
    }

    None
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
    fn ignores_defaulting_and_numeric_slice_forms() {
        let source = "#!/bin/sh\nx=${value:-fallback}\ny=${value:0:1}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshParameterFlag));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_simple_non_nested_colon_forms() {
        let source = "#!/bin/sh\nx=${branch:gs/%/%%}\ny=${PWD:h}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ZshParameterFlag));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\nx=${$(svn info):gs/%/%%}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ZshParameterFlag).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty());
    }
}

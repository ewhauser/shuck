use crate::{Checker, Rule, Violation, static_word_text};

pub struct FunctionInAlias;

impl Violation for FunctionInAlias {
    fn rule() -> Rule {
        Rule::FunctionInAlias
    }

    fn message(&self) -> String {
        "avoid defining functions inside alias strings".to_owned()
    }
}

pub fn function_in_alias(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("alias"))
        .flat_map(|fact| {
            fact.body_args().iter().filter_map(move |word| {
                let text = static_word_text(word, source)?;
                let (_, value) = text.split_once('=')?;
                contains_function_definition(value).then_some(word.span)
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || FunctionInAlias);
}

fn contains_function_definition(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if starts_with_keyword(value, index, "function")
            && precedes_definition_start(value, index)
            && is_definition_after_function_keyword(value, index + "function".len())
        {
            return true;
        }
        if is_identifier_start(bytes[index])
            && precedes_definition_start(value, index)
            && is_definition_after_name(value, index, bytes.len())
        {
            return true;
        }
        index += 1;
    }
    false
}

fn starts_with_keyword(text: &str, index: usize, keyword: &str) -> bool {
    let tail = &text[index..];
    if !tail.starts_with(keyword) {
        return false;
    }
    let before_ok = index == 0 || !is_identifier_char(text.as_bytes()[index - 1]);
    let after_index = index + keyword.len();
    let after_ok = after_index >= text.len() || !is_identifier_char(text.as_bytes()[after_index]);
    before_ok && after_ok
}

fn precedes_definition_start(text: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }

    let bytes = text.as_bytes();
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }

    cursor == 0 || matches!(bytes[cursor - 1], b';' | b'|' | b'&' | b'(' | b'{' | b'\n')
}

fn is_definition_after_function_keyword(text: &str, mut index: usize) -> bool {
    let bytes = text.as_bytes();
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    let Some(end) = parse_identifier(text, index) else {
        return false;
    };
    is_definition_suffix(text, end)
}

fn is_definition_after_name(text: &str, index: usize, len: usize) -> bool {
    let Some(end) = parse_identifier(text, index) else {
        return false;
    };
    if end >= len {
        return false;
    }
    is_definition_suffix(text, end)
}

fn is_definition_suffix(text: &str, mut index: usize) -> bool {
    let bytes = text.as_bytes();
    while index < bytes.len() && bytes[index].is_ascii_whitespace() {
        index += 1;
    }

    if bytes
        .get(index..)
        .is_some_and(|rest| rest.starts_with(b"()"))
    {
        index += 2;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
    }

    bytes.get(index) == Some(&b'{')
}

fn parse_identifier(text: &str, index: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let first = bytes.get(index).copied()?;
    if !is_identifier_start(first) {
        return None;
    }
    let mut end = index + 1;
    while let Some(byte) = bytes.get(end) {
        if !is_identifier_char(*byte) {
            break;
        }
        end += 1;
    }
    Some(end)
}

fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_identifier_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_function_definitions_embedded_in_alias_strings() {
        let source = "\
#!/bin/sh
alias gtl='gtl(){ git tag --sort=-v:refname -n -l \"${1}*\" }; noglob gtl'
alias h='function h { echo hi; }'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "gtl='gtl(){ git tag --sort=-v:refname -n -l \"${1}*\" }; noglob gtl'",
                "h='function h { echo hi; }'",
            ]
        );
    }

    #[test]
    fn ignores_non_definition_alias_expansions() {
        let source = "\
#!/bin/sh
alias foo=$BAR
alias bar='$(printf hi)'
alias baz='noglob gtl'
alias -p
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionInAlias));

        assert!(diagnostics.is_empty());
    }
}

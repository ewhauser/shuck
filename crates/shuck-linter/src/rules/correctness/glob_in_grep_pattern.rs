use crate::{Checker, Rule, Violation, static_word_text};

pub struct GlobInGrepPattern;

impl Violation for GlobInGrepPattern {
    fn rule() -> Rule {
        Rule::GlobInGrepPattern
    }

    fn message(&self) -> String {
        "use regex-style wildcards in grep patterns, not glob-style `*`".to_owned()
    }
}

pub fn glob_in_grep_pattern(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("grep"))
        .filter_map(|fact| fact.options().grep())
        .filter(|grep| !grep.uses_fixed_strings)
        .flat_map(|grep| grep.pattern_words().iter().copied())
        .filter(|word| {
            static_word_text(word, checker.source())
                .is_some_and(|pattern| has_glob_style_star_confusion(&pattern))
        })
        .map(|word| word.span)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || GlobInGrepPattern);
}

fn has_glob_style_star_confusion(pattern: &str) -> bool {
    let bytes = pattern.as_bytes();

    if first_unescaped_star_index(bytes).is_some_and(|index| index == 0) {
        return false;
    }

    let mut index = 0usize;
    while let Some(star_index) = next_unescaped_star_index(bytes, index) {
        let Some(previous) = previous_unescaped_byte(bytes, star_index) else {
            index = star_index + 1;
            continue;
        };

        if matches!(
            previous,
            b'.' | b']' | b')' | b'*' | b'+' | b'?' | b'|' | b'^' | b'$' | b'{' | b'(' | b'\\'
        ) || previous.is_ascii_whitespace()
        {
            index = star_index + 1;
            continue;
        }

        return true;
    }

    false
}

fn first_unescaped_star_index(bytes: &[u8]) -> Option<usize> {
    next_unescaped_star_index(bytes, 0)
}

fn next_unescaped_star_index(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == b'*' {
            return Some(index);
        }
        index += 1;
    }
    None
}

fn previous_unescaped_byte(bytes: &[u8], index: usize) -> Option<u8> {
    let mut candidate = index;
    while candidate > 0 {
        candidate -= 1;
        if !is_escaped(bytes, candidate) {
            return Some(bytes[candidate]);
        }
    }
    None
}

fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let mut backslashes = 0usize;
    let mut cursor = index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        backslashes += 1;
        cursor -= 1;
    }
    backslashes % 2 == 1
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_glob_style_stars_in_grep_patterns() {
        let source = "\
#!/bin/sh
grep start* out.txt
grep \"start*\" out.txt
grep 'foo*bar' out.txt
grep foo*bar out.txt
grep -efoo* out.txt
grep --regexp start* out.txt
grep \"foo*bar\" out.txt
grep item\\* out.txt
grep -E \"foo*bar\" out.txt
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GlobInGrepPattern));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "start*",
                "\"start*\"",
                "'foo*bar'",
                "foo*bar",
                "-efoo*",
                "start*",
                "\"foo*bar\"",
                "item\\*",
                "\"foo*bar\"",
            ]
        );
    }

    #[test]
    fn ignores_regex_operators_and_non_pattern_operands() {
        let source = "\
#!/bin/sh
grep \"a.*\" out.txt
grep a.* out.txt
grep \"[ab]*\" out.txt
grep [ab]* out.txt
grep '*start' out.txt
grep '*start*' out.txt
grep item\\\\* out.txt
grep '^ *#' out.txt
grep '\"name\": *\"$x\"' out.txt
grep -F foo*bar out.txt
grep -F \"foo*bar\" out.txt
grep --fixed-strings foo*bar out.txt
grep --fixed-strings \"foo*bar\" out.txt
grep -eo foo* out.txt
grep --regexp='start*' out.txt
grep --regexp=start* out.txt
grep -efoo out.txt
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::GlobInGrepPattern));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}

use crate::{Checker, Rule, Violation, static_word_text};

pub struct UnquotedTrRange;

impl Violation for UnquotedTrRange {
    fn rule() -> Rule {
        Rule::UnquotedTrRange
    }

    fn message(&self) -> String {
        "quote `tr` character class and range operands".to_owned()
    }
}

pub fn unquoted_tr_range(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("tr") && fact.wrappers().is_empty())
        .flat_map(|fact| {
            fact.body_args().iter().filter_map(|word| {
                let text = static_word_text(word, checker.source())?;
                is_bracketed_tr_set(text.as_str()).then_some(word.span)
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedTrRange);
}

fn is_bracketed_tr_set(text: &str) -> bool {
    if text.len() < 2 || !text.starts_with('[') || !text.ends_with(']') {
        return false;
    }

    let inner = &text[1..text.len() - 1];
    if inner.starts_with('[') && inner.ends_with(']') {
        return inner.contains(':');
    }

    if inner.starts_with(':') && inner.ends_with(':') {
        return false;
    }

    inner
        .bytes()
        .any(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_bracketed_tr_operands() {
        let source = "\
#!/bin/sh
tr '[abc]' '[xyz]'
tr [a-z] [A-Z]
tr '[0-9a-f]' '0'
tr '[[:upper:]]' 'x'
value=$(printf '%s' \"$value\" | tr '[A-Z]' '[a-z]')
digits=$(printf '%s' \"$value\" | tr -d '[0-9]')
command tr '[A-Z]' '[a-z]'
tr '[#/.=()]' '_'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedTrRange));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "'[abc]'",
                "'[xyz]'",
                "[a-z]",
                "[A-Z]",
                "'[0-9a-f]'",
                "'[[:upper:]]'",
                "'[A-Z]'",
                "'[a-z]'",
                "'[0-9]'",
            ]
        );
    }

    #[test]
    fn ignores_non_bracketed_tr_operands_and_other_commands() {
        let source = "\
#!/bin/sh
tr '[:upper:]' '[:lower:]'
tr '[#/.=()]' _
command tr '[A-Z]' '[a-z]'
printf '%s\\n' '[abc]'
command tr x y
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedTrRange));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_repeated_tr_deletes_inside_command_substitutions() {
        let source = "\
#!/bin/sh
_idn_temp=$(printf \"%s\" \"$value\" | tr -d '[0-9]' | tr -d '[a-z]' | tr -d '[A-Z]' | tr -d '*.,-_')
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedTrRange));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["'[0-9]'", "'[a-z]'", "'[A-Z]'",]
        );
    }
}

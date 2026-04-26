use shuck_ast::Span;

use crate::{Checker, Rule, Violation};

pub struct BadVarName;

impl Violation for BadVarName {
    fn rule() -> Rule {
        Rule::BadVarName
    }

    fn message(&self) -> String {
        "assignment target starts with an invalid character".to_owned()
    }
}

pub fn bad_var_name(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| bad_var_name_span(fact.arena_body_name_word(source)?.span(), source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BadVarName);
}

fn bad_var_name_span(word_span: Span, source: &str) -> Option<Span> {
    let text = word_span.slice(source);
    let target_end = text.find('=')?;
    if target_end > 0 && text.as_bytes()[target_end - 1] == b'+' {
        return None;
    }

    let target = &text[..target_end];
    let first = target.chars().next()?;
    if !first.is_ascii_digit() || target.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }

    Some(word_span)
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_assignment_like_words_with_digit_prefixed_names() {
        let source = "\
#!/bin/sh
9var=ok
1_name=value
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BadVarName));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["9var=ok", "1_name=value"]
        );
    }

    #[test]
    fn ignores_numeric_targets_append_forms_and_declaration_builtins() {
        let source = "\
#!/bin/sh
9=ok
10=ok
9var+=ok
export 9var=ok
readonly 9var=ok
command 9var=ok
foo=ok
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BadVarName));

        assert!(diagnostics.is_empty());
    }
}

use shuck_ast::Name;
use shuck_semantic::BindingAttributes;

use crate::{Checker, Rule, Violation};

pub struct UnsetAssociativeArrayElement;

impl Violation for UnsetAssociativeArrayElement {
    fn rule() -> Rule {
        Rule::UnsetAssociativeArrayElement
    }

    fn message(&self) -> String {
        "quote associative-array unset targets as `'name[key]'` to keep keys literal".to_owned()
    }
}

pub fn unset_associative_array_element(checker: &mut Checker) {
    let source = checker.source();
    let semantic = checker.semantic();
    let mut spans = Vec::new();

    for fact in checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.effective_name_is("unset"))
    {
        let Some(unset) = fact.options().unset() else {
            continue;
        };

        for operand in unset.operand_words() {
            let Some((name, key_text)) = parse_array_operand(operand.span.slice(source)) else {
                continue;
            };
            if !key_has_unescaped_quote(key_text) {
                continue;
            }

            let Some(visible) = semantic.visible_binding(&Name::from(name), operand.span) else {
                continue;
            };
            if visible.attributes.contains(BindingAttributes::ASSOC) {
                spans.push(operand.span);
            }
        }
    }

    checker.report_all_dedup(spans, || UnsetAssociativeArrayElement);
}

fn parse_array_operand(text: &str) -> Option<(&str, &str)> {
    let (name, key_with_bracket) = text.split_once('[')?;
    let key = key_with_bracket.strip_suffix(']')?;
    is_shell_name(name).then_some((name, key))
}

fn is_shell_name(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn key_has_unescaped_quote(text: &str) -> bool {
    let mut backslashes = 0usize;
    for ch in text.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }

        let escaped = backslashes % 2 == 1;
        backslashes = 0;

        if !escaped && (ch == '\'' || ch == '"') {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_associative_unset_keys() {
        let source = "\
#!/bin/bash
declare -A parts
parts[one]=1
unset parts[\"one\"]
unset parts['two']
key=three
unset parts[\"$key\"]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnsetAssociativeArrayElement),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["parts[\"one\"]", "parts['two']", "parts[\"$key\"]"]
        );
    }

    #[test]
    fn ignores_indexed_or_safely_quoted_unset_operands() {
        let source = "\
#!/bin/bash
declare -a nums
declare -A parts
key=one
unset nums[\"1\"]
unset parts[$key]
unset 'parts[key]'
unset \"parts[key]\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnsetAssociativeArrayElement),
        );

        assert!(diagnostics.is_empty());
    }
}

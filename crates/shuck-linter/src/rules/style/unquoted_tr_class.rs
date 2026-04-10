use crate::{
    Checker, ExpansionContext, Rule, Violation, WordFactContext, WordQuote, static_word_text,
};

pub struct UnquotedTrClass;

impl Violation for UnquotedTrClass {
    fn rule() -> Rule {
        Rule::UnquotedTrClass
    }

    fn message(&self) -> String {
        "quote `tr` character class operands".to_owned()
    }
}

pub fn unquoted_tr_class(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("tr") && fact.wrappers().is_empty())
        .flat_map(|fact| {
            fact.body_args().iter().filter_map(|word| {
                let word_fact = checker.facts().word_fact(
                    word.span,
                    WordFactContext::Expansion(ExpansionContext::CommandArgument),
                )?;
                if word_fact.classification().quote != WordQuote::Unquoted {
                    return None;
                }
                let text = static_word_text(word, checker.source())?;
                is_unquoted_tr_class(text.as_str()).then_some(word.span)
            })
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedTrClass);
}

fn is_unquoted_tr_class(text: &str) -> bool {
    if text.len() < 4 || !text.starts_with("[:") || !text.ends_with(":]") {
        return false;
    }

    let inner = &text[2..text.len() - 2];
    !inner.is_empty() && inner.bytes().all(|byte| byte.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_tr_class_operands() {
        let source = "\
#!/bin/sh
tr '[:upper:]' [:lower:]
tr [:upper:] [:lower:]
tr [:alpha:] x
tr x [:alpha:]
command tr [:upper:] [:lower:]
tr '[[:upper:]]' [:lower:]
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedTrClass));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "[:lower:]",
                "[:upper:]",
                "[:lower:]",
                "[:alpha:]",
                "[:alpha:]",
                "[:lower:]",
            ]
        );
    }

    #[test]
    fn ignores_quoted_classes_and_other_commands() {
        let source = "\
#!/bin/sh
tr '[:upper:]' '[:lower:]'
tr '[[:upper:]]' '[:lower:]'
command tr '[:upper:]' [:lower:]
printf '%s\\n' '[:lower:]'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnquotedTrClass));

        assert!(diagnostics.is_empty());
    }
}

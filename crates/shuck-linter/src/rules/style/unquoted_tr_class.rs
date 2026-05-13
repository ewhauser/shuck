use shuck_ast::Span;
use shuck_ast::static_word_text;

use crate::{
    Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation,
    WordFactContext, WordQuote,
};

pub struct UnquotedTrClass;

impl Violation for UnquotedTrClass {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::UnquotedTrClass
    }

    fn message(&self) -> String {
        "quote `tr` character class operands".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the `tr` character class".to_owned())
    }
}

pub fn unquoted_tr_class(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
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
                is_unquoted_tr_class(text.as_ref()).then_some(word.span)
            })
        })
        .map(|span| {
            Diagnostic::new(UnquotedTrClass, span)
                .with_fix(Fix::unsafe_edit(single_quote_span_edit(span, source)))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn is_unquoted_tr_class(text: &str) -> bool {
    if text.len() < 4 || !text.starts_with("[:") || !text.ends_with(":]") {
        return false;
    }

    let inner = &text[2..text.len() - 2];
    !inner.is_empty() && inner.bytes().all(|byte| byte.is_ascii_lowercase())
}

fn single_quote_span_edit(span: Span, source: &str) -> Edit {
    Edit::replacement(format!("'{}'", span.slice(source)), span)
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

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

    #[test]
    fn applies_unsafe_fix_by_quoting_tr_character_classes() {
        let source = "#!/bin/sh\ntr [:upper:] [:lower:]\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnquotedTrClass),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\ntr '[:upper:]' '[:lower:]'\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}

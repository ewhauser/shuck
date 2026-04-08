use crate::rules::common::expansion::ExpansionContext;
use crate::{Checker, Rule, Violation, WordFactContext};

pub struct PrintfFormatVariable;

impl Violation for PrintfFormatVariable {
    fn rule() -> Rule {
        Rule::PrintfFormatVariable
    }

    fn message(&self) -> String {
        "keep `printf` format strings literal instead of expanding them from variables".to_owned()
    }
}

pub fn printf_format_variable(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            fact.options()
                .printf()
                .and_then(|printf| printf.format_word)
        })
        .filter_map(|word| {
            checker
                .facts()
                .word_fact(
                    word.span,
                    WordFactContext::Expansion(ExpansionContext::CommandArgument),
                )
                .and_then(|fact| (!fact.classification().is_fixed_literal()).then_some(word.span))
        })
        .collect::<Vec<_>>();

    for span in spans {
        checker.report(PrintfFormatVariable, span);
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_runtime_supplied_formats_and_skips_fixed_literals() {
        let source = "printf '%s\\n' value\nprintf \"$fmt\" value\nprintf \"$(echo %s)\" value\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PrintfFormatVariable),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn skips_v_assignment_target_and_anchors_on_the_real_format_word() {
        let source = "printf -v out \"$fmt\" value\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PrintfFormatVariable),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\"$fmt\"");
    }
}

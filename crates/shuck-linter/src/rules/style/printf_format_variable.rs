use crate::{Checker, ExpansionContext, Rule, Violation, WordFactContext, WrapperKind};

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
        .filter(|fact| matches!(fact.wrappers(), [] | [WrapperKind::Builtin]))
        .filter_map(|fact| {
            let printf = fact.options().printf()?;
            (!printf.format_word_has_literal_percent)
                .then_some(printf.format_word_span)
                .flatten()
        })
        .filter_map(|span| {
            checker
                .facts()
                .word_fact(
                    span,
                    WordFactContext::Expansion(ExpansionContext::CommandArgument),
                )
                .and_then(|fact| (!fact.classification().is_fixed_literal()).then_some(span))
        })
        .collect::<Vec<_>>();

    checker.report_all(spans, || PrintfFormatVariable);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_dynamic_formats_without_literal_percents_and_skips_percent_templates() {
        let source = "printf '%s\\n' value\nprintf \"$fmt\" value\nprintf \"$(echo %s)\" value\nprintf \"${left}${right}\" value\nprintf \"pre$foo\" value\nprintf \"%${span}s\\n\" value\nprintf \"${color}%s${reset}\" value\nprintf \"$fmt%s\" value\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PrintfFormatVariable),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3, 4, 5]
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

    #[test]
    fn skips_v_dash_dash_prefix_before_the_format_word() {
        let source = "printf -v out -- \"$fmt\" value\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PrintfFormatVariable),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\"$fmt\"");
    }

    #[test]
    fn reports_command_substitution_formats_even_with_literal_backslash_prefixes() {
        let source = "i=65\nkeyassoc=\"$( printf \"\\\\$(printf '%03o' \"$i\")\" )\"\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PrintfFormatVariable),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "\"\\\\$(printf '%03o' \"$i\")\""
        );
    }

    #[test]
    fn skips_command_like_wrappers_but_keeps_builtin_and_backslash_forms() {
        let source = "printf \"$fmt\" value\nbuiltin printf \"$fmt\" value\n\\printf \"$fmt\" value\ncommand printf \"$fmt\" value\ncommand -- printf \"$fmt\" value\nexec printf \"$fmt\" value\nnoglob printf \"$fmt\" value\nsudo printf \"$fmt\" value\nbusybox printf \"$fmt\" value\nfind . -exec printf \"$fmt\" value \\;\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PrintfFormatVariable),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }
}

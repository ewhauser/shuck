use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct UnquotedWordBetweenQuotes;

impl Violation for UnquotedWordBetweenQuotes {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::UnquotedWordBetweenQuotes
    }

    fn message(&self) -> String {
        "an unquoted word follows single-quoted text".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the literal word segment".to_owned())
    }
}

pub fn unquoted_word_between_quotes(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .word_facts()
        .iter()
        .flat_map(|fact| fact.unquoted_word_after_single_quoted_segment_spans(source))
        .map(|span| {
            Diagnostic::new(UnquotedWordBetweenQuotes, span)
                .with_fix(Fix::safe_edit(quote_literal_segment_edit(span, source)))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn quote_literal_segment_edit(span: Span, source: &str) -> Edit {
    Edit::replacement(format!("'{}'", span.slice(source)), span)
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_unquoted_words_after_single_quoted_segments() {
        let source = "\
#!/bin/bash
printf '%s\\n' 'foo'Default'baz'
sed -i 's/${title}/'Default'/g' \"$file\"
x='a'b'c'
arr=('a'123'c')
sed -i '/.*certs\\.h/'d dependencies/file.cpp
ip route | grep ^default'\\s'via | head -1
sed -i '/install(/s,\\<lib\\>,'lib$LIBDIRSUFFIX',' CMakeLists.txt
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedWordBetweenQuotes),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["Default", "Default", "b", "123", "d", "via", "lib"]
        );
    }

    #[test]
    fn ignores_punctuation_only_and_non_literal_middle_segments() {
        let source = "\
#!/bin/bash
printf '%s\\n' 'foo'-'baz'
printf '%s\\n' 'foo''baz'
printf '%s\\n' 'foo'$bar'baz'
printf '%s\\n' $'foo'Default'baz'
sed -i -e 's/^package .*/package 'fuzz_ng_$pkg_flat'/' \"$file\"
sed -i -e 's/^package .*/package 'foo_bar'/' \"$file\"
printf '%s\\n' 's/foo/'\\''bar'\\''/g'
sed -i 's/^\\(\\[binaries\\]\\)$/\\1\\nexe_wrapper = '\\''exe_wrapper'\\''/g' \\
  \"$TERMUX_MESON_CROSSFILE\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::UnquotedWordBetweenQuotes),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_safe_fix_by_quoting_middle_literal_segment() {
        let source = "#!/bin/bash\nprintf '%s\\n' 'foo'Default'baz'\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::UnquotedWordBetweenQuotes),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\nprintf '%s\\n' 'foo''Default''baz'\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}

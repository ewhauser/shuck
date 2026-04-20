use crate::{Checker, Rule, Violation};

pub struct UnquotedWordBetweenQuotes;

impl Violation for UnquotedWordBetweenQuotes {
    fn rule() -> Rule {
        Rule::UnquotedWordBetweenQuotes
    }

    fn message(&self) -> String {
        "an unquoted word follows single-quoted text".to_owned()
    }
}

pub fn unquoted_word_between_quotes(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .flat_map(|fact| fact.unquoted_word_after_single_quoted_segment_spans(source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || UnquotedWordBetweenQuotes);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}

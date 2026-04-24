use shuck_ast::{Position, Span};

use crate::{Checker, Rule, Violation, WordFactHostKind, WordQuote};

pub struct TildeInStringComparison;

impl Violation for TildeInStringComparison {
    fn rule() -> Rule {
        Rule::TildeInStringComparison
    }

    fn message(&self) -> String {
        "quoted `~/...` stays literal; use `$HOME` or an unquoted tilde".to_owned()
    }
}

pub fn tilde_in_string_comparison(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .word_facts()
        .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
        .filter_map(|fact| word_fact_tilde_span(fact, source))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TildeInStringComparison);
}

fn word_fact_tilde_span(fact: crate::WordOccurrenceRef<'_, '_>, source: &str) -> Option<Span> {
    let classification = fact.classification();
    (classification.quote != WordQuote::Unquoted).then(|| quoted_tilde_span(fact.span(), source))?
}

fn quoted_tilde_span(span: Span, source: &str) -> Option<Span> {
    let raw = span.slice(source);
    let quote = raw.chars().next()?;
    if !matches!(quote, '"' | '\'') || !raw.get(1..)?.starts_with("~/") {
        return None;
    }

    quoted_tilde_span_from_raw(span, raw)
}

fn quoted_tilde_span_from_raw(span: Span, raw: &str) -> Option<Span> {
    let quote = raw.chars().next()?;
    let quote_len = quote.len_utf8();
    let close_index = raw[quote_len..]
        .find(quote)
        .map(|index| quote_len + index)
        .unwrap_or(raw.len());
    let start_index = if quote == '\'' { 0 } else { quote_len };
    let end_index = if quote == '\'' {
        (close_index + quote_len).min(raw.len())
    } else {
        raw[quote_len..close_index]
            .find(['$', '`'])
            .map(|index| quote_len + index)
            .unwrap_or(close_index)
    };
    let start = advance_position(span.start, &raw[..start_index]);
    let end = advance_position(span.start, &raw[..end_index]);
    Some(Span::from_positions(start, end))
}

fn advance_position(mut position: Position, text: &str) -> Position {
    position = position.advanced_by(text);
    position
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_quoted_home_relative_paths_in_string_comparisons() {
        let source = "\
#!/bin/bash
[ \"$profile\" = \"~/.bashrc\" ]
[ \"~/.bashrc\" = \"$profile\" ]
[[ \"$profile\" == \"~/.profile\" ]]
[ \"$profile\" != '~/.zshrc' ]
[ ! = \"~/.bashrc\" ]
[ ! \"$profile\" = \"~/.bashrc\" ]
[ ! \"~/.bashrc\" = \"$profile\" ]
[ ! \"$profile\" != '~/.zshrc' ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TildeInStringComparison),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "~/.bashrc",
                "~/.bashrc",
                "~/.profile",
                "'~/.zshrc'",
                "~/.bashrc",
                "~/.bashrc",
                "~/.bashrc",
                "'~/.zshrc'",
            ]
        );
    }

    #[test]
    fn ignores_unquoted_tilde_and_non_home_tilde_literals() {
        let source = "\
#!/bin/bash
[ \"$profile\" = ~/.bashrc ]
[ \"$profile\" = \"~user/.bashrc\" ]
[ \"$profile\" = \"~\" ]
[ \"$profile\" = \"foo~/.bashrc\" ]
[[ \"$profile\" == a~/.bashrc ]]
[ ! \"$profile\" = ~/.bashrc ]
[ ! \"$profile\" = \"~user/.bashrc\" ]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TildeInStringComparison),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_dollar_quoted_home_literals_to_match_oracle() {
        let source = "\
#!/bin/bash
profile=$'~/.bashrc'
fallback=$\"~/.profile\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TildeInStringComparison),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_quoted_tilde_literals_in_expanded_words() {
        let source = "\
#!/bin/bash
profile='~/.bash_profile'
VAGRANT_HOME=\"~/.vagrant.d\"
[ -e '~/.bash_profile' ]
printf '%s\n' \"~/.config/powershell/profile.ps1\"
case \"$path\" in \"~/.cache\") : ;; esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TildeInStringComparison),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "'~/.bash_profile'",
                "~/.vagrant.d",
                "'~/.bash_profile'",
                "~/.config/powershell/profile.ps1",
                "~/.cache",
            ]
        );
    }
}

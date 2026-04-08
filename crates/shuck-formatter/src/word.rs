use shuck_ast::{Pattern, Word};
use shuck_format::{FormatResult, text, write};

use crate::FormatNodeRule;
use crate::options::ResolvedShellFormatOptions;
use crate::prelude::ShellFormatter;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatWord;

impl FormatNodeRule<Word> for FormatWord {
    fn fmt(&self, word: &Word, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let rendered = render_word_syntax(
            word,
            formatter.context().source(),
            formatter.context().options(),
        );
        write!(formatter, [text(rendered)])
    }
}

pub(crate) fn render_word_syntax(
    word: &Word,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let rendered = word.render_syntax(source);

    if !options.simplify()
        && !options.minify()
        && let Some(slice) = raw_word_source_slice(word, source)
        && should_preserve_raw_syntax(slice, &rendered)
    {
        return slice.to_string();
    }

    rendered
}

pub(crate) fn render_pattern_syntax(
    pattern: &Pattern,
    source: &str,
    options: &ResolvedShellFormatOptions,
) -> String {
    let rendered = pattern.render_syntax(source);

    if !options.simplify()
        && !options.minify()
        && let Some(slice) = raw_pattern_source_slice(pattern, source)
        && should_preserve_raw_syntax(slice, &rendered)
    {
        return slice.to_string();
    }

    rendered
}

fn raw_word_source_slice<'a>(word: &Word, source: &'a str) -> Option<&'a str> {
    raw_source_slice(word.span, source)
}

fn raw_pattern_source_slice<'a>(pattern: &Pattern, source: &'a str) -> Option<&'a str> {
    raw_source_slice(pattern.span, source)
}

fn raw_source_slice(span: shuck_ast::Span, source: &str) -> Option<&str> {
    if span.start.offset >= span.end.offset || span.end.offset > source.len() {
        return None;
    }

    let slice = span.slice(source);
    if slice.contains('\n') {
        Some(slice)
    } else {
        Some(trim_unescaped_trailing_whitespace(slice))
    }
}

fn should_preserve_raw_syntax(raw: &str, rendered: &str) -> bool {
    raw != rendered
        && (raw.starts_with('\\')
            || raw.starts_with('&')
            || raw.starts_with("$'")
            || raw.contains("\\\\")
            || raw.contains("[^ ]"))
}

fn trim_unescaped_trailing_whitespace(text: &str) -> &str {
    let mut end = text.len();
    while end > 0 {
        let Some((whitespace_start, ch)) = text[..end].char_indices().next_back() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }

        let backslash_count = text.as_bytes()[..whitespace_start]
            .iter()
            .rev()
            .take_while(|byte| **byte == b'\\')
            .count();
        if backslash_count % 2 == 1 {
            break;
        }

        end = whitespace_start;
    }

    &text[..end]
}

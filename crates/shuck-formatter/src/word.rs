use shuck_ast::Word;
use shuck_format::{FormatResult, text, write};

use crate::FormatNodeRule;
use crate::prelude::ShellFormatter;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatWord;

impl FormatNodeRule<Word> for FormatWord {
    fn fmt(&self, word: &Word, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let rendered = word.render(formatter.context().source());
        write!(formatter, [text(rendered)])
    }
}

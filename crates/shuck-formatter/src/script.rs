use shuck_ast::File;
use shuck_format::{FormatResult, hard_line_break, text, write};

use crate::FormatNodeRule;
use crate::command::format_stmt_sequence;
use crate::prelude::ShellFormatter;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatFile;

impl FormatNodeRule<File> for FormatFile {
    fn fmt(&self, file: &File, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        format_stmt_sequence(&file.body, formatter)?;

        if !formatter.context().options().minify() {
            let remaining = formatter.context_mut().comments_mut().take_remaining();
            if !file.body.is_empty() && !remaining.is_empty() {
                write!(formatter, [hard_line_break()])?;
            }
            for (index, comment) in remaining.iter().enumerate() {
                if index > 0 {
                    write!(formatter, [hard_line_break()])?;
                }
                write!(formatter, [text(comment.text().to_string())])?;
            }
        }

        Ok(())
    }
}

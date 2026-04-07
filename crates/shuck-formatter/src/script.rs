use shuck_ast::Script;
use shuck_format::{FormatResult, hard_line_break, text, write};

use crate::FormatNodeRule;
use crate::command::format_command_sequence;
use crate::prelude::ShellFormatter;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatScript;

impl FormatNodeRule<Script> for FormatScript {
    fn fmt(&self, script: &Script, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        format_command_sequence(&script.commands, formatter)?;

        if !formatter.context().options().minify() {
            let remaining = formatter.context_mut().comments_mut().take_remaining();
            if !script.commands.is_empty() && !remaining.is_empty() {
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

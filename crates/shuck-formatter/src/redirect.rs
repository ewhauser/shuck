use shuck_ast::{Redirect, RedirectKind};
use shuck_format::{FormatResult, text, write};

use crate::FormatNodeRule;
use crate::prelude::ShellFormatter;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatRedirect;

impl FormatNodeRule<Redirect> for FormatRedirect {
    fn fmt(&self, redirect: &Redirect, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let source = formatter.context().source();
        let options = formatter.context().options();
        let mut rendered = String::new();

        if let Some(name) = &redirect.fd_var {
            rendered.push('{');
            rendered.push_str(name.as_str());
            rendered.push('}');
        } else if let Some(fd) = redirect.fd {
            rendered.push_str(&fd.to_string());
        }

        rendered.push_str(match redirect.kind {
            RedirectKind::Output => ">",
            RedirectKind::Clobber => ">|",
            RedirectKind::Append => ">>",
            RedirectKind::Input => "<",
            RedirectKind::ReadWrite => "<>",
            RedirectKind::HereDoc => "<<",
            RedirectKind::HereDocStrip => "<<-",
            RedirectKind::HereString => "<<<",
            RedirectKind::DupOutput => ">&",
            RedirectKind::DupInput => "<&",
            RedirectKind::OutputBoth => "&>",
        });

        let target = match (redirect.word_target(), redirect.heredoc()) {
            (Some(word), None) => word.render_syntax(source),
            (None, Some(heredoc)) => heredoc.delimiter.raw.render_syntax(source),
            (None, None) => String::new(),
            (Some(_), Some(_)) => unreachable!("redirect target cannot be both word and heredoc"),
        };
        if options.space_redirects()
            && !matches!(
                redirect.kind,
                RedirectKind::DupOutput | RedirectKind::DupInput
            )
        {
            rendered.push(' ');
        }
        rendered.push_str(&target);

        write!(formatter, [text(rendered)])
    }
}

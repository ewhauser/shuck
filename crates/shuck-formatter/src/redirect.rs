use shuck_ast::{Redirect, RedirectKind};
use shuck_format::{FormatResult, text, write};

use crate::FormatNodeRule;
use crate::prelude::ShellFormatter;
use crate::word::render_word_syntax;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatRedirect;

impl FormatNodeRule<Redirect> for FormatRedirect {
    fn fmt(&self, redirect: &Redirect, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let source = formatter.context().source();
        let options = formatter.context().options();
        if !options.simplify()
            && !options.minify()
            && let Some(raw) = raw_redirect_source_slice(redirect, source)
            && should_preserve_raw_redirect(raw)
        {
            return write!(formatter, [text(raw.to_string())]);
        }

        let mut rendered = String::new();

        if let Some(name) = &redirect.fd_var {
            rendered.push('{');
            rendered.push_str(name.as_str());
            rendered.push('}');
        } else if let Some(fd) = redirect
            .fd
            .filter(|fd| should_render_explicit_fd(*fd, redirect.kind))
        {
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
            (Some(word), None) => render_word_syntax(word, source, options),
            (None, Some(heredoc)) => render_word_syntax(&heredoc.delimiter.raw, source, options),
            (None, None) => String::new(),
            (Some(_), Some(_)) => unreachable!("redirect target cannot be both word and heredoc"),
        };
        if needs_space_before_target(redirect.kind, &target, options.space_redirects()) {
            rendered.push(' ');
        }
        rendered.push_str(&target);

        write!(formatter, [text(rendered)])
    }
}

fn should_render_explicit_fd(fd: i32, kind: RedirectKind) -> bool {
    match kind {
        RedirectKind::Output
        | RedirectKind::Clobber
        | RedirectKind::Append
        | RedirectKind::DupOutput
        | RedirectKind::OutputBoth => fd != 1,
        RedirectKind::Input
        | RedirectKind::ReadWrite
        | RedirectKind::HereDoc
        | RedirectKind::HereDocStrip
        | RedirectKind::HereString
        | RedirectKind::DupInput => fd != 0,
    }
}

fn needs_space_before_target(kind: RedirectKind, target: &str, space_redirects: bool) -> bool {
    if target.is_empty() {
        return false;
    }

    if space_redirects && !matches!(kind, RedirectKind::DupOutput | RedirectKind::DupInput) {
        return true;
    }

    !matches!(kind, RedirectKind::DupOutput | RedirectKind::DupInput)
        && target
            .as_bytes()
            .first()
            .is_some_and(|byte| matches!(byte, b'<' | b'>' | b'&'))
}

fn raw_redirect_source_slice<'a>(redirect: &Redirect, source: &'a str) -> Option<&'a str> {
    let span = redirect.span;
    (span.start.offset < span.end.offset && span.end.offset <= source.len())
        .then(|| span.slice(source))
}

fn should_preserve_raw_redirect(raw: &str) -> bool {
    raw.contains(">&$") || raw.contains("<&$")
}

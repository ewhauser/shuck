use shuck_ast::{Redirect, RedirectKind};
use shuck_format::{FormatResult, space, text, token, write};

use crate::FormatNodeRule;
use crate::prelude::ShellFormatter;
use crate::word::render_word_syntax;

#[derive(Debug, Default, Clone, Copy)]
pub struct FormatRedirect;

impl FormatNodeRule<Redirect> for FormatRedirect {
    fn fmt(&self, redirect: &Redirect, formatter: &mut ShellFormatter<'_, '_>) -> FormatResult<()> {
        let source = formatter.context().source();
        let options = formatter.context().options().clone();
        if !options.simplify()
            && !options.minify()
            && let Some(raw) = raw_redirect_source_slice(redirect, source)
            && should_preserve_raw_redirect(raw)
        {
            return write!(formatter, [text(raw.to_string())]);
        }

        let mut rendered_explicit_fd = false;
        let adjacent_numeric_fd = redirect_has_adjacent_numeric_fd(redirect, source);
        if let Some(name) = &redirect.fd_var {
            write!(
                formatter,
                [token("{"), text(name.as_str().to_string()), token("}")]
            )?;
            rendered_explicit_fd = true;
        } else if let Some(fd) = redirect
            .fd
            .filter(|fd| should_render_explicit_fd(*fd, redirect, source))
        {
            write!(formatter, [text(fd.to_string())])?;
            rendered_explicit_fd = true;
        }

        write!(
            formatter,
            [token(match redirect.kind {
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
            })]
        )?;

        let target = match (redirect.word_target(), redirect.heredoc()) {
            (Some(word), None) => render_word_syntax(word, source, &options),
            (None, Some(heredoc)) => render_word_syntax(&heredoc.delimiter.raw, source, &options),
            (None, None) => String::new(),
            (Some(_), Some(_)) => unreachable!("redirect target cannot be both word and heredoc"),
        };
        if needs_space_before_target(
            redirect.kind,
            &target,
            options.space_redirects(),
            rendered_explicit_fd || adjacent_numeric_fd,
        ) {
            write!(formatter, [space()])?;
        }
        write!(formatter, [text(target)])
    }
}

fn should_render_explicit_fd(fd: i32, redirect: &Redirect, source: &str) -> bool {
    raw_redirect_source_slice(redirect, source).is_some_and(|raw| {
        raw.trim_start()
            .strip_prefix(&fd.to_string())
            .is_some_and(|rest| rest.starts_with(['<', '>']))
    })
}

fn redirect_has_adjacent_numeric_fd(redirect: &Redirect, source: &str) -> bool {
    let start = redirect.span.start.offset.min(source.len());
    let Some(prefix) = source.get(..start) else {
        return false;
    };
    let token = prefix
        .rsplit_once(|ch: char| ch.is_whitespace() || ch == ';' || ch == '&' || ch == '|')
        .map_or(prefix, |(_, token)| token);
    !token.is_empty() && token.chars().all(|ch| ch.is_ascii_digit())
}

fn needs_space_before_target(
    kind: RedirectKind,
    target: &str,
    space_redirects: bool,
    explicit_fd: bool,
) -> bool {
    if target.is_empty() {
        return false;
    }

    if explicit_fd {
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

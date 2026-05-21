use super::*;

impl<'source, 'facts, S> ShellRenderer<'source, 'facts, S>
where
    S: StreamSink,
{
    pub(super) fn format_redirect_list(&mut self, redirects: &[Redirect]) {
        let source = self.source();
        let mut index = 0;
        let mut wrote_redirect = false;
        while index < redirects.len() {
            if wrote_redirect {
                self.write_space();
            }
            if redirects.get(index + 1).is_some_and(|next| {
                append_both_redirect_pair_matches_source(&redirects[index], next, source)
            }) {
                self.format_append_both_redirect(&redirects[index]);
                index += 2;
                wrote_redirect = true;
                continue;
            }
            let redirect = &redirects[index];
            self.format_redirect(redirect);
            index += 1;
            wrote_redirect = true;
        }
    }

    pub(super) fn format_redirect(&mut self, redirect: &Redirect) {
        let source = self.source();
        let options = self.options().clone();
        if !options.simplify()
            && !options.minify()
            && redirect.fd_var.is_none()
            && let Some(raw) = raw_redirect_source_slice(redirect, source)
            && should_preserve_raw_redirect(raw)
        {
            self.write_text(raw);
            return;
        }

        if let Some(name) = &redirect.fd_var {
            self.write_text("{");
            self.write_text(name.as_str());
            self.write_text("}");
        } else if let Some(fd) = redirect.fd
            && (should_render_explicit_fd(fd, redirect.kind)
                || redirect_source_has_explicit_fd(redirect, source, fd))
        {
            self.write_display(fd);
        }

        self.write_text(match redirect.kind {
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

        self.write_redirect_target(redirect, &options, true);
    }

    pub(super) fn format_append_both_redirect(&mut self, redirect: &Redirect) {
        let options = self.options().clone();
        self.write_text("&>>");
        self.write_redirect_target(redirect, &options, false);
    }

    pub(super) fn write_redirect_target(
        &mut self,
        redirect: &Redirect,
        options: &ResolvedShellFormatOptions,
        preserve_here_string_indent: bool,
    ) {
        let mut target = self.take_scratch_buffer();
        match (redirect.word_target(), redirect.heredoc()) {
            (Some(word), None) => self.render_word_to_buffer(word, &mut target),
            (None, Some(heredoc)) => {
                self.render_word_to_buffer(&heredoc.delimiter.raw, &mut target);
            }
            (None, None) => {}
            (Some(_), Some(_)) => {
                unreachable!("redirect target cannot be both word and heredoc")
            }
        }
        let normalized_target = normalized_redirect_target(redirect.kind, &target);
        if redirect_target_starts_on_continuation_line(redirect, self.facts()) {
            self.line_continuation();
            self.write_indent_units(1);
        } else if needs_space_before_target(
            redirect.kind,
            normalized_target,
            options.space_redirects(),
        ) {
            self.write_space();
        }
        if preserve_here_string_indent
            && matches!(redirect.kind, RedirectKind::HereString)
            && normalized_target.contains('\n')
            && !here_string_target_is_multiline_literal(normalized_target)
        {
            self.write_text_preserving_current_line_indent(normalized_target);
        } else {
            self.write_rendered_shell_text(normalized_target);
        }
        self.restore_scratch_buffer(target);
    }

    pub(super) fn queue_heredocs(&mut self, redirects: &[Redirect]) {
        let source = self.source();
        for redirect in redirects {
            let Some(heredoc) = redirect.heredoc() else {
                continue;
            };
            let body = if !self.options.simplify()
                && !self.options.minify()
                && !heredoc_body_contains_command_substitution(&heredoc.body)
                && heredoc.body.span.end.offset <= source.len()
                && heredoc.body.span.start.offset <= heredoc.body.span.end.offset
            {
                heredoc.body.span.slice(source).to_owned()
            } else {
                let mut rendered = String::new();
                render_heredoc_body_to_buf(
                    &heredoc.body,
                    source,
                    &self.options,
                    self.facts,
                    self.indent_level().saturating_add(1),
                    &mut rendered,
                );
                rendered
            };
            // The opening redirection keeps delimiter quoting, but the closing
            // marker line uses the cooked delimiter text after quote removal.
            let delimiter = self
                .facts()
                .heredoc_closing_marker_bounds(heredoc)
                .and_then(|(start, line_end)| source.get(start..line_end))
                .map(str::to_owned)
                .unwrap_or_else(|| heredoc.delimiter.cooked.to_string());
            self.writer.queue_heredoc(PendingHeredoc {
                body,
                delimiter,
                strip_tabs: matches!(redirect.kind, RedirectKind::HereDocStrip),
            });
        }
    }
}

fn raw_redirect_source_slice<'a>(redirect: &Redirect, source: &'a str) -> Option<&'a str> {
    let span = redirect.span;
    (span.start.offset < span.end.offset && span.end.offset <= source.len())
        .then(|| span.slice(source))
}

fn should_preserve_raw_redirect(raw: &str) -> bool {
    raw.contains(">&$")
        || raw.contains("<&$")
        || raw.contains(">&-")
        || raw.contains("<&-")
        || raw.contains(">&/")
        || raw.contains("<&/")
}

pub(super) fn append_both_redirect_pair_matches_source(
    redirect: &Redirect,
    next: &Redirect,
    source: &str,
) -> bool {
    if !matches!(redirect.kind, RedirectKind::Append)
        || redirect.fd.is_some()
        || redirect.fd_var.is_some()
    {
        return false;
    }
    if !matches!(next.kind, RedirectKind::DupOutput)
        || next.fd != Some(2)
        || next
            .word_target()
            .and_then(|word| word.try_static_text(source))
            .is_none_or(|target| target != "1")
    {
        return false;
    }

    let Some(raw) = raw_redirect_source_slice(redirect, source) else {
        return false;
    };
    if raw.starts_with("&>>") {
        return true;
    }
    if raw.starts_with(">>") {
        let Some(operator_start) = redirect.span.start.offset.checked_sub(1) else {
            return false;
        };
        return source
            .as_bytes()
            .get(operator_start)
            .is_some_and(|byte| *byte == b'&');
    }
    false
}

fn redirect_target_starts_on_continuation_line(
    redirect: &Redirect,
    facts: &FormatterFacts<'_>,
) -> bool {
    let target_start = redirect
        .word_target()
        .map(|word| word.span.start.offset)
        .or_else(|| {
            redirect
                .heredoc()
                .map(|heredoc| heredoc.delimiter.span.start.offset)
        });
    let Some(target_start) = target_start else {
        return false;
    };
    facts.has_continuation_line_start_between(redirect.span.start.offset, target_start)
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

fn redirect_source_has_explicit_fd(redirect: &Redirect, source: &str, fd: i32) -> bool {
    let Some(raw) = raw_redirect_source_slice(redirect, source) else {
        return false;
    };
    let rendered_fd = fd.to_string();
    raw.trim_start().starts_with(&rendered_fd)
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

fn normalized_redirect_target(kind: RedirectKind, target: &str) -> &str {
    if matches!(
        kind,
        RedirectKind::Output
            | RedirectKind::Clobber
            | RedirectKind::Append
            | RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::HereDoc
            | RedirectKind::HereDocStrip
            | RedirectKind::HereString
            | RedirectKind::OutputBoth
    ) {
        target.trim_start_matches([' ', '\t', '\r'])
    } else {
        target
    }
}

fn here_string_target_is_multiline_literal(target: &str) -> bool {
    let target = target.strip_prefix('$').unwrap_or(target);
    target.starts_with("\"\n")
        || target.starts_with("\"\\\n")
        || target.starts_with("'\n")
        || target.starts_with("$'\n")
        || target.starts_with("\"\r\n")
        || target.starts_with("\"\\\r\n")
        || target.starts_with("'\r\n")
        || target.starts_with("$'\r\n")
}

pub(super) fn redirect_list_needs_leading_space(
    command_span: Span,
    redirects: &[Redirect],
    source: &str,
) -> bool {
    redirects.first().is_none_or(|redirect| {
        !redirect_is_attached_process_substitution(command_span, redirect, source)
    })
}

pub(super) fn redirect_list_starts_on_continuation_line(
    command_span: Span,
    redirects: &[Redirect],
    facts: &FormatterFacts<'_>,
) -> bool {
    let Some(redirect) = redirects.first() else {
        return false;
    };
    if command_span == Span::new() || redirect.span.start.offset <= command_span.end.offset {
        return false;
    }
    facts.has_continuation_line_start_between(command_span.end.offset, redirect.span.start.offset)
}

pub(super) fn redirect_is_attached_process_substitution(
    _command_span: Span,
    redirect: &Redirect,
    source: &str,
) -> bool {
    let start = redirect.span.start.offset;
    let bytes = source.as_bytes();
    let attached_after_equals = start > 0 && bytes.get(start - 1).is_some_and(|byte| *byte == b'=')
        || start > 1
            && bytes
                .get(start - 1)
                .is_some_and(|byte| matches!(*byte, b'<' | b'>'))
            && bytes.get(start - 2).is_some_and(|byte| *byte == b'=');
    attached_after_equals
        && raw_redirect_source_slice(redirect, source).is_some_and(|raw| {
            raw.starts_with("<(") || raw.starts_with(">(") || raw.starts_with('(')
        })
}

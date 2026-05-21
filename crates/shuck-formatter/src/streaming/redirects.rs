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
            (Some(word), None) => self.render_word_with_facts_to_buffer(word, &mut target),
            (None, Some(heredoc)) => {
                self.render_word_with_facts_to_buffer(&heredoc.delimiter.raw, &mut target);
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

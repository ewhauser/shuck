#[derive(Debug, Clone)]
pub struct RedirectFact<'a> {
    redirect: &'a Redirect,
    brace_fd_redirection_span: Option<Span>,
    operator_span: Span,
    target_span: Option<Span>,
    arithmetic_update_operator_spans: Box<[Span]>,
    analysis: Option<RedirectTargetAnalysis>,
}

impl<'a> RedirectFact<'a> {
    pub fn redirect(&self) -> &'a Redirect {
        self.redirect
    }

    pub fn brace_fd_redirection_span(&self) -> Option<Span> {
        self.brace_fd_redirection_span
    }

    pub fn operator_span(&self) -> Span {
        self.operator_span
    }

    pub fn target_span(&self) -> Option<Span> {
        self.target_span
    }

    pub fn arithmetic_update_operator_spans(&self) -> &[Span] {
        &self.arithmetic_update_operator_spans
    }

    pub fn analysis(&self) -> Option<RedirectTargetAnalysis> {
        self.analysis
    }
}

fn build_redirect_facts<'a>(
    redirects: &'a [Redirect],
    source: &str,
    zsh_options: Option<&ZshOptionState>,
) -> Box<[RedirectFact<'a>]> {
    redirects
        .iter()
        .map(|redirect| RedirectFact {
            redirect,
            brace_fd_redirection_span: brace_fd_redirection_span(redirect, source),
            operator_span: redirect_operator_span(redirect),
            target_span: redirect.word_target().map(|word| word.span),
            arithmetic_update_operator_spans: redirect
                .word_target()
                .map_or_else(Vec::new, |word| {
                    let mut spans = Vec::new();
                    collect_arithmetic_update_operator_spans_from_parts(
                        &word.parts,
                        source,
                        &mut spans,
                    );
                    spans
                })
                .into_boxed_slice(),
            analysis: analyze_redirect_target(redirect, source, zsh_options),
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

fn brace_fd_redirection_span(redirect: &Redirect, source: &str) -> Option<Span> {
    let brace_span = redirect_fd_var_brace_span(redirect, source)?;
    let gap = source.get(brace_span.end.offset..redirect.span.start.offset)?;
    brace_fd_gap_allows_attachment(gap)
        .then(|| Span::from_positions(brace_span.start, redirect.span.end))
}

fn brace_fd_gap_allows_attachment(gap: &str) -> bool {
    if gap.is_empty() {
        return true;
    }

    let mut remaining = gap;
    while !remaining.is_empty() {
        if let Some(stripped) = remaining.strip_prefix("\\\r\n") {
            remaining = stripped;
            continue;
        }
        if let Some(stripped) = remaining.strip_prefix("\\\n") {
            remaining = stripped;
            continue;
        }
        return false;
    }

    true
}

fn redirect_operator_span(redirect: &Redirect) -> Span {
    let operator_start = redirect
        .fd_var_span
        .map(|span| span.end)
        .or_else(|| {
            redirect
                .fd
                .filter(|fd| *fd >= 0)
                .map(|fd| redirect.span.start.advanced_by(&fd.to_string()))
        })
        .unwrap_or(redirect.span.start);
    let operator_end = operator_start.advanced_by(redirect_operator_text(redirect.kind));

    Span::from_positions(operator_start, operator_end)
}

fn redirect_operator_text(kind: RedirectKind) -> &'static str {
    match kind {
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
    }
}

